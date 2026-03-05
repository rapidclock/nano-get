use std::fmt::{self, Display, Formatter};
use std::io::{BufRead, BufReader, Read};
use std::str;

use crate::auth::{parse_authenticate_headers, Challenge};
use crate::errors::NanoGetError;
use crate::request::{Header, Method};

/// HTTP protocol version reported by the server response line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpVersion {
    /// `HTTP/1.0`
    Http10,
    /// `HTTP/1.1`
    Http11,
}

impl Display for HttpVersion {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http10 => write!(f, "HTTP/1.0"),
            Self::Http11 => write!(f, "HTTP/1.1"),
        }
    }
}

/// Parsed HTTP response data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// HTTP version parsed from the status line.
    pub version: HttpVersion,
    /// Numeric status code, for example `200` or `404`.
    pub status_code: u16,
    /// Reason phrase from the status line, for example `OK`.
    pub reason_phrase: String,
    /// Response headers in wire order. Duplicate header names are preserved.
    pub headers: Vec<Header>,
    /// Chunked transfer trailers, when present.
    pub trailers: Vec<Header>,
    /// Raw response body bytes.
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseHead {
    pub version: HttpVersion,
    pub status_code: u16,
    pub reason_phrase: String,
    pub headers: Vec<Header>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyKind {
    None,
    ContentLength,
    Chunked,
    CloseDelimited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedResponse {
    pub response: Response,
    pub body_kind: BodyKind,
    pub connection_close: bool,
}

impl Response {
    /// Returns the first header value matching `name`, using ASCII case-insensitive lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.matches_name(name))
            .map(Header::value)
    }

    /// Iterates over all header values matching `name`, preserving wire order and duplicates.
    pub fn headers_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Header> + 'a {
        self.headers
            .iter()
            .filter(move |header| header.matches_name(name))
    }

    /// Returns the first trailer value matching `name`, using ASCII case-insensitive lookup.
    pub fn trailer(&self, name: &str) -> Option<&str> {
        self.trailers
            .iter()
            .find(|header| header.matches_name(name))
            .map(Header::value)
    }

    /// Parses `WWW-Authenticate` challenges from the response.
    pub fn www_authenticate_challenges(&self) -> Result<Vec<Challenge>, NanoGetError> {
        parse_authenticate_headers(&self.headers, "www-authenticate")
    }

    /// Parses `Proxy-Authenticate` challenges from the response.
    pub fn proxy_authenticate_challenges(&self) -> Result<Vec<Challenge>, NanoGetError> {
        parse_authenticate_headers(&self.headers, "proxy-authenticate")
    }

    /// Decodes the body as UTF-8 without taking ownership.
    pub fn body_text(&self) -> Result<&str, NanoGetError> {
        Ok(str::from_utf8(&self.body)?)
    }

    /// Consumes the response and decodes the body as UTF-8.
    pub fn into_body_text(self) -> Result<String, NanoGetError> {
        String::from_utf8(self.body).map_err(|error| NanoGetError::InvalidUtf8(error.utf8_error()))
    }

    /// Returns `true` when status is in the `2xx` range.
    pub fn is_success(&self) -> bool {
        (200..=299).contains(&self.status_code)
    }

    /// Returns `true` when status is in the `3xx` range.
    pub fn is_redirection(&self) -> bool {
        (300..=399).contains(&self.status_code)
    }

    /// Returns `true` when status is in the `4xx` range.
    pub fn is_client_error(&self) -> bool {
        (400..=499).contains(&self.status_code)
    }

    /// Returns `true` when status is in the `5xx` range.
    pub fn is_server_error(&self) -> bool {
        (500..=599).contains(&self.status_code)
    }
}

#[cfg(test)]
pub(crate) fn read_response<R: Read>(
    reader: &mut BufReader<R>,
    method: Method,
) -> Result<Response, NanoGetError> {
    Ok(read_parsed_response(reader, method, true)?.response)
}

