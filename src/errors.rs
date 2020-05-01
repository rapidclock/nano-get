use std::fmt::Formatter;

#[derive(Debug)]
pub struct NanoGetError {
    kind: ErrorKind,
}

#[derive(Debug)]
pub enum ErrorKind {
    Default,
    ParseError,
    NetworkError,
    HttpMethodError,
    HttpsSslError,
}

impl std::error::Error for NanoGetError {}

impl std::fmt::Display for NanoGetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result<> {
        let kind = &self.kind;
        write!(f, "nano-get Error - {:?}", kind)
    }
}

impl NanoGetError {
    pub fn new(kind: ErrorKind) -> Self {
        NanoGetError { kind }
    }
}