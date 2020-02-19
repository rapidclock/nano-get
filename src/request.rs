use std::collections::HashMap;
use std::error::Error;

use super::{ToUrl, Url};
use super::errors::NanoGetError;
use super::http::request_http_get;
#[cfg(feature = "https")]
use super::https::request_https_get;
use super::Response;
#[cfg(feature = "async")]
use super::asyn;

/// This is the basic HTTP Request Object.
///
/// This is self-containing and you can execute the request using its execute method.
/// It invokes a http or https version depending on the protocol of the embedded url and
/// based on the `"https"` feature flag.
///
/// The major difference between this and the usual `get` method in the crate is the
/// more fine grained control you get in the request and response.
///
/// Running the HTTP(s) method is as simple as calling `Request.execute()` on the
/// constructed request. This returns a `Response` object instead of the body of the response as
/// a String.
///
/// ## Request Body
/// Although, the standard doesn't recommend sending a body with a get request, you can provide an
/// optional body for the request.
/// ### Example
/// ```rust
/// let mut request = nano_get::Request::default_get_request("http://example.com/").unwrap();
/// request.body = Some("Hello World!".to_string());
/// ```
///
/// ## Additional Request Headers
/// You can provide additional headers as part of your request by using the `add_header(key: &str, value: &str)`
/// method. These will be sent along with the default headers as part of the request.
/// ### Example
/// ```rust
/// let mut request = nano_get::Request::default_get_request("http://example.com/").unwrap();
/// request.add_header("test", "value testing");
/// ```
///
/// ## Executing the Request
/// As mentioned earlier, executing the request is as simple as calling `Request.execute()`.
///
/// This is similar to the basic unified HTTP GET in this crate `nano_get::get()`, in the way it
/// handles http/https.
///
/// If the protocol of the embedded url is https and if the `"https"` feature flag is present,
/// the https version of get, based on the [openssl](https://crates.io/crates/openssl) crate is executed.
///
/// The fall-back is the regular HTTP GET.
///
/// ### Example
/// For regular HTTP GET requests,
/// ```rust
/// use nano_get::Response;
/// let mut request = nano_get::Request::default_get_request("http://example.com/").unwrap();
/// request.add_header("test", "value testing");
/// let response: Response = request.execute().unwrap();
/// ```
#[derive(Debug)]
pub struct Request {
    /// The embedded Url that is part of the request. This is used while executing the HTTP Request.
    pub url: Url,
    request_type: RequestType,
    headers: Option<HashMap<String, String>>,
    /// The optional body of the request, that is sent while executing the request.
    pub body: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug)]
enum RequestType {
    HEAD,
    GET,
    PUT,
    POST,
    DELETE,
    OPTIONS,
    CUSTOM(String),
}

impl RequestType {
    fn value(&self) -> &'static str {
        match self {
            RequestType::GET => "GET",
            RequestType::HEAD => "HEAD",
            RequestType::POST => "POST",
            RequestType::PUT => "PUT",
            RequestType::DELETE => "DELETE",
            RequestType::OPTIONS => "OPTIONS",
            RequestType::CUSTOM(_) => "CUSTOM",
        }
    }
}

/// Coveneince wrapper for a tuple of (key: &str, value: &str) that is to be sent as a HTTP header.
pub type Header<'a> = (&'a str, &'a str);

impl Request {
    /// Creates a new Request object, based on the url, and optional headers.
    ///
    /// ## Examples
    /// ```rust
    /// use nano_get::Request;
    /// let request = Request::new("http://example.com", None, None);
    /// ```
    ///
    /// To include custom headers,
    /// ```rust
    /// use nano_get::Request;
    /// let request_headers = vec![("header1", "value1"), ("header2", "value2")];
    /// let request = Request::new("http://example.com", Some(request_headers), None);
    /// ```
    ///
    /// To include custom headers and body
    /// ```rust
    /// use nano_get::Request;
    /// let request_headers = vec![("header1", "value1"), ("header2", "value2")];
    /// let request_body = "Hello World!!".to_string();
    /// let request = Request::new("http://example.com", Some(request_headers), Some(request_body));
    /// ```
    pub fn new<A: ToUrl>(url: A, headers: Option<Vec<Header>>, body: Option<String>) -> Result<Self, Box<dyn Error>> {
        let url = url.to_url()?;
        let mut request = Request {
            url,
            request_type: RequestType::GET,
            headers: None,
            body,
        };
        request.headers = Some(Self::get_default_headers(&request.url));
        let addnl_headers = process_headers(headers);
        request.merge_addnl_headers(addnl_headers);
        Ok(request)
    }

