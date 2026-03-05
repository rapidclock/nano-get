use std::io::{BufReader, Read, Write};
use std::net::TcpStream;

use crate::errors::NanoGetError;
use crate::request::{Header, Request};
use crate::response::{self, ResponseHead};

pub(crate) trait HttpStream: Read + Write + Send {}

impl<T: Read + Write + Send> HttpStream for T {}

pub(crate) type BoxStream = Box<dyn HttpStream>;

pub(crate) fn connect_tcp(address: &str) -> Result<TcpStream, NanoGetError> {
    TcpStream::connect(address).map_err(NanoGetError::Connect)
}

pub(crate) fn read_response_head<S: Read + Write + ?Sized>(
    stream: &mut S,
) -> Result<ResponseHead, NanoGetError> {
    let mut reader = BufReader::new(stream);

    loop {
        let head = response::read_response_head(&mut reader)?;
        if (100..=199).contains(&head.status_code) && head.status_code != 101 {
            continue;
        }

        return Ok(head);
    }
}

pub(crate) fn write_request<W: Write + ?Sized>(
    writer: &mut W,
    request: &Request,
    target: &str,
    connection_close: bool,
) -> Result<(), NanoGetError> {
    write!(
        writer,
        "{} {} HTTP/1.1\r\n",
        request.method().as_str(),
        target
    )?;

    for default_header in request.default_headers_for(connection_close) {
        if !request.has_header(default_header.name()) {
            write_header(writer, &default_header)?;
        }
    }

    for header in request.headers() {
        write_header(writer, header)?;
    }

    writer.write_all(b"\r\n")?;
    Ok(())
}

pub(crate) fn write_connect_request<W: Write + ?Sized>(
    writer: &mut W,
    target_authority: &str,
    headers: &[Header],
    connection_close: bool,
) -> Result<(), NanoGetError> {
    write!(writer, "CONNECT {target_authority} HTTP/1.1\r\n")?;
    write!(writer, "Host: {target_authority}\r\n")?;
    write!(
        writer,
        "Connection: {}\r\n",
        if connection_close {
            "close"
        } else {
            "keep-alive"
        }
    )?;

    for header in headers {
        write_header(writer, header)?;
    }

    writer.write_all(b"\r\n")?;
    Ok(())
}

fn write_header<W: Write + ?Sized>(writer: &mut W, header: &Header) -> Result<(), NanoGetError> {
    write!(writer, "{}: {}\r\n", header.name(), header.value())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{write_connect_request, write_request};
    use crate::request::{Method, Request};

    #[test]
    fn serializes_get_requests() {
        let mut request = Request::get("http://example.com/path?x=1").unwrap();
        request.add_header("X-Test", "123").unwrap();

        let mut bytes = Vec::new();
        write_request(&mut bytes, &request, &request.url().origin_form(), true).unwrap();

        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("GET /path?x=1 HTTP/1.1\r\n"));
        assert!(text.contains("Host: example.com\r\n"));
        assert!(text.contains("User-Agent: nano-get/0.3.0\r\n"));
        assert!(text.contains("X-Test: 123\r\n"));
        assert!(text.contains("Connection: close\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn serializes_head_requests() {
        let request = Request::head("http://example.com").unwrap();
        let mut bytes = Vec::new();
        write_request(&mut bytes, &request, &request.url().origin_form(), true).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("HEAD / HTTP/1.1\r\n"));
        assert!(!matches!(request.method(), Method::Get));
    }

    #[test]
    fn managed_headers_cannot_be_overridden() {
        let error = Request::get("http://example.com")
            .unwrap()
            .add_header("Host", "override.test")
            .unwrap_err();
        assert!(matches!(
            error,
            crate::errors::NanoGetError::ProtocolManagedHeader(_)
        ));
    }

    #[test]
    fn serializes_absolute_form_targets() {
        let request = Request::get("http://example.com/path").unwrap();
        let mut bytes = Vec::new();
        write_request(&mut bytes, &request, &request.url().absolute_form(), false).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("GET http://example.com/path HTTP/1.1\r\n"));
        assert!(text.contains("Connection: keep-alive\r\n"));
    }

    #[test]
    fn serializes_connect_requests() {
        let mut bytes = Vec::new();
        write_connect_request(&mut bytes, "example.com:443", &[], false).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("CONNECT example.com:443 HTTP/1.1\r\n"));
        assert!(text.contains("Host: example.com:443\r\n"));
    }
}
