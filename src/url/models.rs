use std::fmt::{self, Display, Formatter};

use crate::errors::NanoGetError;

/// Parsed URL data used by request builders and transports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// URL scheme (`http` or `https`).
    pub scheme: String,
    /// Hostname or literal IP address (without IPv6 brackets).
    pub host: String,
    /// Effective network port.
    pub port: u16,
    /// Normalized path component (always starts with `/`).
    pub path: String,
    /// Optional query string without the leading `?`.
    pub query: Option<String>,
    explicit_port: bool,
}

impl Url {
    /// Parses a URL string into a [`Url`].
    ///
    /// If no scheme is provided, `http` is assumed.
    pub fn parse(input: &str) -> Result<Self, NanoGetError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(NanoGetError::invalid_url("URL cannot be empty"));
        }

        let (scheme, remainder) = match trimmed.find("://") {
            Some(index) => {
                let scheme = &trimmed[..index];
                validate_scheme(scheme)?;
                (scheme.to_ascii_lowercase(), &trimmed[index + 3..])
            }
            None => ("http".to_string(), trimmed),
        };

        let without_fragment = strip_fragment(remainder);
        let (authority, target) = split_authority_and_target(without_fragment)?;
        let (host, port, explicit_port) = parse_authority(&scheme, authority)?;
        let (path, query) = parse_target(target)?;

        Ok(Self {
            scheme,
            host,
            port,
            path,
            query,
            explicit_port,
        })
    }

    /// Resolves a redirect location against this URL.
    ///
    /// Supports:
    /// - absolute URLs
    /// - scheme-relative URLs
    /// - absolute paths
    /// - relative paths
    /// - query-only redirects
    pub fn resolve(&self, location: &str) -> Result<Self, NanoGetError> {
        let trimmed = strip_fragment(location.trim());
        if trimmed.is_empty() {
            return Err(NanoGetError::invalid_url(
                "redirect location cannot be empty",
            ));
        }

        if trimmed.contains("://") {
            return Self::parse(trimmed);
        }

        if trimmed.starts_with("//") {
            return Self::parse(&format!("{}:{}", self.scheme, trimmed));
        }

        if let Some(query) = trimmed.strip_prefix('?') {
            return Ok(Self {
                scheme: self.scheme.clone(),
                host: self.host.clone(),
                port: self.port,
                path: self.path.clone(),
                query: Some(query.to_string()),
                explicit_port: self.explicit_port,
            });
        }

        let (path, query) = if trimmed.starts_with('/') {
            parse_target(trimmed)?
        } else {
            let (relative_path, query) = split_path_and_query(trimmed);
            let combined = format!("{}{}", base_directory(&self.path), relative_path);
            (
                normalize_path(&combined),
                query.map(|value| value.to_string()),
            )
        };

        Ok(Self {
            scheme: self.scheme.clone(),
            host: self.host.clone(),
            port: self.port,
            path,
            query,
            explicit_port: self.explicit_port,
        })
    }

    /// Returns the request-target in origin-form, for example `/path?query`.
    pub fn origin_form(&self) -> String {
        match &self.query {
            Some(query) => format!("{}?{}", self.path, query),
            None => self.path.clone(),
        }
    }

    /// Returns the request-target in absolute-form, for proxy HTTP requests.
    pub fn absolute_form(&self) -> String {
        format!(
            "{}://{}{}",
            self.scheme,
            self.host_header_value(),
            self.origin_form()
        )
    }

    /// Returns the request-target in authority-form, for example `example.com:443`.
    pub fn authority_form(&self) -> String {
        self.connect_host_with_port()
    }

    /// Returns the value used for the `Host` header.
    pub fn host_header_value(&self) -> String {
        let host = format_host_for_authority(&self.host);
        if self.explicit_port || !self.is_default_port() {
            format!("{host}:{}", self.port)
        } else {
            host
        }
    }

    /// Returns `host:port`, with IPv6 hosts bracketed as required.
    pub fn connect_host_with_port(&self) -> String {
        format!("{}:{}", format_host_for_authority(&self.host), self.port)
    }

    /// Returns the full normalized URL string.
    pub fn full_url(&self) -> String {
        self.absolute_form()
    }

    /// Returns `true` when scheme is `https`.
    pub fn is_https(&self) -> bool {
        self.scheme == "https"
    }

    /// Returns `true` when scheme is `http`.
    pub fn is_http(&self) -> bool {
        self.scheme == "http"
    }

    /// Returns `true` if the port matches the scheme default.
    pub fn is_default_port(&self) -> bool {
        default_port_for_scheme(&self.scheme) == Some(self.port)
    }

    /// Returns `true` when scheme, host, and port are identical.
    pub fn same_authority(&self, other: &Self) -> bool {
        self.scheme == other.scheme && self.host == other.host && self.port == other.port
    }

    /// Returns the cache key used by this crate's in-memory cache.
    pub fn cache_key(&self) -> String {
        self.full_url()
    }
}

