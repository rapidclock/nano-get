//! This module relates to the HTTPS GET using OpenSSL.
extern crate openssl;

use std::net::TcpStream;

use openssl::ssl::{SslConnector, SslMethod, SslStream};

use super::{Request, Response, ToUrl, Url};
use super::errors::NanoGetError;
use super::http;
use crate::errors::ErrorKind;

/// The implementation of HTTPS GET using OpenSSL.
///
/// This is identical in most ways to the regular HTTP version provided in the crate.
/// This function panics if anything breaks in the process.
pub fn get_https<A: ToUrl>(url: A) -> String {
    let request = Request::default_get_request(url).expect("Url couldn't be formed!");
    let response = request_https_get(&request).unwrap();
    response.body
}

fn acquire_ssl_stream(url: &Url) -> Result<SslStream<TcpStream>, NanoGetError> {
    let connector: SslConnector = SslConnector::builder(SslMethod::tls())
        .map_err(|_err| NanoGetError::new(ErrorKind::HttpsSslError))?.build();
    let stream = TcpStream::connect(&url.get_host_with_port()).unwrap();
    connector.connect(&url.host, stream).map_err(|_err| NanoGetError::new(ErrorKind::HttpsSslError))
}

pub fn request_https_get(request: &Request) -> Result<Response, NanoGetError> {
    let mut ssl_stream = acquire_ssl_stream(&request.url)?;
    http::execute(&mut ssl_stream, &request)
}