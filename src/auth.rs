use std::sync::Arc;

use crate::errors::NanoGetError;
use crate::request::Header;
use crate::response::Response;
use crate::url::Url;

/// Indicates which authentication space a challenge belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthTarget {
    /// Origin server authentication (`WWW-Authenticate` / `Authorization`).
    Origin,
    /// Proxy authentication (`Proxy-Authenticate` / `Proxy-Authorization`).
    Proxy,
}

/// A single auth-param pair parsed from a challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthParam {
    /// Parameter name.
    pub name: String,
    /// Parameter value with surrounding quotes removed when applicable.
    pub value: String,
}

/// Parsed authentication challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    /// Authentication scheme token, for example `Basic`.
    pub scheme: String,
    /// Optional token68 payload.
    pub token68: Option<String>,
    /// Optional list of auth-params.
    pub params: Vec<AuthParam>,
}

/// Authentication handler result for a challenge set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    /// Retry the request with these headers.
    UseHeaders(Vec<Header>),
    /// Do not handle this challenge.
    NoMatch,
    /// Stop authentication processing and return an error.
    Abort,
}

/// Callback interface for custom authentication schemes.
pub trait AuthHandler {
    /// Chooses how to respond to a challenge set.
    ///
    /// Return [`AuthDecision::UseHeaders`] to retry with credentials, [`AuthDecision::NoMatch`]
    /// to leave the response unchanged, or [`AuthDecision::Abort`] to stop with an error.
    fn respond(
        &self,
        target: AuthTarget,
        url: &Url,
        challenges: &[Challenge],
        request: &crate::request::Request,
        response: &Response,
    ) -> Result<AuthDecision, NanoGetError>;
}

pub(crate) type DynAuthHandler = Arc<dyn AuthHandler + Send + Sync>;

#[derive(Clone)]
pub(crate) struct BasicAuthHandler {
    header_value: String,
    target: AuthTarget,
}

impl BasicAuthHandler {
    pub(crate) fn new(
        username: impl Into<String>,
        password: impl Into<String>,
        target: AuthTarget,
    ) -> Self {
        Self {
            header_value: basic_authorization_value(username.into(), password.into()),
            target,
        }
    }

    pub(crate) fn header_value(&self) -> &str {
        &self.header_value
    }
}

impl AuthHandler for BasicAuthHandler {
    fn respond(
        &self,
        target: AuthTarget,
        _url: &Url,
        challenges: &[Challenge],
        _request: &crate::request::Request,
        _response: &Response,
    ) -> Result<AuthDecision, NanoGetError> {
        if target != self.target {
            return Ok(AuthDecision::NoMatch);
        }

        if challenges
            .iter()
            .any(|challenge| challenge.scheme.eq_ignore_ascii_case("basic"))
        {
            let header_name = match target {
                AuthTarget::Origin => "Authorization",
                AuthTarget::Proxy => "Proxy-Authorization",
            };
            return Ok(AuthDecision::UseHeaders(vec![Header::new(
                header_name,
                self.header_value.clone(),
            )?]));
        }

        Ok(AuthDecision::NoMatch)
    }
}

pub(crate) fn basic_authorization_value(
    username: impl Into<String>,
    password: impl Into<String>,
) -> String {
    let credentials = format!("{}:{}", username.into(), password.into());
    format!("Basic {}", base64_encode(credentials.as_bytes()))
}

pub(crate) fn parse_authenticate_headers(
    headers: &[Header],
    header_name: &str,
) -> Result<Vec<Challenge>, NanoGetError> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|header| header.matches_name(header_name))
        .map(Header::value)
        .collect();

    if values.is_empty() {
        return Ok(Vec::new());
    }

    let mut challenges = Vec::new();
    for value in values {
        challenges.extend(parse_challenge_list(value)?);
    }
    Ok(challenges)
}

fn parse_challenge_list(value: &str) -> Result<Vec<Challenge>, NanoGetError> {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    let mut challenges = Vec::new();

    while index < bytes.len() {
        skip_ows_and_commas(bytes, &mut index);
        if index >= bytes.len() {
            break;
        }

        let scheme = parse_token(bytes, &mut index)
            .ok_or_else(|| NanoGetError::MalformedChallenge(value.to_string()))?;
        skip_spaces(bytes, &mut index);

        let mut challenge = Challenge {
            scheme,
            token68: None,
            params: Vec::new(),
        };

        if index < bytes.len() && bytes[index] != b',' {
            if looks_like_auth_param(bytes, index) {
                challenge.params = parse_auth_params(bytes, &mut index)?;
            } else {
                challenge.token68 = Some(parse_token68(bytes, &mut index)?);
            }
        }

        challenges.push(challenge);

        skip_spaces(bytes, &mut index);
        if index < bytes.len() && bytes[index] == b',' {
            index += 1;
        }
    }

    Ok(challenges)
}