pub(crate) fn read_parsed_response<R: Read>(
    reader: &mut BufReader<R>,
    method: Method,
    strict: bool,
) -> Result<ParsedResponse, NanoGetError> {
    loop {
        let head = read_response_head(reader, strict)?;

        if (100..=199).contains(&head.status_code) && head.status_code != 101 {
            continue;
        }

        let body_kind = determine_body_kind(&head.headers, method, head.status_code, strict)?;
        let (body, trailers) = match body_kind {
            BodyKind::None => (Vec::new(), Vec::new()),
            BodyKind::Chunked => read_chunked_body(reader, strict)?,
            BodyKind::ContentLength => {
                let content_length = content_length(&head.headers)?.unwrap_or(0);
                read_content_length_body(reader, content_length)?
            }
            BodyKind::CloseDelimited => read_eof_body(reader)?,
        };

        let connection_close = should_close_connection(head.version, &head.headers, body_kind);

        return Ok(ParsedResponse {
            response: Response {
                version: head.version,
                status_code: head.status_code,
                reason_phrase: head.reason_phrase,
                headers: head.headers,
                trailers,
                body,
            },
            body_kind,
            connection_close,
        });
    }
}

pub(crate) fn read_response_head<R: BufRead>(
    reader: &mut R,
    strict: bool,
) -> Result<ResponseHead, NanoGetError> {
    let (status_line, status_line_has_crlf) = read_line(reader).map_err(|error| match error {
        NanoGetError::Io(error) => NanoGetError::MalformedStatusLine(error.to_string()),
        NanoGetError::MalformedHeader(line) => NanoGetError::MalformedStatusLine(line),
        other => other,
    })?;
    if strict && !status_line_has_crlf {
        return Err(NanoGetError::MalformedStatusLine(status_line));
    }
    let (version, status_code, reason_phrase) = parse_status_line(&status_line)?;
    let headers = read_headers(reader, strict)?;
    Ok(ResponseHead {
        version,
        status_code,
        reason_phrase,
        headers,
    })
}

fn read_headers<R: BufRead>(reader: &mut R, strict: bool) -> Result<Vec<Header>, NanoGetError> {
    let mut headers = Vec::new();

    loop {
        let (line, has_crlf) = read_line(reader).map_err(|error| match error {
            NanoGetError::Io(io_error) => NanoGetError::MalformedHeader(io_error.to_string()),
            other => other,
        })?;
        if strict && !has_crlf {
            return Err(NanoGetError::MalformedHeader(line));
        }

        if line.is_empty() {
            return Ok(headers);
        }

        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(NanoGetError::MalformedHeader(line));
        }

        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| NanoGetError::MalformedHeader(line.clone()))?;
        headers.push(Header::new(name.to_string(), value.trim().to_string())?);
    }
}

fn read_line<R: BufRead>(reader: &mut R) -> Result<(String, bool), NanoGetError> {
    let mut line = Vec::new();
    let bytes_read = reader.read_until(b'\n', &mut line)?;
    if bytes_read == 0 {
        return Err(NanoGetError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "unexpected EOF",
        )));
    }

    let mut has_crlf = false;
    if line.ends_with(b"\r\n") {
        has_crlf = true;
        line.truncate(line.len() - 2);
    } else if line.ends_with(b"\n") {
        line.truncate(line.len() - 1);
    }

    let text = String::from_utf8(line)
        .map_err(|error| NanoGetError::MalformedHeader(error.utf8_error().to_string()))?;
    Ok((text, has_crlf))
}

fn parse_status_line(line: &str) -> Result<(HttpVersion, u16, String), NanoGetError> {
    let mut parts = line.splitn(3, ' ');
    let version = match parts.next() {
        Some("HTTP/1.0") => HttpVersion::Http10,
        Some("HTTP/1.1") => HttpVersion::Http11,
        _ => return Err(NanoGetError::MalformedStatusLine(line.to_string())),
    };

    let status_code_token = parts
        .next()
        .ok_or_else(|| NanoGetError::MalformedStatusLine(line.to_string()))?;
    if status_code_token.len() != 3 || !status_code_token.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(NanoGetError::MalformedStatusLine(line.to_string()));
    }

    let status_code = status_code_token
        .parse::<u16>()
        .map_err(|_| NanoGetError::MalformedStatusLine(line.to_string()))?;

    let reason_phrase = parts.next().unwrap_or("").to_string();
    Ok((version, status_code, reason_phrase))
}

