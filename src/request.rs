use std::time::SystemTime;

use crate::auth::basic_authorization_value;
use crate::client::Client;
use crate::date::format_http_date;
use crate::errors::NanoGetError;
use crate::url::{ToUrl, Url};

#[cfg(test)]
const DEFAULT_USER_AGENT: &str = "nano-get/0.3.0";
#[cfg(test)]
const DEFAULT_ACCEPT: &str = "*/*";

/// A single HTTP header field.
///
/// Header name matching in this crate is ASCII case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    name: String,
    value: String,
}

impl Header {
    /// Creates a new header, validating the header name and value.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Result<Self, NanoGetError> {
        let name = name.into();
        let value = value.into();
        validate_header_name(&name)?;
        validate_header_value(&value)?;
        Ok(Self { name, value })
    }

    pub(crate) fn unchecked(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Returns the header field-name as provided.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the header field-value as provided.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns `true` when `needle` matches this header name (case-insensitive).
    pub fn matches_name(&self, needle: &str) -> bool {
        self.name.eq_ignore_ascii_case(needle)
    }
}

/// Supported request methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// HTTP `GET`.
    Get,
    /// HTTP `HEAD`.
    Head,
}

impl Method {
    /// Returns the method token as sent on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
        }
    }
}

/// Redirect behavior for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectPolicy {
    /// Do not follow redirects automatically.
    None,
    /// Follow redirects up to `max_redirects`.
    Follow {
        /// Maximum number of redirect hops to follow.
        max_redirects: usize,
    },
}

impl RedirectPolicy {
    /// Convenience constructor for [`RedirectPolicy::None`].
    pub const fn none() -> Self {
        Self::None
    }

    /// Convenience constructor for [`RedirectPolicy::Follow`].
    pub const fn follow(max_redirects: usize) -> Self {
        Self::Follow { max_redirects }
    }

    /// Returns the configured redirect limit for [`RedirectPolicy::Follow`], otherwise `None`.
    pub fn max_redirects(self) -> Option<usize> {
        match self {
            Self::None => None,
            Self::Follow { max_redirects } => Some(max_redirects),
        }
    }
}

/// A typed HTTP request for `GET` or `HEAD`.
#[derive(Debug, Clone)]
pub struct Request {
    url: Url,
    method: Method,
    headers: Vec<Header>,
    redirect_policy: RedirectPolicy,
    redirect_policy_explicit: bool,
    preemptive_origin_auth_allowed: bool,
}

impl Request {
    /// Creates a new request with the given method and URL.
    pub fn new<U: ToUrl>(method: Method, url: U) -> Result<Self, NanoGetError> {
        Ok(Self {
            url: url.to_url()?,
            method,
            headers: Vec::new(),
            redirect_policy: RedirectPolicy::none(),
            redirect_policy_explicit: false,
            preemptive_origin_auth_allowed: true,
        })
    }

    /// Creates a new `GET` request.
    pub fn get<U: ToUrl>(url: U) -> Result<Self, NanoGetError> {
        Self::new(Method::Get, url)
    }

    /// Creates a new `HEAD` request.
    pub fn head<U: ToUrl>(url: U) -> Result<Self, NanoGetError> {
        Self::new(Method::Head, url)
    }

    /// Returns the request method.
    pub fn method(&self) -> Method {
        self.method
    }

    /// Returns the parsed request URL.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Returns all request headers in insertion order.
    pub fn headers(&self) -> &[Header] {
        &self.headers
    }

