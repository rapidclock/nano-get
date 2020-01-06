//! This crate provides a basic implementation of the HTTP(s) GET Method.
//!
//! This uses only the standard Rust Library and has no 3rd party dependencies by default.
//!
//! ## Quick Example
//!
//! An example usage is shown below:
//! ```rust
//! let response = nano_get::get_http("http://example.com");
//! println!("{}", response);
//! ```
//!
//! ## HTTPS
//!
//! A HTTPS version is provided since v0.2.x that depends on OpenSSL & the [Rust OpenSSL wrapper](https://crates.io/crates/openssl) crate.
//! This can be enabled by the "https" feature flag (which is NOT activated by default).
//!
//! This provides you with the `nano_get::get_https` method which has the same signature as
//! the standard `nano_get::get_http` method.
//!
//! ## Unified HTTP GET
//!
//! From version 0.2.x, we profide the new unified `nano_get::get` method which activates the
//! specific version of http/https GET, based on the protocol of the URL.
//!
//! However, please note that the https part still depends on the "https" feature flag
//! (which is NOT activated by default). The `nano_get::get` falls back on the regular http version,
//! incase the "https" feature flag is not enabled.
//!
//! An example usage of the unified get is shown below:
//! ```rust
//! let response = nano_get::get("http://dummy.restapiexample.com/api/v1/employees");
//! println!("{}", response);
//! ```
//!
//! or, with the "https" feature flag enabled and the OpenSSL library present,
//!
//! ```rust
//! let response = nano_get::get("https://www.google.com");
//! println!("{}", response);
//! ```
//!
//! ## Executing HTTP(s) Requests:
//!
//! There are two ways to execute the HTTP(s) requests.
//!
//! ### Basic Get
//!
//! The basic version, demonstrated by the use of the `nano_get::get` function, which takes a url
//! and returns the body of the response.
//!
//! #### Example
//! ```rust
//! let response = nano_get::get("https://www.google.com");
//! println!("{}", response);
//! ```
//!
//! ### Request-Response based
//!
//! Another more fine-grained method exists by using the `nano_get::Request` object.
//! This gives you access to request headers, optional request body and the execution returns a
//! `nano_get::Response` object. This allows inspection of HTTP Response codes, response body, etc.
//!
//! #### Example
//! ```rust
//! use nano_get::Response;
//! let mut request = nano_get::Request::default_get_request("http://example.com/").unwrap();
//! let response: Response = request.execute().unwrap();
//! ```
//!
//! For details, check the `Request` and `Response` structure documentation.
pub use http::get_http;
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