fn determine_body_kind(
    headers: &[Header],
    method: Method,
    status_code: u16,
    strict: bool,
) -> Result<BodyKind, NanoGetError> {
    if response_has_no_body(method, status_code) {
        return Ok(BodyKind::None);
    }

    let has_content_length = content_length(headers)?.is_some();
    if let Some(transfer_encoding) = transfer_encoding(headers)? {
        if strict && has_content_length {
            return Err(NanoGetError::AmbiguousResponseFraming(
                "response contains both Transfer-Encoding and Content-Length".to_string(),
            ));
        }
        if transfer_encoding.eq_ignore_ascii_case("chunked") {
            return Ok(BodyKind::Chunked);
        }

        return Err(NanoGetError::UnsupportedTransferEncoding(transfer_encoding));
    }

    if has_content_length {
        return Ok(BodyKind::ContentLength);
    }

    Ok(BodyKind::CloseDelimited)
}

fn response_has_no_body(method: Method, status_code: u16) -> bool {
    method == Method::Head
        || (100..=199).contains(&status_code)
        || status_code == 204
        || status_code == 304
}

fn transfer_encoding(headers: &[Header]) -> Result<Option<String>, NanoGetError> {
    let values: Vec<&str> = headers
        .iter()
        .filter(|header| header.matches_name("transfer-encoding"))
        .map(Header::value)
        .collect();

    if values.is_empty() {
        return Ok(None);
    }

    let tokens: Vec<String> = values
        .iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();

    if tokens.len() == 1 {
        return Ok(Some(tokens[0].clone()));
    }

    Err(NanoGetError::UnsupportedTransferEncoding(tokens.join(",")))
}

pub(crate) fn content_length(headers: &[Header]) -> Result<Option<usize>, NanoGetError> {
    let mut values = headers
        .iter()
        .filter(|header| header.matches_name("content-length"))
        .flat_map(|header| header.value().split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_content_length_value);

    let Some(first) = values.next() else {
        return Ok(None);
    };
    let first = first?;

    for value in values {
        if value? != first {
            return Err(NanoGetError::InvalidContentLength(
                "mismatched duplicate content-length headers".to_string(),
            ));
        }
    }

    Ok(Some(first))
}

fn parse_content_length_value(value: &str) -> Result<usize, NanoGetError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NanoGetError::InvalidContentLength(value.to_string()));
    }

    value
        .parse::<usize>()
        .map_err(|_| NanoGetError::InvalidContentLength(value.to_string()))
}

fn read_content_length_body<R: Read>(
    reader: &mut R,
    content_length: usize,
) -> Result<(Vec<u8>, Vec<Header>), NanoGetError> {
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            NanoGetError::IncompleteMessage(
                "unexpected EOF while reading Content-Length body".to_string(),
            )
        } else {
            NanoGetError::Io(error)
        }
    })?;
    Ok((body, Vec::new()))
}

fn read_eof_body<R: Read>(reader: &mut R) -> Result<(Vec<u8>, Vec<Header>), NanoGetError> {
    let mut body = Vec::new();
    reader.read_to_end(&mut body)?;
    Ok((body, Vec::new()))
}