    fn merge_addnl_headers(&mut self, addnl_headers: Option<HashMap<String, String>>) {
        if self.headers.is_some() {
            let headers = self.headers.as_mut().unwrap();
            if let Some(extra_headers) = addnl_headers {
                for (k, v) in extra_headers {
                    headers.insert(k, v);
                }
            }
        } else {
            self.headers = addnl_headers;
        }
    }

    /// Simplified version to create a Request based only on the given Url.
    ///
    /// Default Headers are inserted and the Body is set to `None`.
    /// The values can be modified if required later.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use nano_get::Request;
    /// let request = Request::default_get_request("http://example.com");
    /// ```
    pub fn default_get_request<A: ToUrl>(url: A) -> Result<Self, Box<dyn Error>> {
        Self::new(url, None, None)
    }

    fn get_default_headers(url: &Url) -> HashMap<String, String> {
        let mut headers = HashMap::with_capacity(4);
        headers.insert("user-agent".to_string(), "mini-get/0.1.0".to_string());
        headers.insert("accept".to_string(), "*/*".to_string());
        headers.insert("host".to_string(), url.host.clone());
        headers.insert("connection".to_string(), "close".to_string());
        headers
    }

    /// Executes the request and returns a `nano_get::Response` object based `std::result::Result`.
    ///
    /// If the protocol of the embedded url is https and if the `"https"` feature flag is present,
    /// the https version of get, based on the [openssl](https://crates.io/crates/openssl) crate is executed.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use nano_get::Response;
    ///
    /// let mut request = nano_get::Request::default_get_request("http://example.com/").unwrap();
    /// request.add_header("test", "value testing");
    /// let response: Response = request.execute().unwrap();
    /// println!(response.status);
    /// println!(response.body);
    /// ```
    pub fn execute(&self) -> Result<Response, NanoGetError> {
        #[cfg(feature = "https")] {
            if self.is_https() {
                return request_https_get(&self);
            }
        }
        request_http_get(&self)
    }

    #[cfg(feature = "async")]
    pub async fn async_exec(&self) -> Result<Response, NanoGetError> {
        todo!()
    }

    /// Returns the headers as an Iterator over the key-value pairs.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use nano_get::Response;
    ///
    /// let mut request = nano_get::Request::default_get_request("http://example.com/").unwrap();
    /// request.add_header("test", "value testing");
    /// for (k, v) in request.get_request_headers() {
    ///     println!("{}, {}", k, v);
    /// }
    /// ```
    pub fn get_request_headers(&self) -> impl Iterator<Item=(&str, &str)> {
        self.headers.as_ref().unwrap().iter().map(|(k, v)| {
            (k.as_str(), v.as_str())
        })
    }

    /// Convenience method to check if the request is a https request based
    /// on the embedded url's protocol.
    pub fn is_https(&self) -> bool {
        self.url.protocol.as_str() == "https"
    }

    /// Returns the type of HTTP Request.
    ///
    /// Currently only returns `"GET"`. For Future Use.
    pub fn get_request_type(&self) -> &str {
        self.request_type.value()
    }

    /// Add an additional header to the request.
    ///
    /// You can overwrite existing values by adding the header with the new value.
    ///
    /// You cannot however remove the presence of a header.
    pub fn add_header(&mut self, key: &str, value: &str) {
        if self.headers.is_some() {
            self.headers.as_mut().unwrap().insert((*key).to_string(), (*value).to_string());
        } else {
            let mut headers = HashMap::new();
            headers.insert((*key).to_string(), (*value).to_string());
            self.headers = Some(headers);
        }
    }
}

fn process_headers(headers: Option<Vec<Header>>) -> Option<HashMap<String, String>> {
    headers.map(|vec| {
        vec.iter().cloned().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    })
}