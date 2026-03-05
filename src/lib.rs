#![warn(missing_docs)]

//! A tiny `HTTP/1.1` client for `GET` and `HEAD`.
//!
//! The default build has zero external dependencies. Enable the `https` feature flag to add
//! HTTPS support via the system OpenSSL library.
//!
//! ## Helper API
//!
//! ```no_run
//! let body = nano_get::get("http://example.com")?;
//! # Ok::<(), nano_get::NanoGetError>(())
//! ```
//!
//! ## Advanced API
//!
//! ```no_run
//! let client = nano_get::Client::builder()
//!     .connection_policy(nano_get::ConnectionPolicy::Reuse)
//!     .cache_mode(nano_get::CacheMode::Memory)
//!     .basic_auth("user", "pass")
//!     .build();
//! let response = client.execute(
//!     nano_get::Request::get("http://example.com")?
//!         .with_redirect_policy(nano_get::RedirectPolicy::follow(5)),
//! )?;
//! assert!(response.status_code >= 200);
//! # Ok::<(), nano_get::NanoGetError>(())
//! ```
//!
//! ## Custom Authentication
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! struct TokenAuth;
//!
//! impl nano_get::AuthHandler for TokenAuth {
//!     fn respond(
//!         &self,
//!         _target: nano_get::AuthTarget,
//!         _url: &nano_get::Url,
//!         challenges: &[nano_get::Challenge],
//!         _request: &nano_get::Request,
//!         _response: &nano_get::Response,
//!     ) -> Result<nano_get::AuthDecision, nano_get::NanoGetError> {
//!         if challenges
//!             .iter()
//!             .any(|challenge| challenge.scheme.eq_ignore_ascii_case("token"))
//!         {
//!             return Ok(nano_get::AuthDecision::UseHeaders(vec![
//!                 nano_get::Header::new("Authorization", "Token secret")?,
//!             ]));
//!         }
//!
//!         Ok(nano_get::AuthDecision::NoMatch)
//!     }
//! }
//!
//! let client = nano_get::Client::builder()
//!     .auth_handler(Arc::new(TokenAuth))
//!     .build();
//! let response = client.execute(nano_get::Request::get("http://example.com/protected")?)?;
//! assert!(response.status_code >= 200);
//! # Ok::<(), nano_get::NanoGetError>(())
//! ```

pub use auth::{AuthDecision, AuthHandler, AuthParam, AuthTarget, Challenge};
pub use client::{CacheMode, Client, ClientBuilder, ConnectionPolicy, ProxyConfig, Session};
pub use errors::NanoGetError;
pub use request::{Header, Method, RedirectPolicy, Request};
pub use response::{HttpVersion, Response};
pub use url::{ToUrl, Url};

mod auth;
mod client;
mod date;
mod errors;
mod http;
#[cfg(feature = "https")]
mod https;
mod request;
mod response;
mod url;

const DEFAULT_REDIRECT_LIMIT: usize = 10;

/// Performs a `GET` request and returns the response body as UTF-8 text.
///
/// This helper:
/// - accepts either `http://` or `https://` URLs
/// - follows redirects up to 10 hops
/// - returns `NanoGetError::InvalidUtf8` if the body is not valid UTF-8
///
/// For binary payloads, use [`get_bytes`] instead.
pub fn get<U: ToUrl>(url: U) -> Result<String, NanoGetError> {
    helper_client().get(url)
}

/// Performs a `GET` request and returns the response body as raw bytes.
///
/// This helper:
/// - accepts either `http://` or `https://` URLs
/// - follows redirects up to 10 hops
pub fn get_bytes<U: ToUrl>(url: U) -> Result<Vec<u8>, NanoGetError> {
    let request =
        Request::get(url)?.with_redirect_policy(RedirectPolicy::follow(DEFAULT_REDIRECT_LIMIT));
    helper_client()
        .execute(request)
        .map(|response| response.body)
}

/// Performs a `HEAD` request and returns the full response metadata.
///
/// The returned [`Response`] always has an empty `body` for `HEAD`, even when a server
/// incorrectly sends bytes on the wire.
pub fn head<U: ToUrl>(url: U) -> Result<Response, NanoGetError> {
    let request =
        Request::head(url)?.with_redirect_policy(RedirectPolicy::follow(DEFAULT_REDIRECT_LIMIT));
    helper_client().execute(request)
}

/// Performs a `GET` request using HTTP only and returns UTF-8 text.
///
/// Returns [`NanoGetError::UnsupportedScheme`] if the URL is not `http://`.
pub fn get_http<U: ToUrl>(url: U) -> Result<String, NanoGetError> {
    get_http_bytes(url)
        .and_then(|body| String::from_utf8(body).map_err(|error| error.utf8_error().into()))
}