    /// Returns the first header value matching `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.matches_name(name))
            .map(Header::value)
    }

    /// Iterates over all headers matching `name` (case-insensitive), preserving insertion order.
    pub fn headers_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Header> + 'a {
        self.headers
            .iter()
            .filter(move |header| header.matches_name(name))
    }

    /// Returns this request's redirect policy.
    pub fn redirect_policy(&self) -> RedirectPolicy {
        self.redirect_policy
    }

    /// Sets redirect policy on this request using builder style.
    pub fn with_redirect_policy(mut self, policy: RedirectPolicy) -> Self {
        self.redirect_policy = policy;
        self.redirect_policy_explicit = true;
        self
    }

    /// Sets redirect policy on this request in-place.
    pub fn set_redirect_policy(&mut self, policy: RedirectPolicy) -> &mut Self {
        self.redirect_policy = policy;
        self.redirect_policy_explicit = true;
        self
    }

    /// Appends a header without replacing existing headers of the same name.
    ///
    /// Protocol-managed and hop-by-hop header names are rejected.
    pub fn add_header(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<&mut Self, NanoGetError> {
        let name = name.into();
        validate_request_header_name(&name)?;
        self.headers.push(Header::new(name, value)?);
        Ok(self)
    }

    /// Sets a header value by removing existing headers with the same name first.
    ///
    /// Protocol-managed and hop-by-hop header names are rejected.
    pub fn set_header(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<&mut Self, NanoGetError> {
        let name = name.into();
        validate_request_header_name(&name)?;
        self.remove_headers_named(&name);
        self.headers.push(Header::new(name, value)?);
        Ok(self)
    }

    /// Removes all headers with the provided name.
    pub fn remove_headers_named(&mut self, name: &str) -> &mut Self {
        self.headers.retain(|header| !header.matches_name(name));
        self
    }

    /// Sets `If-None-Match`.
    pub fn if_none_match(&mut self, etag: impl Into<String>) -> Result<&mut Self, NanoGetError> {
        self.set_header("If-None-Match", etag)
    }

    /// Sets `If-Match`.
    pub fn if_match(&mut self, etag: impl Into<String>) -> Result<&mut Self, NanoGetError> {
        self.set_header("If-Match", etag)
    }

    /// Sets `If-Modified-Since` using IMF-fixdate formatting.
    pub fn if_modified_since(&mut self, timestamp: SystemTime) -> Result<&mut Self, NanoGetError> {
        self.set_header("If-Modified-Since", format_http_date(timestamp)?)
    }

    /// Sets `If-Unmodified-Since` using IMF-fixdate formatting.
    pub fn if_unmodified_since(
        &mut self,
        timestamp: SystemTime,
    ) -> Result<&mut Self, NanoGetError> {
        self.set_header("If-Unmodified-Since", format_http_date(timestamp)?)
    }

    /// Sets `If-Range`.
    pub fn if_range(&mut self, value: impl Into<String>) -> Result<&mut Self, NanoGetError> {
        self.set_header("If-Range", value)
    }

    /// Sets an explicit `Authorization` header for this request.
    ///
    /// Manual request-level credentials take precedence over automatic client-level auth helpers.
    pub fn authorization(&mut self, value: impl Into<String>) -> Result<&mut Self, NanoGetError> {
        self.set_header("Authorization", value)
    }

    /// Sets an explicit `Proxy-Authorization` header for this request.
    ///
    /// Manual request-level credentials take precedence over automatic client-level proxy auth
    /// helpers.
    pub fn proxy_authorization(
        &mut self,
        value: impl Into<String>,
    ) -> Result<&mut Self, NanoGetError> {
        self.set_header("Proxy-Authorization", value)
    }

    /// Encodes `username:password` as HTTP Basic credentials and stores them in
    /// `Authorization`.
    pub fn basic_auth(
        &mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<&mut Self, NanoGetError> {
        self.authorization(basic_authorization_value(username.into(), password.into()))
    }

    /// Encodes `username:password` as HTTP Basic credentials and stores them in
    /// `Proxy-Authorization`.
    pub fn proxy_basic_auth(
        &mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<&mut Self, NanoGetError> {
        self.proxy_authorization(basic_authorization_value(username.into(), password.into()))
    }

    /// Sets a `Range` header for byte-range requests.
    ///
    /// Valid forms:
    /// - `Some(start), Some(end)` => `bytes=start-end`
    /// - `Some(start), None` => `bytes=start-`
    /// - `None, Some(end)` => `bytes=-end`
    pub fn range_bytes(
        &mut self,
        start: Option<u64>,
        end: Option<u64>,
    ) -> Result<&mut Self, NanoGetError> {
        let range = match (start, end) {
            (Some(start), Some(end)) if start <= end => format!("bytes={start}-{end}"),
            (Some(start), None) => format!("bytes={start}-"),
            (None, Some(end)) => format!("bytes=-{end}"),
            _ => {
                return Err(NanoGetError::InvalidHeaderValue(
                    "invalid byte range".to_string(),
                ))
            }
        };

        self.set_header("Range", range)
    }

    /// Executes this request using [`Client::default`].
    ///
    /// Use [`crate::Client`] directly when you need explicit client configuration.
    pub fn execute(&self) -> Result<crate::response::Response, NanoGetError> {
        Client::default().execute(self.clone())
    }

    pub(crate) fn has_header(&self, name: &str) -> bool {
        self.headers.iter().any(|header| header.matches_name(name))
    }

    #[cfg(test)]
    pub(crate) fn default_headers(&self) -> [Header; 4] {
        self.default_headers_for(true)
    }

    #[cfg(test)]
    pub(crate) fn default_headers_for(&self, connection_close: bool) -> [Header; 4] {
        [
            Header::unchecked("Host", self.url.host_header_value()),
            Header::unchecked("User-Agent", DEFAULT_USER_AGENT),
            Header::unchecked("Accept", DEFAULT_ACCEPT),
            Header::unchecked(
                "Connection",
                if connection_close {
                    "close"
                } else {
                    "keep-alive"
                },
            ),
        ]
    }

    pub(crate) fn clone_with_url(&self, url: Url) -> Self {
        let mut cloned = self.clone();
        cloned.url = url;
        cloned
    }

    pub(crate) fn effective_redirect_policy(&self, fallback: RedirectPolicy) -> RedirectPolicy {
        if self.redirect_policy_explicit {
            self.redirect_policy
        } else {
            fallback
        }
    }

    pub(crate) fn preemptive_origin_auth_allowed(&self) -> bool {
        self.preemptive_origin_auth_allowed
    }

    pub(crate) fn disable_preemptive_origin_auth(&mut self) {
        self.preemptive_origin_auth_allowed = false;
    }
}

fn validate_header_name(name: &str) -> Result<(), NanoGetError> {
    if name.is_empty() || !name.as_bytes().iter().all(|byte| is_tchar(*byte)) {
        return Err(NanoGetError::InvalidHeaderName(name.to_string()));
    }
    Ok(())
}

fn validate_header_value(value: &str) -> Result<(), NanoGetError> {
    if value
        .chars()
        .any(|ch| ch == '\r' || ch == '\n' || (ch.is_ascii_control() && ch != '\t'))
    {
        return Err(NanoGetError::InvalidHeaderValue(value.to_string()));
    }
    Ok(())
}

fn is_tchar(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn validate_request_header_name(name: &str) -> Result<(), NanoGetError> {
    if matches_protocol_managed_header(name) {
        return Err(NanoGetError::ProtocolManagedHeader(name.to_string()));
    }

    if matches_hop_by_hop_header(name) {
        return Err(NanoGetError::HopByHopHeader(name.to_string()));
    }

    Ok(())
}

fn matches_protocol_managed_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "connection" | "content-length" | "transfer-encoding" | "trailer" | "upgrade"
    )
}

fn matches_hop_by_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "keep-alive" | "proxy-connection" | "te"
    )
}

