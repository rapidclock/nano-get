//! This module provides the main HTTP Get method.
use std::io::{Read, Result, Write};
use std::net::TcpStream;

use crate::url::{ToUrl, URL};

/// The basic implementation of the HTTP GET method.
///
/// This can be called on anything that implements the ToUrl Trait.
/// This library provides implementation of the ToUrl trait for String, &str and URL itself.
/// This function returns the response body as a String.
pub fn get<A: ToUrl>(url: A) -> String {
    let url = url.to_url().unwrap();
    let mut stream = TcpStream::connect(url.get_host_with_port()).unwrap();
    write_http_method(&mut stream, &url).unwrap();
    write_std_headers(&mut stream, &url).unwrap();
    let response = read_response(&mut stream).unwrap();
    let response_str = String::from_utf8(response).unwrap();
    let response_body = parse_body_from_response(&response_str);
    response_body
}

fn write_http_method(stream: &mut TcpStream, url: &URL) -> Result<()> {
    stream.write_fmt(format_args!("GET {} HTTP/1.1\r\n", &url.path))?;
    Ok(())
}

fn write_std_headers(stream: &mut TcpStream, url: &URL) -> Result<()> {
    stream.write(b"user-agent: mini-get/0.1.0\r\n")?;
    stream.write(b"accept: */*\r\n")?;
    stream.write_fmt(format_args!("host: {}\r\n", &url.host))?;
    stream.write(b"connection: close\r\n")?;
    stream.write(b"\r\n")?;
    Ok(())
}

fn read_response(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut lines: Vec<u8> = Vec::with_capacity(2048);
    stream.read_to_end(&mut lines)?;
    Ok(lines)
}

fn parse_body_from_response(response: &str) -> String {
    let lines: Vec<&str> = response.splitn(2,"\r\n\r\n").collect();
    lines.last().unwrap().to_string()
}