//! This module relates to the HTTPS GET using OpenSSL.
extern crate openssl;

use std::net::TcpStream;

use openssl::ssl::{SslConnector, SslMethod};

use crate::ToUrl;
use crate::http;
use crate::request::Request;

/// The implementation of HTTPS GET using OpenSSL.
///
/// This is identical in most ways to the regular HTTP version provided in the crate.
pub fn get_https<A: ToUrl>(url: A) -> String {
    let request = Request::default_get_request(url).unwrap();
    let connector = SslConnector::builder(SslMethod::tls()).unwrap().build();
    let stream = TcpStream::connect(request.url.get_host_with_port()).unwrap();
    let mut stream = connector.connect(&request.url.host, stream).unwrap();

    http::send_request(&mut stream, &request).unwrap();
    let response = http::receive_response(&mut stream).unwrap();
    response.body
}