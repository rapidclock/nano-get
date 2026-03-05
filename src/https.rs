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

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use openssl::error::ErrorStack;
    use openssl::ssl::HandshakeError;

    use super::handshake_error;

    #[derive(Debug)]
    struct WouldBlockStream;

    impl Read for WouldBlockStream {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "would block",
            ))
        }
    }

    impl Write for WouldBlockStream {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "would block",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn handshake_error_maps_setup_and_would_block_variants() {
        let setup_error = HandshakeError::<WouldBlockStream>::SetupFailure(ErrorStack::get());
        let setup = handshake_error(setup_error);
        assert!(matches!(setup, crate::NanoGetError::Tls(_)));

        let mut stream = WouldBlockStream;
        let mut buf = [0u8; 1];
        assert!(stream.read(&mut buf).is_err());
        assert!(stream.write(&buf).is_err());
        stream.flush().unwrap();

        let mut builder =
            openssl::ssl::SslConnector::builder(openssl::ssl::SslMethod::tls()).unwrap();
        builder.set_verify(openssl::ssl::SslVerifyMode::NONE);
        let connector = builder.build();
        let error = connector
            .connect("example.com", WouldBlockStream)
            .unwrap_err();
        assert!(matches!(error, HandshakeError::WouldBlock(_)));
        let mapped = handshake_error(error);
        assert_eq!(mapped.to_string(), "TLS error: TLS handshake would block");
    }
}