impl Display for Url {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.full_url())
    }
}

/// Conversion trait for values that can be parsed into a [`Url`].
pub trait ToUrl {
    /// Parses or converts `self` into a [`Url`].
    fn to_url(&self) -> Result<Url, NanoGetError>;
}

impl ToUrl for String {
    fn to_url(&self) -> Result<Url, NanoGetError> {
        Url::parse(self)
    }
}

impl ToUrl for &str {
    fn to_url(&self) -> Result<Url, NanoGetError> {
        Url::parse(self)
    }
}

impl ToUrl for &String {
    fn to_url(&self) -> Result<Url, NanoGetError> {
        Url::parse(self)
    }
}

impl ToUrl for Url {
    fn to_url(&self) -> Result<Url, NanoGetError> {
        Ok(self.clone())
    }
}

impl ToUrl for &Url {
    fn to_url(&self) -> Result<Url, NanoGetError> {
        Ok((*self).clone())
    }
}

fn validate_scheme(scheme: &str) -> Result<(), NanoGetError> {
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" => Ok(()),
        other => Err(NanoGetError::UnsupportedScheme(other.to_string())),
    }
}

fn strip_fragment(input: &str) -> &str {
    input.split('#').next().unwrap_or(input)
}

fn split_authority_and_target(input: &str) -> Result<(&str, &str), NanoGetError> {
    if input.is_empty() {
        return Err(NanoGetError::invalid_url("missing host"));
    }

    match input.find(['/', '?']) {
        Some(index) => Ok((&input[..index], &input[index..])),
        None => Ok((input, "")),
    }
}

fn parse_authority(scheme: &str, authority: &str) -> Result<(String, u16, bool), NanoGetError> {
    if authority.is_empty() {
        return Err(NanoGetError::invalid_url("missing host"));
    }
    if authority.contains('@') {
        return Err(NanoGetError::invalid_url(
            "user info in URLs is not supported",
        ));
    }

    let default_port = default_port_for_scheme(scheme)
        .ok_or_else(|| NanoGetError::UnsupportedScheme(scheme.to_string()))?;

    if authority.starts_with('[') {
        let closing = authority
            .find(']')
            .ok_or_else(|| NanoGetError::invalid_url("unterminated IPv6 host"))?;
        let host = authority[1..closing].to_string();
        let remainder = &authority[closing + 1..];
        if remainder.is_empty() {
            return Ok((host, default_port, false));
        }
        if let Some(port) = remainder.strip_prefix(':') {
            return Ok((host, parse_port(port)?, true));
        }
        return Err(NanoGetError::invalid_url("invalid IPv6 authority"));
    }

    let (host, port, explicit_port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host.to_string(), parse_port(port)?, true),
        Some(_) => {
            return Err(NanoGetError::invalid_url(
                "IPv6 hosts must use bracket notation",
            ));
        }
        None => (authority.to_string(), default_port, false),
    };

    if host.is_empty() {
        return Err(NanoGetError::invalid_url("missing host"));
    }

    Ok((host, port, explicit_port))
}

