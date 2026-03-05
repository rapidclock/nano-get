use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io;
use std::str::Utf8Error;

/// Error type for all fallible operations in this crate.
#[derive(Debug)]
pub enum NanoGetError {
    /// URL input was invalid.
    InvalidUrl(String),
    /// URL scheme is unsupported.
    UnsupportedScheme(String),
    /// Proxy URL scheme is unsupported.
    UnsupportedProxyScheme(String),
    /// HTTPS was requested without enabling the `https` feature.
    HttpsFeatureRequired,
    /// Header name failed validation.
    InvalidHeaderName(String),
    /// Header value failed validation.
    InvalidHeaderValue(String),
    /// TCP connect operation failed.
    Connect(io::Error),
    /// Generic I/O failure.
    Io(io::Error),
    /// TLS handshake or TLS configuration failure.
    Tls(String),
    /// HTTP `CONNECT` tunnel setup failed with `(status_code, reason_phrase)`.
    ProxyConnectFailed(u16, String),
    /// Authentication challenge syntax was malformed.
    MalformedChallenge(String),
    /// HTTP status line syntax was malformed.
    MalformedStatusLine(String),
    /// Header block syntax was malformed.
    MalformedHeader(String),
    /// `Content-Length` was invalid or conflicting.
    InvalidContentLength(String),
    /// Chunked transfer framing was invalid.
    InvalidChunk(String),
    /// Transfer encoding is unsupported by this crate.
    UnsupportedTransferEncoding(String),
    /// Response used ambiguous body framing.
    AmbiguousResponseFraming(String),
    /// Response body ended before the declared framing boundary.
    IncompleteMessage(String),
    /// Redirect chain exceeded the configured maximum.
    RedirectLimitExceeded(usize),
    /// Response body could not be decoded as UTF-8.
    InvalidUtf8(Utf8Error),
    /// In-memory cache operation failed.
    Cache(String),
    /// Pipelining operation failed.
    Pipeline(String),
    /// Generic authentication error.
    Authentication(String),
    /// Authentication retries would loop.
    AuthenticationLoop(String),
    /// Authentication handler explicitly rejected continuation.
    AuthenticationRejected(String),
    /// Caller attempted to set a protocol-managed header.
    ProtocolManagedHeader(String),
    /// Caller attempted to set a forbidden hop-by-hop header.
    HopByHopHeader(String),
    /// Conditional request header combination was invalid.
    InvalidConditionalRequest(String),
}

impl NanoGetError {
    pub(crate) fn invalid_url(message: impl Into<String>) -> Self {
        Self::InvalidUrl(message.into())
    }
}

impl Display for NanoGetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(message) => write!(f, "invalid URL: {message}"),
            Self::UnsupportedScheme(scheme) => write!(f, "unsupported URL scheme: {scheme}"),
            Self::UnsupportedProxyScheme(scheme) => {
                write!(f, "unsupported proxy URL scheme: {scheme}")
            }
            Self::HttpsFeatureRequired => {
                write!(f, "the `https` feature flag is required for HTTPS requests")
            }
            Self::InvalidHeaderName(name) => write!(f, "invalid header name: {name}"),
            Self::InvalidHeaderValue(value) => write!(f, "invalid header value: {value}"),
            Self::Connect(error) => write!(f, "failed to connect: {error}"),
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Tls(message) => write!(f, "TLS error: {message}"),
            Self::ProxyConnectFailed(code, reason) => {
                write!(f, "proxy CONNECT failed with status {code}: {reason}")
            }
            Self::MalformedChallenge(value) => {
                write!(f, "malformed authentication challenge: {value}")
            }
            Self::MalformedStatusLine(line) => write!(f, "malformed status line: {line}"),
            Self::MalformedHeader(line) => write!(f, "malformed header: {line}"),
            Self::InvalidContentLength(value) => write!(f, "invalid content-length: {value}"),
            Self::InvalidChunk(message) => write!(f, "invalid chunked body: {message}"),
            Self::UnsupportedTransferEncoding(value) => {
                write!(f, "unsupported transfer-encoding: {value}")
            }
            Self::AmbiguousResponseFraming(message) => {
                write!(f, "ambiguous response framing: {message}")
            }
            Self::IncompleteMessage(message) => write!(f, "incomplete message: {message}"),
            Self::RedirectLimitExceeded(limit) => {
                write!(f, "redirect limit exceeded after {limit} hops")
            }
            Self::InvalidUtf8(error) => write!(f, "response body is not valid UTF-8: {error}"),
            Self::Cache(message) => write!(f, "cache error: {message}"),
            Self::Pipeline(message) => write!(f, "pipeline error: {message}"),
            Self::Authentication(message) => write!(f, "authentication error: {message}"),
            Self::AuthenticationLoop(message) => {
                write!(f, "authentication retry loop detected: {message}")
            }
            Self::AuthenticationRejected(message) => {
                write!(f, "authentication rejected: {message}")
            }
            Self::ProtocolManagedHeader(name) => {
                write!(
                    f,
                    "header is managed by the protocol implementation: {name}"
                )
            }
            Self::HopByHopHeader(name) => write!(f, "hop-by-hop header is not allowed: {name}"),
            Self::InvalidConditionalRequest(message) => {
                write!(f, "invalid conditional request: {message}")
            }
        }
    }
}