fn parse_auth_params(bytes: &[u8], index: &mut usize) -> Result<Vec<AuthParam>, NanoGetError> {
    let mut params = Vec::new();

    loop {
        skip_spaces(bytes, index);
        let name = parse_token(bytes, index).ok_or_else(|| {
            NanoGetError::MalformedChallenge(String::from_utf8_lossy(bytes).into_owned())
        })?;
        skip_spaces(bytes, index);

        if *index >= bytes.len() || bytes[*index] != b'=' {
            return Err(NanoGetError::MalformedChallenge(
                String::from_utf8_lossy(bytes).into_owned(),
            ));
        }
        *index += 1;
        skip_spaces(bytes, index);

        let value = if *index < bytes.len() && bytes[*index] == b'"' {
            parse_quoted_string(bytes, index)?
        } else {
            parse_token(bytes, index).ok_or_else(|| {
                NanoGetError::MalformedChallenge(String::from_utf8_lossy(bytes).into_owned())
            })?
        };
        params.push(AuthParam { name, value });

        skip_spaces(bytes, index);
        if *index >= bytes.len() || bytes[*index] != b',' {
            break;
        }

        let lookahead = *index + 1;
        let mut next_index = lookahead;
        skip_spaces(bytes, &mut next_index);
        if !looks_like_auth_param(bytes, next_index) {
            break;
        }
        *index += 1;
    }

    Ok(params)
}

fn looks_like_auth_param(bytes: &[u8], mut index: usize) -> bool {
    let token_start = index;
    while index < bytes.len() && is_tchar(bytes[index]) {
        index += 1;
    }

    if index == token_start {
        return false;
    }

    while index < bytes.len() && bytes[index] == b' ' {
        index += 1;
    }

    if index >= bytes.len() || bytes[index] != b'=' {
        return false;
    }

    let mut after_equals = index + 1;
    while after_equals < bytes.len() && bytes[after_equals] == b' ' {
        after_equals += 1;
    }

    if after_equals >= bytes.len() {
        return false;
    }

    if bytes[after_equals] == b'"' {
        return true;
    }

    is_tchar(bytes[after_equals])
}

fn parse_token68(bytes: &[u8], index: &mut usize) -> Result<String, NanoGetError> {
    let start = *index;
    while *index < bytes.len() && is_token68(bytes[*index]) {
        *index += 1;
    }

    if *index == start {
        return Err(NanoGetError::MalformedChallenge(
            String::from_utf8_lossy(bytes).into_owned(),
        ));
    }

    Ok(String::from_utf8_lossy(&bytes[start..*index]).into_owned())
}

fn parse_token(bytes: &[u8], index: &mut usize) -> Option<String> {
    let start = *index;
    while *index < bytes.len() && is_tchar(bytes[*index]) {
        *index += 1;
    }

    if *index == start {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes[start..*index]).into_owned())
    }
}

fn parse_quoted_string(bytes: &[u8], index: &mut usize) -> Result<String, NanoGetError> {
    if *index >= bytes.len() || bytes[*index] != b'"' {
        return Err(NanoGetError::MalformedChallenge(
            String::from_utf8_lossy(bytes).into_owned(),
        ));
    }
    *index += 1;

    let mut value = String::new();
    while *index < bytes.len() {
        match bytes[*index] {
            b'\\' => {
                *index += 1;
                if *index >= bytes.len() {
                    return Err(NanoGetError::MalformedChallenge(
                        String::from_utf8_lossy(bytes).into_owned(),
                    ));
                }
                value.push(bytes[*index] as char);
                *index += 1;
            }
            b'"' => {
                *index += 1;
                return Ok(value);
            }
            byte => {
                value.push(byte as char);
                *index += 1;
            }
        }
    }

    Err(NanoGetError::MalformedChallenge(
        String::from_utf8_lossy(bytes).into_owned(),
    ))
}

fn skip_spaces(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index] == b' ' {
        *index += 1;
    }
}

fn skip_ows_and_commas(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && (bytes[*index] == b' ' || bytes[*index] == b',') {
        *index += 1;
    }
}

fn is_tchar(byte: u8) -> bool {
    matches!(
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
    ) || byte.is_ascii_alphanumeric()
}

