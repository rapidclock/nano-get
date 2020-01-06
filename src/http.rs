//! This module provides the main HTTP Get method.
use std::io::{Read, Write};
use std::net::TcpStream;

use crate::url::{ToUrl};
use crate::request::Request;
use crate::response::Response;
use crate::errors::{NanoGetError};

/// The basic implementation of the HTTP GET method.
///
/// This can be called on anything that implements the ToUrl Trait.
///
/// This library provides implementation of the ToUrl trait for String, &str, URL and &URL.
///
/// This function returns the response body as a String.
pub fn get_http<A: ToUrl>(url: A) -> String {
    let request = Request::default_get_request(url).expect("Url couldn't be formed!");
    request_http_get(&request).unwrap().body
}

pub fn request_http_get(request: &Request) -> Result<Response, NanoGetError> {
    let mut stream = TcpStream::connect(request.url.get_host_with_port()).unwrap();
    execute(&mut stream, &request)
}

pub fn execute<S: Read + Write>(mut stream: S, request: &Request) -> Result<Response, NanoGetError> {
    send_request(&mut stream, &request).unwrap();
    receive_response(&mut stream)
}

pub fn send_request(stream: &mut dyn Write, request: &Request) -> std::io::Result<()> {
    write_http_method(stream, request)?;
    write_std_headers(stream, request)?;
    if request.body.is_some() {
        return write_request_body(stream, request);
    }
    Ok(())
}

fn write_http_method(stream: &mut dyn Write, request: &Request) -> std::io::Result<()> {
    stream.write_fmt(format_args!("{method} {path} HTTP/1.1\r\n",
                                  method = request.get_request_type(),
                                  path = request.url.path))?;
    Ok(())
}

fn write_std_headers(stream: &mut dyn Write, request: &Request) -> std::io::Result<()> {
    for (k, v) in request.get_headers() {
        writeln!(stream, "{}: {}\r", k, v)?;
    }
    stream.write_all(b"\r\n")?;
    Ok(())
}

fn write_request_body(stream: &mut dyn Write, request: &Request) -> std::io::Result<()> {
    write!(stream, "{}", request.body.as_ref().unwrap())
}

pub fn receive_response(stream: &mut dyn Read) -> Result<Response, NanoGetError> {
    let response_vec = read_response(stream).unwrap();
    let response_str = String::from_utf8_lossy(&response_vec);
    let response = parse_body_from_response(&response_str);
    Ok(response)
}

fn read_response(stream: &mut dyn Read) -> std::io::Result<Vec<u8>> {
    let mut lines: Vec<u8> = Vec::with_capacity(2048);
    stream.read_to_end(&mut lines)?;
    Ok(lines)
}

fn parse_body_from_response(response: &str) -> Response {
    Response::new_from_net_response(response.to_string())
}