pub(crate) fn should_follow_redirect(status_code: u16) -> bool {
    matches!(status_code, 301 | 302 | 303 | 307 | 308)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{Method, RedirectPolicy, Request};
    use crate::errors::NanoGetError;

    #[test]
    fn request_defaults_to_no_redirects() {
        let request = Request::get("http://example.com").unwrap();
        assert_eq!(request.redirect_policy(), RedirectPolicy::None);
    }

    #[test]
    fn add_header_validates_name() {
        let error = Request::get("http://example.com")
            .unwrap()
            .add_header("bad:name", "value")
            .unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidHeaderName(_)));

        let error = Request::get("http://example.com")
            .unwrap()
            .add_header("bad(name)", "value")
            .unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidHeaderName(_)));
    }

    #[test]
    fn add_header_validates_value() {
        let error = Request::get("http://example.com")
            .unwrap()
            .add_header("x-test", "bad\r\nvalue")
            .unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidHeaderValue(_)));

        let error = Request::get("http://example.com")
            .unwrap()
            .add_header("x-test", "bad\u{0000}value")
            .unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidHeaderValue(_)));
    }

    #[test]
    fn builder_updates_redirect_policy() {
        let request = Request::head("http://example.com")
            .unwrap()
            .with_redirect_policy(RedirectPolicy::follow(5));
        assert_eq!(request.method(), Method::Head);
        assert_eq!(request.redirect_policy().max_redirects(), Some(5));
        assert_eq!(RedirectPolicy::none().max_redirects(), None);
    }

    #[test]
    fn set_redirect_policy_updates_in_place() {
        let mut request = Request::get("http://example.com").unwrap();
        request.set_redirect_policy(RedirectPolicy::follow(2));
        assert_eq!(request.redirect_policy().max_redirects(), Some(2));
    }

    #[test]
    fn default_headers_include_host() {
        let request = Request::get("http://example.com:8080/path").unwrap();
        let headers = request.default_headers();
        assert!(headers
            .iter()
            .any(|header| { header.matches_name("host") && header.value() == "example.com:8080" }));
    }

    #[test]
    fn set_header_replaces_existing_values() {
        let mut request = Request::get("http://example.com").unwrap();
        request.add_header("X-Test", "one").unwrap();
        request.set_header("x-test", "two").unwrap();
        let values: Vec<_> = request
            .headers_named("X-Test")
            .map(|header| header.value())
            .collect();
        assert_eq!(values, vec!["two"]);
    }

    #[test]
    fn range_header_helper_supports_suffixes() {
        let mut request = Request::get("http://example.com").unwrap();
        request.range_bytes(None, Some(128)).unwrap();
        assert_eq!(request.header("range"), Some("bytes=-128"));
    }

    #[test]
    fn authorization_helpers_set_headers() {
        let mut request = Request::get("http://example.com").unwrap();
        request.basic_auth("user", "pass").unwrap();
        request.proxy_basic_auth("proxy", "secret").unwrap();
        assert_eq!(request.header("authorization"), Some("Basic dXNlcjpwYXNz"));
        assert_eq!(
            request.header("proxy-authorization"),
            Some("Basic cHJveHk6c2VjcmV0")
        );
    }

    #[test]
    fn rejects_protocol_managed_headers() {
        for name in [
            "Host",
            "Connection",
            "Content-Length",
            "Transfer-Encoding",
            "Trailer",
            "Upgrade",
        ] {
            let error = Request::get("http://example.com")
                .unwrap()
                .add_header(name, "value")
                .unwrap_err();
            assert!(matches!(error, NanoGetError::ProtocolManagedHeader(_)));
        }
    }

    #[test]
    fn rejects_hop_by_hop_headers() {
        for name in ["Keep-Alive", "Proxy-Connection", "TE"] {
            let error = Request::get("http://example.com")
                .unwrap()
                .add_header(name, "value")
                .unwrap_err();
            assert!(matches!(error, NanoGetError::HopByHopHeader(_)));
        }
    }

    #[test]
    fn date_header_helpers_format_http_dates() {
        let mut request = Request::get("http://example.com").unwrap();
        request
            .if_modified_since(UNIX_EPOCH + Duration::from_secs(784_111_777))
            .unwrap();
        request
            .if_unmodified_since(UNIX_EPOCH + Duration::from_secs(784_111_777))
            .unwrap();
        request.if_match("\"etag\"").unwrap();
        assert_eq!(
            request.header("if-modified-since"),
            Some("Sun, 06 Nov 1994 08:49:37 GMT")
        );
        assert_eq!(
            request.header("if-unmodified-since"),
            Some("Sun, 06 Nov 1994 08:49:37 GMT")
        );
        assert_eq!(request.header("if-match"), Some("\"etag\""));
    }

    #[test]
    fn range_helper_supports_open_ended_ranges_and_rejects_invalid_values() {
        let mut request = Request::get("http://example.com").unwrap();
        request.range_bytes(Some(128), None).unwrap();
        assert_eq!(request.header("range"), Some("bytes=128-"));

        let error = request.range_bytes(Some(10), Some(2)).unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidHeaderValue(_)));
    }
}