fn is_token68(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();

    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let triple = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;

        output.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);

        if chunk.len() > 1 {
            output.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }

        if chunk.len() > 2 {
            output.push(TABLE[(triple & 0x3f) as usize] as char);
        } else {
            output.push('=');
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        basic_authorization_value, looks_like_auth_param, parse_auth_params,
        parse_authenticate_headers, parse_quoted_string, parse_token, AuthDecision, AuthHandler,
        AuthTarget, BasicAuthHandler, Challenge,
    };
    use crate::errors::NanoGetError;
    use crate::request::{Header, Request};
    use crate::response::{HttpVersion, Response};
    use crate::url::Url;

    #[test]
    fn parses_single_challenge() {
        let headers = vec![Header::unchecked("WWW-Authenticate", "Basic realm=\"api\"")];
        let challenges = parse_authenticate_headers(&headers, "www-authenticate").unwrap();
        assert_eq!(challenges.len(), 1);
        assert_eq!(challenges[0].scheme, "Basic");
        assert_eq!(challenges[0].params[0].name, "realm");
        assert_eq!(challenges[0].params[0].value, "api");
    }

    #[test]
    fn parses_multiple_challenges_in_one_field() {
        let headers = vec![Header::unchecked(
            "WWW-Authenticate",
            "Basic realm=\"api\", Bearer token68token",
        )];
        let challenges = parse_authenticate_headers(&headers, "www-authenticate").unwrap();
        assert_eq!(challenges.len(), 2);
        assert_eq!(challenges[1].scheme, "Bearer");
        assert_eq!(challenges[1].token68.as_deref(), Some("token68token"));
    }

    #[test]
    fn parses_multiple_header_fields() {
        let headers = vec![
            Header::unchecked("WWW-Authenticate", "Basic realm=\"one\""),
            Header::unchecked("WWW-Authenticate", "Digest realm=\"two\""),
        ];
        let challenges = parse_authenticate_headers(&headers, "www-authenticate").unwrap();
        assert_eq!(challenges.len(), 2);
    }

    #[test]
    fn parses_quoted_commas_and_escapes() {
        let headers = vec![Header::unchecked(
            "WWW-Authenticate",
            "Digest realm=\"a,b\", title=\"say \\\"hi\\\"\"",
        )];
        let challenges = parse_authenticate_headers(&headers, "www-authenticate").unwrap();
        assert_eq!(challenges[0].params[0].value, "a,b");
        assert_eq!(challenges[0].params[1].value, "say \"hi\"");
    }

    #[test]
    fn rejects_malformed_challenges() {
        let headers = vec![Header::unchecked(
            "WWW-Authenticate",
            "Basic realm=\"unterminated",
        )];
        assert!(parse_authenticate_headers(&headers, "www-authenticate").is_err());
    }

    #[test]
    fn encodes_basic_auth_values() {
        assert_eq!(
            basic_authorization_value("user", "pass"),
            "Basic dXNlcjpwYXNz"
        );
        assert_eq!(basic_authorization_value("user", ""), "Basic dXNlcjo=");
        assert_eq!(basic_authorization_value("", ""), "Basic Og==");
    }

    #[test]
    fn basic_handler_matches_basic_challenges() {
        let handler = BasicAuthHandler::new("user", "pass", AuthTarget::Origin);
        let response = Response {
            version: HttpVersion::Http11,
            status_code: 401,
            reason_phrase: "Unauthorized".to_string(),
            headers: Vec::new(),
            trailers: Vec::new(),
            body: Vec::new(),
        };
        let decision = handler
            .respond(
                AuthTarget::Origin,
                &Url::parse("http://example.com").unwrap(),
                &[Challenge {
                    scheme: "Basic".to_string(),
                    token68: None,
                    params: Vec::new(),
                }],
                &Request::get("http://example.com").unwrap(),
                &response,
            )
            .unwrap();
        assert!(matches!(decision, AuthDecision::UseHeaders(_)));
    }

    #[test]
    fn basic_handler_propagates_header_validation_errors() {
        let handler = BasicAuthHandler {
            header_value: "line\nbreak".to_string(),
            target: AuthTarget::Origin,
        };
        let response = Response {
            version: HttpVersion::Http11,
            status_code: 401,
            reason_phrase: "Unauthorized".to_string(),
            headers: Vec::new(),
            trailers: Vec::new(),
            body: Vec::new(),
        };
        let error = handler
            .respond(
                AuthTarget::Origin,
                &Url::parse("http://example.com").unwrap(),
                &[Challenge {
                    scheme: "Basic".to_string(),
                    token68: None,
                    params: Vec::new(),
                }],
                &Request::get("http://example.com").unwrap(),
                &response,
            )
            .unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidHeaderValue(_)));
    }

    #[test]
    fn basic_handler_returns_no_match_for_other_target_or_scheme() {
        let handler = BasicAuthHandler::new("user", "pass", AuthTarget::Origin);
        let response = Response {
            version: HttpVersion::Http11,
            status_code: 401,
            reason_phrase: "Unauthorized".to_string(),
            headers: Vec::new(),
            trailers: Vec::new(),
            body: Vec::new(),
        };
        let request = Request::get("http://example.com").unwrap();
        let url = Url::parse("http://example.com").unwrap();

        let wrong_target = handler
            .respond(
                AuthTarget::Proxy,
                &url,
                &[Challenge {
                    scheme: "Basic".to_string(),
                    token68: None,
                    params: Vec::new(),
                }],
                &request,
                &response,
            )
            .unwrap();
        assert!(matches!(wrong_target, AuthDecision::NoMatch));

        let wrong_scheme = handler
            .respond(
                AuthTarget::Origin,
                &url,
                &[Challenge {
                    scheme: "Digest".to_string(),
                    token68: None,
                    params: Vec::new(),
                }],
                &request,
                &response,
            )
            .unwrap();
        assert!(matches!(wrong_scheme, AuthDecision::NoMatch));
    }

    #[test]
    fn parse_headers_handles_empty_and_malformed_token68_cases() {
        let empty = parse_authenticate_headers(&[], "www-authenticate").unwrap();
        assert!(empty.is_empty());

        let trailing = vec![Header::unchecked(
            "WWW-Authenticate",
            "Basic realm=\"a\", ,",
        )];
        let challenges = parse_authenticate_headers(&trailing, "www-authenticate").unwrap();
        assert_eq!(challenges.len(), 1);

        let malformed = vec![Header::unchecked("WWW-Authenticate", "Bearer ?")];
        assert!(matches!(
            parse_authenticate_headers(&malformed, "www-authenticate"),
            Err(NanoGetError::MalformedChallenge(_))
        ));

        let bare_scheme = vec![Header::unchecked(
            "WWW-Authenticate",
            "Negotiate, Basic realm=\"api\"",
        )];
        let challenges = parse_authenticate_headers(&bare_scheme, "www-authenticate").unwrap();
        assert_eq!(challenges[0].scheme, "Negotiate");
        assert!(challenges[0].token68.is_none());
    }

    #[test]
    fn private_parser_helpers_cover_error_paths() {
        let mut index = 0usize;
        let error = parse_auth_params(b"=oops", &mut index).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedChallenge(_)));

        let mut index = 0usize;
        let error = parse_auth_params(b"realm x", &mut index).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedChallenge(_)));

        let mut index = 5usize;
        let error = parse_auth_params(b"realm", &mut index).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedChallenge(_)));

        let mut index = 0usize;
        let error = parse_auth_params(b"realm= ", &mut index).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedChallenge(_)));

        let bytes = b"token=   ";
        assert!(!looks_like_auth_param(bytes, 0));
        let bytes = b"token =\"x\"";
        assert!(looks_like_auth_param(bytes, 0));
        let bytes = b"token =!";
        assert!(looks_like_auth_param(bytes, 0));

        let mut token_index = 0usize;
        assert!(parse_token(b"=", &mut token_index).is_none());

        let mut quoted_index = 0usize;
        let error = parse_quoted_string(b"token", &mut quoted_index).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedChallenge(_)));

        let mut escaped_index = 0usize;
        let error = parse_quoted_string(br#""unterminated\"#, &mut escaped_index).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedChallenge(_)));
    }

    struct NoopHandler;

    impl AuthHandler for NoopHandler {
        fn respond(
            &self,
            _target: AuthTarget,
            _url: &Url,
            _challenges: &[Challenge],
            _request: &Request,
            _response: &Response,
        ) -> Result<AuthDecision, NanoGetError> {
            Ok(AuthDecision::NoMatch)
        }
    }

    #[test]
    fn auth_handlers_are_object_safe() {
        let _handler: Arc<dyn AuthHandler + Send + Sync> = Arc::new(NoopHandler);
    }

    #[test]
    fn noop_handler_returns_nomatch() {
        let handler = NoopHandler;
        let decision = handler
            .respond(
                AuthTarget::Origin,
                &Url::parse("http://example.com").unwrap(),
                &[],
                &Request::get("http://example.com").unwrap(),
                &Response {
                    version: HttpVersion::Http11,
                    status_code: 401,
                    reason_phrase: "Unauthorized".to_string(),
                    headers: Vec::new(),
                    trailers: Vec::new(),
                    body: Vec::new(),
                },
            )
            .unwrap();
        assert!(matches!(decision, AuthDecision::NoMatch));
    }
}