fn read_chunked_body<R: BufRead>(
    reader: &mut R,
    strict: bool,
) -> Result<(Vec<u8>, Vec<Header>), NanoGetError> {
    let mut body = Vec::new();

    loop {
        let (line, has_crlf) = read_line(reader).map_err(|error| match error {
            NanoGetError::Io(io_error) if io_error.kind() == std::io::ErrorKind::UnexpectedEof => {
                NanoGetError::IncompleteMessage(
                    "unexpected EOF while reading chunk size".to_string(),
                )
            }
            NanoGetError::Io(io_error) => NanoGetError::InvalidChunk(io_error.to_string()),
            other => other,
        })?;
        if strict && !has_crlf {
            return Err(NanoGetError::InvalidChunk(
                "chunk-size line is not CRLF-terminated".to_string(),
            ));
        }
        let size_token = line.split(';').next().unwrap_or("").trim();
        let chunk_size = usize::from_str_radix(size_token, 16)
            .map_err(|_| NanoGetError::InvalidChunk(line.clone()))?;

        if chunk_size == 0 {
            let trailers = read_headers(reader, strict)?;
            return Ok((body, trailers));
        }

        let start = body.len();
        body.resize(start + chunk_size, 0);
        reader.read_exact(&mut body[start..]).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                NanoGetError::IncompleteMessage(
                    "unexpected EOF while reading chunk body".to_string(),
                )
            } else {
                NanoGetError::Io(error)
            }
        })?;

        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                NanoGetError::IncompleteMessage("unexpected EOF after chunk body".to_string())
            } else {
                NanoGetError::Io(error)
            }
        })?;
        if crlf != *b"\r\n" {
            return Err(NanoGetError::InvalidChunk(
                "missing CRLF after chunk body".to_string(),
            ));
        }
    }
}

pub(crate) fn should_close_connection(
    version: HttpVersion,
    headers: &[Header],
    body_kind: BodyKind,
) -> bool {
    if body_kind == BodyKind::CloseDelimited {
        return true;
    }

    if has_connection_token(headers, "close") {
        return true;
    }

    version == HttpVersion::Http10 && !has_connection_token(headers, "keep-alive")
}

fn has_connection_token(headers: &[Header], token: &str) -> bool {
    headers
        .iter()
        .filter(|header| header.matches_name("connection"))
        .flat_map(|header| header.value().split(','))
        .map(str::trim)
        .any(|candidate| candidate.eq_ignore_ascii_case(token))
}

#[cfg(test)]
pub(crate) fn parse_response_bytes(bytes: &[u8], method: Method) -> Result<Response, NanoGetError> {
    let mut reader = BufReader::new(bytes);
    read_response(&mut reader, method)
}

#[cfg(test)]
mod tests {
    use std::io::{self, BufRead, BufReader, Cursor, Read};

    use super::{
        parse_response_bytes, read_chunked_body, read_content_length_body, read_parsed_response,
        read_response_head, BodyKind, HttpVersion,
    };
    use crate::errors::NanoGetError;
    use crate::request::Method;

