use std::collections::HashMap;
use crate::{ToUrl, Url, execute};
use std::error::Error;
use crate::response::Response;
use crate::errors::{NanoGetError};

pub struct Request {
    pub url: Url,
    request_type: RequestType,
    headers: Option<HashMap<String, String>>,
    pub body: Option<String>,
}

enum RequestType {
    HEAD,
    GET,
    PUT,
    POST,
    DELETE,
    OPTIONS,
}

impl RequestType {
    fn value(&self) -> &'static str{
        match self {
            RequestType::GET => "GET",
            RequestType::HEAD => "HEAD",
            RequestType::POST => "POST",
            RequestType::PUT => "PUT",
            RequestType::DELETE => "DELETE",
            RequestType::OPTIONS => "OPTIONS"
        }
    }
}

pub type Header<'a> = (&'a str, &'a str);

impl Request {
    pub fn new<A: ToUrl>(url: A, headers: Option<Vec<Header>>) -> Result<Self, Box<dyn Error>> {
        let url = url.to_url()?;
        let mut request = Request {
            url,
            request_type: RequestType::GET,
            headers: Some(Self::get_default_headers()),
            body: None
        };
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

    pub fn default_get_request<A: ToUrl> (url: A) -> Result<Self, Box<dyn Error>> {
        let url = url.to_url()?;
        let mut headers = Self::get_default_headers();
        headers.insert("host".to_string(), url.host.clone());
        Ok(Request {
            url,
            request_type : RequestType::GET,
            headers: Some(headers),
            body: None,
        })
    }

    fn get_default_headers() -> HashMap<String, String> {
        let mut headers = HashMap::with_capacity(4);
        headers.insert("user-agent".to_string(), "mini-get/0.1.0".to_string());
        headers.insert("accept".to_string(), "*/*".to_string());
        headers.insert("connection".to_string(), "close".to_string());
        headers
    }

    pub fn execute(&self) -> Result<Response, NanoGetError> {
        execute(&self)
    }

    pub fn get_headers(&self) -> impl Iterator<Item=(&str, &str)> {
        self.headers.as_ref().unwrap().iter().map(|(k, v)| {
            (k.as_str(), v.as_str())
        })
    }

    pub fn get_request_type(&self) -> &str {
        self.request_type.value()
    }

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