impl Error for NanoGetError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Connect(error) | Self::Io(error) => Some(error),
            Self::InvalidUtf8(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for NanoGetError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<Utf8Error> for NanoGetError {
    fn from(error: Utf8Error) -> Self {
        Self::InvalidUtf8(error)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::io;

    use super::NanoGetError;

    #[test]
    fn formats_new_error_variants() {
        let ambiguous = NanoGetError::AmbiguousResponseFraming("bad".to_string());
        assert_eq!(ambiguous.to_string(), "ambiguous response framing: bad");

        let incomplete = NanoGetError::IncompleteMessage("eof".to_string());
        assert_eq!(incomplete.to_string(), "incomplete message: eof");

        let conditional = NanoGetError::InvalidConditionalRequest("invalid".to_string());
        assert_eq!(
            conditional.to_string(),
            "invalid conditional request: invalid"
        );
    }

    #[test]
    fn formats_all_error_variants_and_sources() {
        let invalid = vec![0xff];
        let utf8_error = std::str::from_utf8(&invalid).unwrap_err();
        let io_error = io::Error::new(io::ErrorKind::Other, "io");
        let connect_error = io::Error::new(io::ErrorKind::ConnectionRefused, "connect");

        let variants = vec![
            NanoGetError::InvalidUrl("url".to_string()),
            NanoGetError::UnsupportedScheme("ftp".to_string()),
            NanoGetError::UnsupportedProxyScheme("https".to_string()),
            NanoGetError::HttpsFeatureRequired,
            NanoGetError::InvalidHeaderName("x".to_string()),
            NanoGetError::InvalidHeaderValue("y".to_string()),
            NanoGetError::Connect(connect_error),
            NanoGetError::Io(io_error),
            NanoGetError::Tls("tls".to_string()),
            NanoGetError::ProxyConnectFailed(407, "Proxy".to_string()),
            NanoGetError::MalformedChallenge("challenge".to_string()),
            NanoGetError::MalformedStatusLine("status".to_string()),
            NanoGetError::MalformedHeader("header".to_string()),
            NanoGetError::InvalidContentLength("len".to_string()),
            NanoGetError::InvalidChunk("chunk".to_string()),
            NanoGetError::UnsupportedTransferEncoding("gzip".to_string()),
            NanoGetError::AmbiguousResponseFraming("ambiguous".to_string()),
            NanoGetError::IncompleteMessage("incomplete".to_string()),
            NanoGetError::RedirectLimitExceeded(3),
            NanoGetError::InvalidUtf8(utf8_error),
            NanoGetError::Cache("cache".to_string()),
            NanoGetError::Pipeline("pipeline".to_string()),
            NanoGetError::Authentication("auth".to_string()),
            NanoGetError::AuthenticationLoop("loop".to_string()),
            NanoGetError::AuthenticationRejected("rejected".to_string()),
            NanoGetError::ProtocolManagedHeader("host".to_string()),
            NanoGetError::HopByHopHeader("te".to_string()),
            NanoGetError::InvalidConditionalRequest("conditional".to_string()),
        ];

        for error in variants {
            assert!(!error.to_string().is_empty());
        }

        let io_error = NanoGetError::from(io::Error::new(io::ErrorKind::Other, "io"));
        assert!(io_error.source().is_some());
        let invalid = vec![0xff];
        let utf8_error = NanoGetError::from(std::str::from_utf8(&invalid).unwrap_err());
        assert!(utf8_error.source().is_some());
        assert!(NanoGetError::UnsupportedScheme("ftp".to_string())
            .source()
            .is_none());
    }
}
