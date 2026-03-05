use std::io::{Read, Write};
use std::net::TcpStream;

use openssl::ssl::HandshakeError;
use openssl::ssl::{SslConnector, SslMethod};

use crate::errors::NanoGetError;
use crate::http::BoxStream;
use crate::url::Url;

pub(crate) fn connect_tls(url: &Url) -> Result<BoxStream, NanoGetError> {
    let stream = TcpStream::connect(url.connect_host_with_port()).map_err(NanoGetError::Connect)?;
    connect_tls_over_stream(url, stream)
}

pub(crate) fn connect_tls_over_stream<S>(url: &Url, stream: S) -> Result<BoxStream, NanoGetError>
where
    S: Read + Write + Send + 'static,
{
    let mut builder = SslConnector::builder(SslMethod::tls())
        .map_err(|error| NanoGetError::Tls(error.to_string()))?;
    builder
        .set_default_verify_paths()
        .map_err(|error| NanoGetError::Tls(error.to_string()))?;

    let connector = builder.build();
    let stream = connector
        .connect(&url.host, stream)
        .map_err(handshake_error)?;
    Ok(Box::new(stream))
}

fn handshake_error<S>(error: HandshakeError<S>) -> NanoGetError {
    NanoGetError::Tls(match error {
        HandshakeError::SetupFailure(error) => error.to_string(),
        HandshakeError::Failure(error) => error.error().to_string(),
        HandshakeError::WouldBlock(_) => "TLS handshake would block".to_string(),
    })
}
