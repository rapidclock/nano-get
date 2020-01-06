use std::collections::HashMap;
use std::fmt::{Display, Formatter, Error};


/// This is the HTTP Reponse Object.
///
/// Represents the Response from executing a HTTP `Request`.
///
pub struct Response {
    /// The status of the Response.
    pub status: ResponseStatus,
    /// The body of the Response.
    pub body: String,
    _headers: Option<HashMap<String, String>>,
}

pub fn new_response_from_complete(response: String) -> Response {
    let lines: Vec<&str> = response.splitn(2, "\r\n\r\n").collect();
    let heads = (*lines.first().unwrap()).to_string();
    let head_lines: Vec<&str> = heads.split("\r\n").collect();
    let (resp_state, _) = process_head_lines(head_lines);
    let body = (*lines.last().unwrap()).to_string();
    Response {
        status: resp_state,
        body,
        _headers: None,
    }
}

fn process_head_lines(lines: Vec<&str>) -> (ResponseStatus, Option<HashMap<String, String>>) {
    let head = *lines.get(0).unwrap();
    let parts: Vec<&str> = head.split(' ').collect();
    let status_code = StatusCode::from_code(parts.get(1).unwrap());
    let reason = parts.get(2).map(|v| (*v).to_string());
    (ResponseStatus(status_code, reason), None)
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