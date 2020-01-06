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
}

impl std::error::Error for NanoGetError {}

impl std::fmt::Display for NanoGetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result<> {
        let kind = &self.kind;
        write!(f, "nano-get Error - {:?}", kind)
    }
}

//impl From for NanoGetError {
//    fn from(err: std::io::Error) -> Self {
//
//    }
//}

impl NanoGetError {
    pub fn new(kind: ErrorKind) -> Self {
        NanoGetError {kind}
    }
}


//pub type Result<T> = std::result::Result<T, NanoGetError>;