//! This module relates to the HTTPS GET using OpenSSL.
extern crate openssl;

use std::net::TcpStream;

use openssl::ssl::{SslConnector, SslMethod};

use crate::{Request, Response, ToUrl, Url};
use crate::http;

use self::openssl::ssl::SslStream;
use crate::errors::NanoGetError;

/// The implementation of HTTPS GET using OpenSSL.
///
/// This is identical in most ways to the regular HTTP version provided in the crate.
/// This function panics if anything breaks in the process.
pub fn get_https<A: ToUrl>(url: A) -> String {
    let request = Request::default_get_request(url).expect("Url couldn't be formed!");
    let response = request_https_get(&request).unwrap();
    response.body
}

fn acquire_ssl_stream(url: &Url) -> SslStream<TcpStream> {
    let connector = SslConnector::builder(SslMethod::tls()).unwrap().build();
    let stream = TcpStream::connect(&url.get_host_with_port()).unwrap();
    connector.connect(&url.host, stream).unwrap()
}

pub fn request_https_get(request: &Request) -> Result<Response, NanoGetError> {
    let mut ssl_stream = acquire_ssl_stream(&request.url);
    http::execute(&mut ssl_stream, &request)
}