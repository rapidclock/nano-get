extern crate tokio;
extern crate tokio_tls;


use tokio_tls::{TlsStream, TlsConnector};



use tokio::net::TcpStream;

use std::io::{Read, Write};
use crate::{Request, Response};
use crate::errors::NanoGetError;
use super::https::create_ssl_stream;
use std::error::Error;

pub async fn async_get(request: &Request) -> Result<Response, NanoGetError> {
    let mut stream = TcpStream::connect(request.url.get_host_with_port()).await?;
    if request.is_https() {
        let connector = TlsConnector::builder(SslMethod::tls()).unwrap().build();
        let mut ssl_stream = connector.connect(&request.url.host, stream).unwrap();
        execute_https(&mut ssl_stream, request).await
    } else {
        execute(&mut stream, request).await
    }
}

pub async fn execute_https(mut stream: &mut SslStream<TcpStream>, request: &Request) -> Result<Response, NanoGetError> {
    todo!()
}

pub async fn execute(mut stream: &mut TcpStream, request: &Request) -> Result<Response, NanoGetError> {
    send_request(&mut stream, request).await?;
    receive_response(&mut stream).await
}

async fn send_request(mut stream: &mut TcpStream, request: &Request) -> Result<(), Box<dyn Error>> {
    write_method(stream, request).await?;
    write_headers(stream, request).await?;
    if request.body.is_some() {
        write_body(stream, request).await?;
    }
    Ok(())
}

async fn write_method(mut stream: &mut TcpStream, request: &Request) -> Result<(), Box<dyn Error>> {
    todo!()
}

async fn write_headers(mut stream: &mut TcpStream, request: &Request) -> Result<(), Box<dyn Error>> {
    todo!()
}

async fn write_body(mut stream: &mut TcpStream, request: &Request) -> Result<(), Box<dyn Error>> {
    todo!()
}

async fn receive_response(mut stream: &mut TcpStream) -> Result<Response, NanoGetError> {
    todo!()
}