    #[test]
    fn parses_content_length_response() {
        let response = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Test: 1\r\n\r\nhello",
            Method::Get,
        )
        .unwrap();
        assert_eq!(response.version, HttpVersion::Http11);
        assert_eq!(response.status_code, 200);
        assert_eq!(response.reason_phrase, "OK");
        assert_eq!(response.header("x-test"), Some("1"));
        assert_eq!(response.body, b"hello");
    }

    #[test]
    fn parses_reason_phrases_with_spaces() {
        let response = parse_response_bytes(
            b"HTTP/1.0 404 Not Found Here\r\nContent-Length: 0\r\n\r\n",
            Method::Get,
        )
        .unwrap();
        assert_eq!(response.version, HttpVersion::Http10);
        assert_eq!(response.reason_phrase, "Not Found Here");
    }

    #[test]
    fn parses_chunked_responses_and_trailers() {
        let response = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nrust\r\n6\r\nacean!\r\n0\r\nX-Trailer: done\r\n\r\n",
            Method::Get,
        )
        .unwrap();
        assert_eq!(response.body, b"rustacean!");
        assert_eq!(response.trailer("x-trailer"), Some("done"));
    }

    #[test]
    fn head_responses_ignore_declared_body() {
        let response = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
            Method::Head,
        )
        .unwrap();
        assert!(response.body.is_empty());
    }

    #[test]
    fn parses_connection_close_bodies() {
        let mut reader = BufReader::new(&b"HTTP/1.1 200 OK\r\n\r\neof body"[..]);
        let parsed = read_parsed_response(&mut reader, Method::Get, true).unwrap();
        assert_eq!(parsed.body_kind, BodyKind::CloseDelimited);
        assert!(parsed.connection_close);
        assert_eq!(parsed.response.body, b"eof body");
    }

    #[test]
    fn rejects_invalid_status_lines() {
        let error = parse_response_bytes(b"HTP/1.1 200 OK\r\n\r\n", Method::Get).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedStatusLine(_)));

        let error = parse_response_bytes(b"HTTP/1.1 20 OK\r\n\r\n", Method::Get).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedStatusLine(_)));

        let error = parse_response_bytes(b"HTTP/1.1 2000 OK\r\n\r\n", Method::Get).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedStatusLine(_)));
    }

    #[test]
    fn rejects_unsupported_transfer_encodings() {
        let error = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n",
            Method::Get,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            NanoGetError::UnsupportedTransferEncoding(_)
        ));

        let error = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n",
            Method::Get,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            NanoGetError::UnsupportedTransferEncoding(_)
        ));
    }

    #[test]
    fn rejects_mismatched_duplicate_content_lengths() {
        let error = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello!",
            Method::Get,
        )
        .unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidContentLength(_)));
    }

    #[test]
    fn accepts_duplicate_content_lengths_with_equal_numeric_values() {
        let response = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nContent-Length: 05\r\nContent-Length: 5\r\n\r\nhello",
            Method::Get,
        )
        .unwrap();
        assert_eq!(response.body, b"hello");
    }

    #[test]
    fn rejects_non_numeric_content_lengths() {
        let error = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nContent-Length: +5\r\n\r\nhello",
            Method::Get,
        )
        .unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidContentLength(_)));
    }

    #[test]
    fn rejects_invalid_chunk_sizes() {
        let error = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nbogus\r\n",
            Method::Get,
        )
        .unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidChunk(_)));
    }

    #[test]
    fn body_text_reports_invalid_utf8() {
        let response = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n\xff\xff",
            Method::Get,
        )
        .unwrap();
        assert!(matches!(
            response.body_text(),
            Err(NanoGetError::InvalidUtf8(_))
        ));
    }

    #[test]
    fn skips_interim_responses_but_not_switching_protocols() {
        let response = parse_response_bytes(
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            Method::Get,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(response.body, b"ok");

        let response =
            parse_response_bytes(b"HTTP/1.1 101 Switching Protocols\r\n\r\n", Method::Get).unwrap();
        assert_eq!(response.status_code, 101);
    }

    #[test]
    fn rejects_malformed_headers() {
        let error = parse_response_bytes(b"HTTP/1.1 200 OK\r\nBroken-Header\r\n\r\n", Method::Get)
            .unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedHeader(_)));
    }

    #[test]
    fn preserves_duplicate_headers() {
        let response = parse_response_bytes(
            b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Length: 0\r\n\r\n",
            Method::Get,
        )
        .unwrap();
        let cookies: Vec<_> = response
            .headers_named("set-cookie")
            .map(|header| header.value().to_string())
            .collect();
        assert_eq!(cookies, vec!["a=1".to_string(), "b=2".to_string()]);
    }

    #[test]
    fn parses_response_heads() {
        let mut reader = BufReader::new(&b"HTTP/1.1 200 OK\r\nX-Test: yes\r\n\r\n"[..]);
        let head = read_response_head(&mut reader, true).unwrap();
        assert_eq!(head.status_code, 200);
        assert_eq!(head.headers[0].value(), "yes");
    }

    #[test]
    fn keep_alive_is_honored_for_http_10() {
        let mut reader = BufReader::new(
            &b"HTTP/1.0 200 OK\r\nConnection: keep-alive\r\nContent-Length: 2\r\n\r\nok"[..],
        );
        let parsed = read_parsed_response(&mut reader, Method::Get, true).unwrap();
        assert!(!parsed.connection_close);
    }

    #[test]
    fn strict_mode_rejects_lf_only_lines() {
        let mut reader = BufReader::new(&b"HTTP/1.1 200 OK\nContent-Length: 0\n\n"[..]);
        let error = read_parsed_response(&mut reader, Method::Get, true).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedStatusLine(_)));
    }

    #[test]
    fn lenient_mode_accepts_lf_only_lines() {
        let mut reader = BufReader::new(&b"HTTP/1.1 200 OK\nContent-Length: 2\n\nok"[..]);
        let parsed = read_parsed_response(&mut reader, Method::Get, false).unwrap();
        assert_eq!(parsed.response.body, b"ok");
    }

    #[test]
    fn strict_mode_rejects_transfer_encoding_with_content_length() {
        let mut reader = BufReader::new(
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n2\r\nok\r\n0\r\n\r\n"[..],
        );
        let error = read_parsed_response(&mut reader, Method::Get, true).unwrap_err();
        assert!(matches!(error, NanoGetError::AmbiguousResponseFraming(_)));
    }

    #[test]
    fn incomplete_content_length_body_reports_incomplete_message() {
        let mut reader = BufReader::new(&b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nok"[..]);
        let error = read_parsed_response(&mut reader, Method::Get, true).unwrap_err();
        assert!(matches!(error, NanoGetError::IncompleteMessage(_)));
    }

    #[test]
    fn incomplete_chunked_body_reports_incomplete_message() {
        let mut reader =
            BufReader::new(&b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nok"[..]);
        let error = read_parsed_response(&mut reader, Method::Get, true).unwrap_err();
        assert!(matches!(error, NanoGetError::IncompleteMessage(_)));
    }

    #[test]
    fn response_status_helpers_cover_all_classes() {
        let ok = parse_response_bytes(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", Method::Get)
            .unwrap();
        assert!(ok.is_success());
        assert!(!ok.is_redirection());

        let redirect = parse_response_bytes(
            b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n",
            Method::Get,
        )
        .unwrap();
        assert!(redirect.is_redirection());

        let client_error = parse_response_bytes(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
            Method::Get,
        )
        .unwrap();
        assert!(client_error.is_client_error());

        let server_error = parse_response_bytes(
            b"HTTP/1.1 500 Boom\r\nContent-Length: 0\r\n\r\n",
            Method::Get,
        )
        .unwrap();
        assert!(server_error.is_server_error());
    }

    #[test]
    fn parses_authenticate_challenges_from_response_helpers() {
        let response = parse_response_bytes(
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"api\"\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n",
            Method::Get,
        )
        .unwrap();
        assert_eq!(response.www_authenticate_challenges().unwrap().len(), 1);
        assert_eq!(response.proxy_authenticate_challenges().unwrap().len(), 1);
    }

    #[test]
    fn http_version_display_formats_wire_tokens() {
        assert_eq!(HttpVersion::Http10.to_string(), "HTTP/1.0");
        assert_eq!(HttpVersion::Http11.to_string(), "HTTP/1.1");
    }

    #[test]
    fn response_head_maps_status_and_header_read_io_errors() {
        let mut empty = BufReader::new(&b""[..]);
        let error = read_response_head(&mut empty, true).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedStatusLine(_)));

        let mut truncated = BufReader::new(&b"HTTP/1.1 200 OK\r\nX-Test: 1\r\n"[..]);
        let error = read_response_head(&mut truncated, true).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedHeader(_)));

        let mut invalid_status = BufReader::new(&b"\xff\r\n\r\n"[..]);
        let error = read_response_head(&mut invalid_status, true).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedStatusLine(_)));

        let mut invalid_header = BufReader::new(&b"HTTP/1.1 200 OK\r\nX:\xff\r\n\r\n"[..]);
        let error = read_response_head(&mut invalid_header, true).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedHeader(_)));
    }

    #[test]
    fn strict_header_parsing_rejects_lf_only_and_obs_fold() {
        let mut lf_only = BufReader::new(&b"HTTP/1.1 200 OK\r\nX-Test: 1\n\n"[..]);
        let error = read_response_head(&mut lf_only, true).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedHeader(_)));

        let mut obs_fold = BufReader::new(&b"HTTP/1.1 200 OK\r\n value\r\n\r\n"[..]);
        let error = read_response_head(&mut obs_fold, true).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedHeader(_)));
    }

    struct FailingRead {
        cursor: Cursor<Vec<u8>>,
        fail_at: usize,
        kind: io::ErrorKind,
    }

    impl Read for FailingRead {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let pos = self.cursor.position() as usize;
            if pos >= self.fail_at {
                return Err(io::Error::new(self.kind, "forced read failure"));
            }
            let max = (self.fail_at - pos).min(buf.len());
            self.cursor.read(&mut buf[..max])
        }
    }

    struct AlwaysErrBufRead;

    impl Read for AlwaysErrBufRead {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::Other, "forced read failure"))
        }
    }

    impl BufRead for AlwaysErrBufRead {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Err(io::Error::new(io::ErrorKind::Other, "forced fill failure"))
        }

        fn consume(&mut self, _amt: usize) {}
    }

    #[test]
    fn body_readers_map_non_eof_io_failures() {
        let mut failing = FailingRead {
            cursor: Cursor::new(vec![1, 2, 3]),
            fail_at: 0,
            kind: io::ErrorKind::Other,
        };
        let error = read_content_length_body(&mut failing, 1).unwrap_err();
        assert!(matches!(error, NanoGetError::Io(_)));

        let mut failing_chunk = AlwaysErrBufRead;
        let error = read_chunked_body(&mut failing_chunk, true).unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidChunk(_)));

        let mut read_buf = [0u8; 1];
        let mut always = AlwaysErrBufRead;
        assert!(always.read(&mut read_buf).is_err());
        always.consume(0);
    }

    #[test]
    fn chunked_parser_covers_additional_error_paths() {
        let mut empty = BufReader::new(&b""[..]);
        let error = read_chunked_body(&mut empty, true).unwrap_err();
        assert!(matches!(error, NanoGetError::IncompleteMessage(_)));

        let mut lf_only = BufReader::new(&b"2\nok\r\n0\r\n\r\n"[..]);
        let error = read_chunked_body(&mut lf_only, true).unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidChunk(_)));

        let mut body_fail = BufReader::new(FailingRead {
            cursor: Cursor::new(b"2\r\nok\r\n0\r\n\r\n".to_vec()),
            fail_at: 3,
            kind: io::ErrorKind::Other,
        });
        let error = read_chunked_body(&mut body_fail, true).unwrap_err();
        assert!(matches!(error, NanoGetError::Io(_)));

        let mut crlf_eof = BufReader::new(&b"2\r\nok"[..]);
        let error = read_chunked_body(&mut crlf_eof, true).unwrap_err();
        assert!(matches!(error, NanoGetError::IncompleteMessage(_)));

        let mut crlf_fail = BufReader::new(FailingRead {
            cursor: Cursor::new(b"2\r\nok\r\n0\r\n\r\n".to_vec()),
            fail_at: 5,
            kind: io::ErrorKind::Other,
        });
        let error = read_chunked_body(&mut crlf_fail, true).unwrap_err();
        assert!(matches!(error, NanoGetError::Io(_)));

        let mut bad_crlf = BufReader::new(&b"2\r\nokxx"[..]);
        let error = read_chunked_body(&mut bad_crlf, true).unwrap_err();
        assert!(matches!(error, NanoGetError::InvalidChunk(_)));

        let mut invalid_utf8_chunk = BufReader::new(&b"\xff\n"[..]);
        let error = read_chunked_body(&mut invalid_utf8_chunk, false).unwrap_err();
        assert!(matches!(error, NanoGetError::MalformedHeader(_)));
    }
}
