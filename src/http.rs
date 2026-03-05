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
    strict: bool,
) -> Result<ResponseHead, NanoGetError> {
    let mut reader = BufReader::new(stream);

    loop {
        let head = response::read_response_head(&mut reader, strict)?;
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
    let connection_value = if connection_close {
        "close"
    } else {
        "keep-alive"
    };
    write!(writer, "CONNECT {target_authority} HTTP/1.1\r\n")?;
    write!(writer, "Host: {target_authority}\r\n")?;
    write!(writer, "Connection: {connection_value}\r\n")?;

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
    use std::io::{Cursor, Read, Write};

    use super::{write_connect_request, write_request};
    use crate::request::{Method, Request};

    struct InMemoryStream {
        reader: Cursor<Vec<u8>>,
        writes: Vec<u8>,
    }

    impl InMemoryStream {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                reader: Cursor::new(bytes),
                writes: Vec::new(),
            }
        }
    }

    impl Read for InMemoryStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reader.read(buf)
        }
    }

    impl Write for InMemoryStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

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

    #[test]
    fn read_response_head_skips_interim_responses() {
        let bytes =
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec();
        let mut stream = InMemoryStream::new(bytes);
        let head = super::read_response_head(&mut stream, true).unwrap();
        assert_eq!(head.status_code, 200);
    }

    #[test]
    fn serializes_connect_requests_with_close() {
        let mut bytes = Vec::new();
        write_connect_request(&mut bytes, "example.com:443", &[], true).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("Connection: close\r\n"));
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "forced failure",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "forced failure",
            ))
        }
    }

    #[test]
    fn write_helpers_propagate_io_errors() {
        let request = Request::get("http://example.com").unwrap();

        let mut writer = FailingWriter;
        let error = write_request(&mut writer, &request, "/", true).unwrap_err();
        assert!(matches!(error, crate::errors::NanoGetError::Io(_)));

        let mut writer = FailingWriter;
        let error = write_connect_request(&mut writer, "example.com:443", &[], false).unwrap_err();
        assert!(matches!(error, crate::errors::NanoGetError::Io(_)));
        assert!(writer.flush().is_err());
    }

    struct FailOnThirdWrite {
        writes: usize,
    }

    impl Write for FailOnThirdWrite {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.writes >= 3 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "forced third write failure",
                ))
            } else {
                Ok(buf.len())
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn connect_writer_errors_can_happen_after_initial_lines() {
        let mut writer = FailOnThirdWrite { writes: 0 };
        let error = write_connect_request(&mut writer, "example.com:443", &[], false).unwrap_err();
        assert!(matches!(error, crate::errors::NanoGetError::Io(_)));
        writer.flush().unwrap();
    }

    #[test]
    fn in_memory_stream_write_impl_is_exercised() {
        let request = Request::get("http://example.com/path").unwrap();
        let mut stream = InMemoryStream::new(Vec::new());
        write_request(&mut stream, &request, "/path", false).unwrap();
        stream.flush().unwrap();
        assert!(!stream.writes.is_empty());
    }
}