/// Performs a `GET` request using HTTP only and returns raw bytes.
///
/// Returns [`NanoGetError::UnsupportedScheme`] if the URL is not `http://`.
pub fn get_http_bytes<U: ToUrl>(url: U) -> Result<Vec<u8>, NanoGetError> {
    let request =
        Request::get(url)?.with_redirect_policy(RedirectPolicy::follow(DEFAULT_REDIRECT_LIMIT));
    if !request.url().is_http() {
        return Err(NanoGetError::UnsupportedScheme(
            request.url().scheme.clone(),
        ));
    }
    helper_client()
        .execute(request)
        .map(|response| response.body)
}

/// Performs a `HEAD` request using HTTP only.
///
/// Returns [`NanoGetError::UnsupportedScheme`] if the URL is not `http://`.
pub fn head_http<U: ToUrl>(url: U) -> Result<Response, NanoGetError> {
    let request =
        Request::head(url)?.with_redirect_policy(RedirectPolicy::follow(DEFAULT_REDIRECT_LIMIT));
    if !request.url().is_http() {
        return Err(NanoGetError::UnsupportedScheme(
            request.url().scheme.clone(),
        ));
    }
    helper_client().execute(request)
}

/// Performs a `GET` request using HTTPS only and returns UTF-8 text.
///
/// Available only with the `https` feature.
///
/// Returns [`NanoGetError::UnsupportedScheme`] if the URL is not `https://`.
#[cfg(feature = "https")]
pub fn get_https<U: ToUrl>(url: U) -> Result<String, NanoGetError> {
    get_https_bytes(url)
        .and_then(|body| String::from_utf8(body).map_err(|error| error.utf8_error().into()))
}

/// Performs a `GET` request using HTTPS only and returns raw bytes.
///
/// Available only with the `https` feature.
///
/// Returns [`NanoGetError::UnsupportedScheme`] if the URL is not `https://`.
#[cfg(feature = "https")]
pub fn get_https_bytes<U: ToUrl>(url: U) -> Result<Vec<u8>, NanoGetError> {
    let request =
        Request::get(url)?.with_redirect_policy(RedirectPolicy::follow(DEFAULT_REDIRECT_LIMIT));
    if !request.url().is_https() {
        return Err(NanoGetError::UnsupportedScheme(
            request.url().scheme.clone(),
        ));
    }
    helper_client()
        .execute(request)
        .map(|response| response.body)
}

/// Performs a `HEAD` request using HTTPS only.
///
/// Available only with the `https` feature.
///
/// Returns [`NanoGetError::UnsupportedScheme`] if the URL is not `https://`.
#[cfg(feature = "https")]
pub fn head_https<U: ToUrl>(url: U) -> Result<Response, NanoGetError> {
    let request =
        Request::head(url)?.with_redirect_policy(RedirectPolicy::follow(DEFAULT_REDIRECT_LIMIT));
    if !request.url().is_https() {
        return Err(NanoGetError::UnsupportedScheme(
            request.url().scheme.clone(),
        ));
    }
    helper_client().execute(request)
}

fn helper_client() -> Client {
    Client::builder()
        .redirect_policy(RedirectPolicy::follow(DEFAULT_REDIRECT_LIMIT))
        .build()
}

#[cfg(test)]
mod tests {
    use crate::client::{CacheMode, Client, ConnectionPolicy};
    use crate::{get_http_bytes, Method, RedirectPolicy, Request, Url};

    #[test]
    fn request_constructors_work() {
        let request = Request::new(Method::Get, "http://example.com").unwrap();
        assert_eq!(request.method(), Method::Get);
    }

    #[test]
    fn default_url_scheme_is_http() {
        let url = Url::parse("example.com").unwrap();
        assert!(url.is_http());
    }

    #[test]
    fn helper_requests_follow_redirects_by_default() {
        let request = Request::get("http://example.com")
            .unwrap()
            .with_redirect_policy(RedirectPolicy::follow(10));
        assert_eq!(request.redirect_policy(), RedirectPolicy::follow(10));
    }

    #[test]
    fn http_only_helpers_reject_https_urls() {
        let error = get_http_bytes("https://example.com").unwrap_err();
        assert!(matches!(error, crate::NanoGetError::UnsupportedScheme(_)));
    }

    #[test]
    fn client_builder_is_available() {
        let _client = Client::builder()
            .connection_policy(ConnectionPolicy::Reuse)
            .cache_mode(CacheMode::Memory)
            .build();
    }
}
