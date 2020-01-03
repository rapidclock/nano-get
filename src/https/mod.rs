//! This module relates to the HTTPS GET using OpenSSL.
extern crate openssl;

use std::net::TcpStream;

use openssl::ssl::{SslConnector, SslMethod};

use crate::ToUrl;
use crate::http;

/// The implementation of HTTPS GET using OpenSSL.
///
/// This is identical in most ways to the regular HTTP version provided in the crate.
pub fn get_https<A: ToUrl>(url: A) -> String {
    let url = url.to_url().unwrap();
    let connector = SslConnector::builder(SslMethod::tls()).unwrap().build();
    let stream = TcpStream::connect(url.get_host_with_port()).unwrap();
    let mut stream = connector.connect(&url.host, stream).unwrap();

    http::send_request(&mut stream, &url).unwrap();
    http::receive_response(&mut stream)
}