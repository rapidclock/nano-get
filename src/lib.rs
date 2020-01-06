//! This crate provides a basic implementation of the HTTP GET Method.
//! This uses only the standard Rust Library and has no 3rd party dependencies by default.
//!
//! ## Quick Example
//!
//! An example usage is shown below:
//! ```rust
//! let response = nano_get::get_http("http://dummy.restapiexample.com/api/v1/employees");
//! println!("{}", response);
//! ```
//!
//! ## HTTPS
//!
//! A HTTPS version is provided since v0.2.0 that depends on OpenSSL & the Rust OpenSSL wrapper lib.
//! This can be enabled by the "https" feature flag (which is NOT activated by default).
//!
//! This provides you with the `nano_get::get_https` method which has the same signature as
//! the standard `nano_get::get_http` method.
//!
//! ## Unified HTTP GET
//!
//! From version 0.2.0, we profide the new unified `nano_get::get` method which activates the
//! specific version of http/https get, based on the protocol of the URL.
//!
//! However, please note that the https part still depends on the "https" feature flag
//! (which is NOT activated by default). The `nano_get::get` falls back on the regular http version,
//! incase the "https" feature flag is not enabled.
//!
//! An example usage of the unified get is shown below:
//! ```rust
//! let response = nano_get::get_http("http://dummy.restapiexample.com/api/v1/employees");
//! println!("{}", response);
//! ```
//!
//! or, with the "https" feature flag enabled and the OpenSSL library present,
//!
//! ```rust
//! let response = nano_get::get("https://www.google.com");
//! println!("{}", response);
//! ```

pub use http::{execute, get_http};
#[cfg(feature = "https")]
pub use https::get_https;
pub use request::{Header, Request};
pub use response::{Response, ResponseStatus, StatusCode};
pub use url::{ToUrl, Url};

mod url;
mod http;
mod request;
mod response;
mod errors;

#[cfg(feature = "https")]
mod https;

/// This is a unified function for the HTTP GET method.
///
/// This calls the http version of GET provided in this crate by default.
///
/// If the "https" feature flag is enabled, then this calls the get_https method, if the protocol is
/// https, else it calls the get_http method. The "https" feature flag is NOT enabled by default.
///
/// This function is a wrapper around the http/https get methods provided in this crate.
///
/// If you require manual control of the method that is called, you should use the specific method.
///
/// This can be called on anything that implements the ToUrl Trait.
#[allow(unused_variables)]
pub fn get<U: ToUrl>(url: U) -> String {
    let url = url.to_url().unwrap();
    let protocol = &url.protocol[..];

    #[cfg(feature = "https")] {
        if protocol.eq("https") {
            return get_https(&url);
        }
    }

    get_http(&url)
}

#[cfg(test)]
mod tests {
    use url;

    use super::*;

    #[test]
    fn test_proto_parse_http() {
        let url_str = "http://example.com/?a=1&b=2&c=3".to_string();
        let (a, b) = url::parse_proto(url_str, None);
        println!("{}, {}", a, b);
        assert_eq!(a, "http".to_string());
    }

    #[test]
    fn test_proto_parse_https() {
        let url_str = "https://example.com/?a=1&b=2&c=3".to_string();
        let (a, b) = url::parse_proto(url_str, None);
        println!("{}, {}", a, b);
        assert_eq!(a, "https".to_string());
    }

    #[test]
    fn test_proto_parse_ftp() {
        let url_str = "ftp://example.com/?a=1&b=2&c=3".to_string();
        let (a, b) = url::parse_proto(url_str, None);
        println!("{}, {}", a, b);
        assert_eq!(a, "ftp".to_string());
    }

    #[test]
    fn test_proto_parse_none() {
        let url_str = "example.com/?a=1&b=2&c=3".to_string();
        let (a, b) = url::parse_proto(url_str, None);
        println!("{}, {}", a, b);
        assert_eq!(a, "http".to_string());
    }
}
