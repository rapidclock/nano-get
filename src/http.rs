use std::io::{BufRead, Read, Write};
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

pub(crate) fn read_response_head<R: BufRead>(
    reader: &mut R,
    strict: bool,
) -> Result<ResponseHead, NanoGetError> {
    loop {
        let head = response::read_response_head(reader, strict)?;
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
    let mut bytes = Vec::with_capacity(estimate_request_capacity(request, target));
    bytes.extend_from_slice(request.method().as_str().as_bytes());
    bytes.extend_from_slice(b" ");
    bytes.extend_from_slice(target.as_bytes());
    bytes.extend_from_slice(b" HTTP/1.1\r\n");

    if !request.has_header("host") {
        append_header_line(&mut bytes, "Host", &request.url().host_header_value());
    }
    if !request.has_header("user-agent") {
        append_header_line(&mut bytes, "User-Agent", "nano-get/0.3.0");
    }
    if !request.has_header("accept") {
        append_header_line(&mut bytes, "Accept", "*/*");
    }
    if !request.has_header("connection") {
        let connection = if connection_close {
            "close"
        } else {
            "keep-alive"
        };
        append_header_line(&mut bytes, "Connection", connection);
    }

    for header in request.headers() {
        append_header_line(&mut bytes, header.name(), header.value());
    }

    bytes.extend_from_slice(b"\r\n");
    writer.write_all(&bytes)?;
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
    let mut bytes = Vec::with_capacity(estimate_connect_capacity(target_authority, headers));
    bytes.extend_from_slice(b"CONNECT ");
    bytes.extend_from_slice(target_authority.as_bytes());
    bytes.extend_from_slice(b" HTTP/1.1\r\n");
    append_header_line(&mut bytes, "Host", target_authority);
    append_header_line(&mut bytes, "Connection", connection_value);

    for header in headers {
        append_header_line(&mut bytes, header.name(), header.value());
    }

    bytes.extend_from_slice(b"\r\n");
    writer.write_all(&bytes)?;
    Ok(())
}

fn append_header_line(buffer: &mut Vec<u8>, name: &str, value: &str) {
    buffer.extend_from_slice(name.as_bytes());
    buffer.extend_from_slice(b": ");
    buffer.extend_from_slice(value.as_bytes());
    buffer.extend_from_slice(b"\r\n");
}

fn estimate_request_capacity(request: &Request, target: &str) -> usize {
    let request_line = request.method().as_str().len() + 1 + target.len() + " HTTP/1.1\r\n".len();
    let custom_headers: usize = request
        .headers()
        .iter()
        .map(|header| header.name().len() + header.value().len() + 4)
        .sum();

    request_line + custom_headers + 160
}

fn estimate_connect_capacity(target_authority: &str, headers: &[Header]) -> usize {
    let request_line = "CONNECT ".len() + target_authority.len() + " HTTP/1.1\r\n".len();
    let custom_headers: usize = headers
        .iter()
        .map(|header| header.name().len() + header.value().len() + 4)
        .sum();

    request_line + custom_headers + 96
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor, Read, Write};

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
        let stream = InMemoryStream::new(bytes);
        let mut reader = BufReader::new(stream);
        let head = super::read_response_head(&mut reader, true).unwrap();
        assert_eq!(head.status_code, 200);
    }

    #[test]
    fn read_response_head_preserves_prefetched_bytes_in_reader_buffer() {
        let bytes = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\nprefetched".to_vec();
        let stream = InMemoryStream::new(bytes);
        let mut reader = BufReader::new(stream);
        let head = super::read_response_head(&mut reader, true).unwrap();
        assert_eq!(head.status_code, 200);
        assert_eq!(reader.buffer(), b"prefetched");
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

    struct PartialThenFailWriter {
        writes: usize,
    }

    impl Write for PartialThenFailWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.writes >= 3 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "forced write failure",
                ))
            } else if buf.len() > 1 {
                Ok(buf.len() / 2)
            } else {
                Ok(buf.len())
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn connect_writer_handles_partial_writes_and_late_errors() {
        let mut writer = PartialThenFailWriter { writes: 0 };
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