fn parse_port(input: &str) -> Result<u16, NanoGetError> {
    input
        .parse::<u16>()
        .map_err(|_| NanoGetError::invalid_url(format!("invalid port: {input}")))
}

fn parse_target(target: &str) -> Result<(String, Option<String>), NanoGetError> {
    if target.is_empty() {
        return Ok(("/".to_string(), None));
    }

    if let Some(query) = target.strip_prefix('?') {
        return Ok(("/".to_string(), Some(query.to_string())));
    }

    if !target.starts_with('/') {
        return Err(NanoGetError::invalid_url("path must start with `/`"));
    }

    let (path, query) = split_path_and_query(target);
    Ok((normalize_path(path), query.map(|value| value.to_string())))
}

fn split_path_and_query(input: &str) -> (&str, Option<&str>) {
    match input.find('?') {
        Some(index) => (&input[..index], Some(&input[index + 1..])),
        None => (input, None),
    }
}

fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn format_host_for_authority(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn base_directory(path: &str) -> String {
    if path.ends_with('/') {
        return path.to_string();
    }

    match path.rfind('/') {
        Some(index) if index > 0 => path[..index + 1].to_string(),
        Some(_) => "/".to_string(),
        None => "/".to_string(),
    }
}

fn normalize_path(path: &str) -> String {
    let preserve_trailing_slash = path.ends_with('/') && path.len() > 1;
    let mut parts = Vec::new();

    for segment in path.split('/') {
        match segment {
            "" | "." => continue,
            ".." => {
                parts.pop();
            }
            value => parts.push(value),
        }
    }

    let mut normalized = String::from("/");
    normalized.push_str(&parts.join("/"));

    if preserve_trailing_slash && normalized != "/" {
        normalized.push('/');
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::{
        default_port_for_scheme, normalize_path, parse_target, split_authority_and_target, ToUrl,
        Url,
    };
    use crate::errors::NanoGetError;

    #[test]
    fn parses_default_http_url() {
        let url = Url::parse("example.com/a/b?c=1").unwrap();
        assert_eq!(url.scheme, "http");
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 80);
        assert_eq!(url.path, "/a/b");
        assert_eq!(url.query.as_deref(), Some("c=1"));
        assert_eq!(url.origin_form(), "/a/b?c=1");
    }

    #[test]
    fn parses_https_url_with_explicit_port() {
        let url = Url::parse("https://example.com:8443/path").unwrap();
        assert_eq!(url.scheme, "https");
        assert_eq!(url.port, 8443);
        assert_eq!(url.host_header_value(), "example.com:8443");
        assert_eq!(url.connect_host_with_port(), "example.com:8443");
    }

    #[test]
    fn parses_bracketed_ipv6_hosts() {
        let url = Url::parse("http://[::1]:8080/").unwrap();
        assert_eq!(url.host, "::1");
        assert_eq!(url.connect_host_with_port(), "[::1]:8080");
        assert_eq!(url.host_header_value(), "[::1]:8080");
    }

    #[test]
    fn strips_fragments() {
        let url = Url::parse("http://example.com/path?a=1#fragment").unwrap();
        assert_eq!(url.origin_form(), "/path?a=1");
        assert_eq!(url.full_url(), "http://example.com/path?a=1");
    }

    #[test]
    fn resolves_relative_redirects() {
        let base = Url::parse("http://example.com/a/b/index.html?x=1").unwrap();
        let resolved = base.resolve("../next?y=2").unwrap();
        assert_eq!(resolved.full_url(), "http://example.com/a/next?y=2");
    }

    #[test]
    fn resolves_absolute_path_redirects() {
        let base = Url::parse("https://example.com/one/two").unwrap();
        let resolved = base.resolve("/rooted").unwrap();
        assert_eq!(resolved.full_url(), "https://example.com/rooted");
    }

    #[test]
    fn resolves_query_only_redirects() {
        let base = Url::parse("http://example.com/path?a=1").unwrap();
        let resolved = base.resolve("?b=2").unwrap();
        assert_eq!(resolved.full_url(), "http://example.com/path?b=2");
    }

    #[test]
    fn rejects_unsupported_schemes() {
        let error = Url::parse("ftp://example.com").unwrap_err();
        assert!(matches!(error, NanoGetError::UnsupportedScheme(ref value) if value == "ftp"));
    }

    #[test]
    fn rejects_unbracketed_ipv6_hosts() {
        let error = Url::parse("http://::1/path").unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidUrl(_)));
    }

    #[test]
    fn rejects_empty_and_userinfo_urls() {
        assert!(matches!(Url::parse(""), Err(NanoGetError::InvalidUrl(_))));
        assert!(matches!(
            Url::parse("http://user@example.com"),
            Err(NanoGetError::InvalidUrl(_))
        ));
    }

    #[test]
    fn rejects_invalid_ports_and_ipv6_authorities() {
        assert!(matches!(
            Url::parse("http://example.com:abc"),
            Err(NanoGetError::InvalidUrl(_))
        ));
        assert!(matches!(
            Url::parse("http://[::1]bad"),
            Err(NanoGetError::InvalidUrl(_))
        ));
    }

    #[test]
    fn resolves_scheme_relative_redirects() {
        let base = Url::parse("https://example.com/one").unwrap();
        let resolved = base.resolve("//cdn.example.com/path").unwrap();
        assert_eq!(resolved.full_url(), "https://cdn.example.com/path");
    }

    #[test]
    fn builds_absolute_and_authority_forms() {
        let url = Url::parse("http://example.com:8080/path?x=1").unwrap();
        assert_eq!(url.absolute_form(), "http://example.com:8080/path?x=1");
        assert_eq!(url.authority_form(), "example.com:8080");
    }

    #[test]
    fn detects_matching_authorities() {
        let one = Url::parse("https://example.com/path").unwrap();
        let two = Url::parse("https://example.com/other").unwrap();
        let three = Url::parse("http://example.com/other").unwrap();
        assert!(one.same_authority(&two));
        assert!(!one.same_authority(&three));
    }

    #[test]
    fn covers_display_and_tourl_variants() {
        let url = Url::parse("http://example.com/path").unwrap();
        assert_eq!(url.to_string(), "http://example.com/path");
        assert_eq!(url.to_url().unwrap(), url);
        assert_eq!(<&Url as ToUrl>::to_url(&&url).unwrap(), url);
    }

    #[test]
    fn covers_missing_hosts_and_invalid_targets() {
        assert!(matches!(
            Url::parse("http://"),
            Err(NanoGetError::InvalidUrl(_))
        ));
        assert!(matches!(
            Url::parse("http:///path"),
            Err(NanoGetError::InvalidUrl(_))
        ));
        assert!(matches!(
            split_authority_and_target(""),
            Err(NanoGetError::InvalidUrl(_))
        ));
        assert!(matches!(
            Url::parse("http://:80/path"),
            Err(NanoGetError::InvalidUrl(_))
        ));
        assert!(matches!(
            parse_target("not-a-path"),
            Err(NanoGetError::InvalidUrl(_))
        ));
        assert_eq!(
            parse_target("?x=1").unwrap(),
            ("/".to_string(), Some("x=1".to_string()))
        );
    }

    #[test]
    fn covers_helper_branches_for_paths_and_ports() {
        let ipv6 = Url::parse("http://[::1]/").unwrap();
        assert_eq!(ipv6.authority_form(), "[::1]:80");

        assert_eq!(default_port_for_scheme("http"), Some(80));
        assert_eq!(default_port_for_scheme("https"), Some(443));
        assert_eq!(default_port_for_scheme("ws"), None);

        assert_eq!(super::base_directory("/a/b/"), "/a/b/");
        assert_eq!(super::base_directory("/a"), "/");
        assert_eq!(super::base_directory("a"), "/");
        assert_eq!(normalize_path("/a/b/"), "/a/b/");

        let base = Url::parse("http://example.com/path").unwrap();
        let error = base.resolve("   ").unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidUrl(_)));
    }
}
