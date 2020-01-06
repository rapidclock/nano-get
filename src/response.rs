use std::collections::HashMap;
use std::fmt::{Display, Error, Formatter};

use super::url::Tuple;

/// This is the HTTP Reponse Object.
///
/// Represents the Response from executing a HTTP `Request`.
///
/// This allows inspection of the HTTP Status Code & Reason and HTTP Response Body.
///
/// ## Example
/// ```rust
/// use nano_get::Response;
/// let mut request = nano_get::Request::default_get_request("http://example.com/").unwrap();
/// request.add_header("test", "value testing");
/// let response: Response = request.execute().unwrap();
/// println!("Status: {}", response.status);
/// println!("Body: {}", response.body);
/// ```
pub struct Response {
    /// The status of the Response.
    pub status: ResponseStatus,
    /// The body of the Response.
    pub body: String,
    headers: Option<HashMap<String, String>>,
}

impl Response {
    /// Get an iterator of the Headers in the Response.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use nano_get::Response;
    ///
    /// let mut request = nano_get::Request::default_get_request("http://example.com/").unwrap();
    /// request.add_header("test", "value testing");
    /// let response = request.execute().unwrap();
    /// for (k, v) in response.get_response_headers().unwrap() {
    ///     println!("{}, {}", k, v);
    /// }
    /// ```
    pub fn get_response_headers(&self) -> Option<impl Iterator<Item=(&str, &str)>> {
        if self.headers.is_none() {
            return None;
        }
        Some(self.headers.as_ref().unwrap().iter().map(|(k, v)| {
            (k.as_str(), v.as_str())
        }))
    }

    /// Returns the status code of the Response as an unsigned 16-bit Integer (u16).
    ///
    /// Provided as a convenience. This can be got through the embedded `ResponseStatus` also.
    pub fn get_status_code(&self) -> Option<u16> {
        self.status.0.get_code()
    }
}

pub fn new_response_from_complete(response: String) -> Response {
    let lines: Vec<&str> = response.splitn(2, "\r\n\r\n").collect();
    let heads = (*lines.first().unwrap()).to_string();
    let head_lines: Vec<&str> = heads.split("\r\n").collect();
    let (resp_state, headers) = process_head_lines(head_lines);
    let body = (*lines.last().unwrap()).to_string();
    Response {
        status: resp_state,
        body,
        headers,
    }
}

fn process_head_lines(lines: Vec<&str>) -> (ResponseStatus, Option<HashMap<String, String>>) {
    let head = *lines.get(0).unwrap();
    let parts: Vec<&str> = head.split(' ').collect();
    let status_code = StatusCode::from_code(parts.get(1).unwrap());
    let reason = parts.get(2).map(|v| (*v).to_string());
    let response_headers = process_response_headers(&lines[1..]);
    (ResponseStatus(status_code, reason), response_headers)
}

fn process_response_headers(lines: &[&str]) -> Option<HashMap<String, String>> {
    if lines.is_empty() {
        None
    } else {
        let mut headers = HashMap::new();
        for &line in lines {
            if line.contains(':') {
                let line_comp: Tuple<&str> = line.splitn(2, ':').collect();
                headers.insert((*line_comp.left).to_string(), (*line_comp.right).trim().to_string());
            } else {
                continue;
            }
        }
        Some(headers)
    }
}

#[derive(Debug, Clone)]
/// Represents the status of the Response. This includes HTTP Status Code & Reason Phrase as per [RFC-2616](https://www.w3.org/Protocols/rfc2616/rfc2616-sec6.html#sec6.1).
pub struct ResponseStatus(pub StatusCode, pub Option<String>);

impl Display for ResponseStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        if let Some(reason) = self.1.as_ref() {
            write!(f, "{} - {}", &self.0, reason.as_str())
        } else {
            write!(f, "{}", self.0)
        }
    }
}

#[derive(Debug, Copy, Clone)]
/// Represents the HTTP Status Codes.
///
/// Based on the general categories of the Response [RFC-2616](https://www.w3.org/Protocols/rfc2616/rfc2616-sec6.html#sec6.1.1).
/// [Wikipedia](https://en.wikipedia.org/wiki/List_of_HTTP_status_codes) article for the same.
///
/// The `Ignore` and `Failure` are for Internal purposes.
pub enum StatusCode {
    /// Represents status codes in the 1xx range.
    Informational(u16),
    /// Represents status codes in the 2xx range. Generally the successful responses.
    Success(u16),
    /// Represents status codes in the 3xx range.
    Redirection(u16),
    /// Represents status codes in the 4xx range.
    ClientError(u16),
    /// Represents status codes in the 5xx range.
    ServerError(u16),
    Ignore,
    Failure,
}

impl Display for StatusCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        if let Some(code) = self.get_code().as_ref() {
            write!(f, "HTTP Response Code: {}", *code)
        } else {
            write!(f, "HTTP Response Code: ERROR!")
        }
    }
}

impl StatusCode {
    /// Extracts the actual numeric status code (like 200, 404, etc.).
    pub fn get_code(self) -> Option<u16> {
        match self {
            StatusCode::Informational(val) => Some(val),
            StatusCode::ClientError(val) => Some(val),
            StatusCode::ServerError(val) => Some(val),
            StatusCode::Success(val) => Some(val),
            StatusCode::Redirection(val) => Some(val),
            _ => None,
        }
    }

    fn from_code(code: &str) -> Self {
        let code = code.trim();
        if code.len() != 3 {
            return StatusCode::Failure;
        }
        let code_num: u16 = code.parse().unwrap();
        match code_num {
            100..=199 => StatusCode::Informational(code_num),
            200..=299 => StatusCode::Success(code_num),
            300..=399 => StatusCode::Redirection(code_num),
            400..=499 => StatusCode::ClientError(code_num),
            500..=599 => StatusCode::ServerError(code_num),
            _ => StatusCode::Failure,
        }
    }
}