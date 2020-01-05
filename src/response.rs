use std::collections::HashMap;
use std::fmt::{Display, Formatter, Error};

pub struct Response {
    pub status: ResponseStatus,
    _headers: Option<HashMap<String, String>>,
    pub body: String,
}


impl Response {
    pub fn new_from_net_response(response: String) -> Self {
        let lines: Vec<&str> = response.splitn(2, "\r\n\r\n").collect();
        let heads = (*lines.first().unwrap()).to_string();
        let head_lines: Vec<&str> = heads.split("\r\n").collect();
        let (resp_state, _) = process_head_lines(head_lines);
        let body = (*lines.last().unwrap()).to_string();
        Response {
            status: resp_state,
            _headers: None,
            body,
        }
    }

    pub fn new(body: String) -> Self {
        Response {
            status: ResponseStatus(StatusCode::Ignore, None),
            _headers: None,
            body,
        }
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
pub struct ResponseStatus(StatusCode, Option<String>);

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
pub enum StatusCode {
    Informational(u16),
    Success(u16),
    Redirection(u16),
    ClientError(u16),
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

    pub fn from_code(code: &str) -> Self {
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