#![cfg(feature = "https")]

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::ssl::{SslAcceptor, SslMethod};
use openssl::x509::extension::{BasicConstraints, KeyUsage, SubjectAlternativeName};
use openssl::x509::{X509NameBuilder, X509};

struct HttpsTestServer {
    base_url: String,
    cert_path: PathBuf,
    request_lines: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl HttpsTestServer {
    fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
        let _ = fs::remove_file(&self.cert_path);
    }

    fn port(&self) -> u16 {
        self.base_url.rsplit(':').next().unwrap().parse().unwrap()
    }
}

#[test]
fn get_https_works_when_the_certificate_is_trusted() {
    let server = spawn_https_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()
    ]);

    let result =
        with_ssl_cert_file(Some(&server.cert_path), || nano_get::get(&server.base_url)).unwrap();
    let request_lines = server.request_lines.lock().unwrap().clone();

    assert_eq!(result, "hello");
    assert_eq!(request_lines, vec!["GET / HTTP/1.1".to_string()]);
    server.join();
}

#[test]
fn head_https_returns_metadata() {
    let server = spawn_https_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nX-Mode: tls\r\n\r\nok".to_vec(),
    ]);

    let response = with_ssl_cert_file(Some(&server.cert_path), || {
        nano_get::head_https(&server.base_url)
    })
    .unwrap();
    let request_lines = server.request_lines.lock().unwrap().clone();

    assert_eq!(response.header("x-mode"), Some("tls"));
    assert!(response.body.is_empty());
    assert_eq!(request_lines, vec!["HEAD / HTTP/1.1".to_string()]);
    server.join();
}

#[test]
fn untrusted_https_certificate_returns_tls_error() {
    let server = spawn_https_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()
    ]);

    let error = with_ssl_cert_file(None, || nano_get::get(&server.base_url)).unwrap_err();
    assert!(matches!(error, nano_get::NanoGetError::Tls(_)));
    server.join();
}

#[test]
fn https_requests_can_tunnel_through_http_proxies() {
    let server = spawn_https_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()
    ]);
    let proxy = spawn_connect_proxy(server.port());
    let client = nano_get::Client::builder()
        .proxy(nano_get::ProxyConfig::new(&proxy.base_url).unwrap())
        .build();

    let response = with_ssl_cert_file(Some(&server.cert_path), || {
        client.execute(nano_get::Request::get(&server.base_url).unwrap())
    })
    .unwrap();

    assert_eq!(response.body_text().unwrap(), "hello");
    assert_eq!(
        proxy.request_lines.lock().unwrap().clone(),
        vec![format!("CONNECT 127.0.0.1:{} HTTP/1.1", server.port())]
    );
    proxy.join();
    server.join();
}

#[test]
fn connect_proxy_auth_retries_before_starting_tls() {
    let server = spawn_https_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()
    ]);
    let proxy = spawn_authenticating_connect_proxy(server.port());
    let client = nano_get::Client::builder()
        .proxy(nano_get::ProxyConfig::new(&proxy.base_url).unwrap())
        .basic_proxy_auth("proxy", "secret")
        .build();

    let response = with_ssl_cert_file(Some(&server.cert_path), || {
        client.execute(nano_get::Request::get(&server.base_url).unwrap())
    })
    .unwrap();

    assert_eq!(response.body_text().unwrap(), "hello");
    let proxy_requests = proxy.requests.lock().unwrap().clone();
    assert_eq!(proxy_requests.len(), 2);
    assert!(!proxy_requests[0].contains("Proxy-Authorization:"));
    assert!(proxy_requests[1].contains("Proxy-Authorization: Basic cHJveHk6c2VjcmV0\r\n"));
    let origin_requests = server.requests.lock().unwrap().clone();
    assert_eq!(origin_requests.len(), 1);
    assert!(!origin_requests[0].contains("Proxy-Authorization:"));
    proxy.join();
    server.join();
}

#[test]
fn origin_authorization_is_not_sent_on_connect_requests() {
    let server = spawn_https_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()
    ]);
    let proxy = spawn_connect_proxy(server.port());
    let client = nano_get::Client::builder()
        .proxy(nano_get::ProxyConfig::new(&proxy.base_url).unwrap())
        .preemptive_basic_auth("user", "pass")
        .build();

    let response = with_ssl_cert_file(Some(&server.cert_path), || {
        client.execute(nano_get::Request::get(&server.base_url).unwrap())
    })
    .unwrap();

    assert_eq!(response.body_text().unwrap(), "ok");
    let proxy_requests = proxy.requests.lock().unwrap().clone();
    assert_eq!(proxy_requests.len(), 1);
    assert!(!proxy_requests[0].contains("Authorization:"));
    let origin_requests = server.requests.lock().unwrap().clone();
    assert_eq!(origin_requests.len(), 1);
    assert!(origin_requests[0].contains("Authorization: Basic dXNlcjpwYXNz\r\n"));
    proxy.join();
    server.join();
}

#[test]
fn failed_connect_tunnels_return_proxy_errors() {
    let proxy = spawn_rejecting_proxy();
    let client = nano_get::Client::builder()
        .proxy(nano_get::ProxyConfig::new(&proxy.base_url).unwrap())
        .build();

    let error = client
        .execute(nano_get::Request::get("https://127.0.0.1:44443").unwrap())
        .unwrap_err();

    assert!(matches!(
        error,
        nano_get::NanoGetError::ProxyConnectFailed(407, _)
    ));
    proxy.join();
}

fn spawn_https_server(responses: Vec<Vec<u8>>) -> HttpsTestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_lines_for_thread = Arc::clone(&request_lines);
    let requests_for_thread = Arc::clone(&requests);
    let (private_key, certificate, certificate_pem) = generate_certificate();
    let cert_path = write_certificate_file(&certificate_pem);

    let handle = thread::spawn(move || {
        let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        acceptor.set_private_key(&private_key).unwrap();
        acceptor.set_certificate(&certificate).unwrap();
        acceptor.check_private_key().unwrap();
        let acceptor = acceptor.build();

        for response in responses {
            let (stream, _) = listener.accept().unwrap();
            let Ok(mut stream) = acceptor.accept(stream) else {
                continue;
            };

            let request = read_request(&mut stream);
            let request_line = request
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches('\r')
                .to_string();
            request_lines_for_thread.lock().unwrap().push(request_line);
            requests_for_thread.lock().unwrap().push(request);
            stream.write_all(&response).unwrap();
        }
    });

    HttpsTestServer {
        base_url: format!("https://127.0.0.1:{}", address.port()),
        cert_path,
        request_lines,
        requests,
        handle: Some(handle),
    }
}

fn generate_certificate() -> (PKey<Private>, X509, Vec<u8>) {
    let rsa = Rsa::generate(2048).unwrap();
    let key = PKey::from_rsa(rsa).unwrap();

    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "127.0.0.1").unwrap();
    let name = name.build();

    let mut builder = X509::builder().unwrap();
    builder.set_version(2).unwrap();
    builder.set_subject_name(&name).unwrap();
    builder.set_issuer_name(&name).unwrap();
    builder.set_pubkey(&key).unwrap();
    builder
        .set_not_before(Asn1Time::days_from_now(0).unwrap().as_ref())
        .unwrap();
    builder
        .set_not_after(Asn1Time::days_from_now(30).unwrap().as_ref())
        .unwrap();

    let basic_constraints = BasicConstraints::new().critical().ca().build().unwrap();
    builder.append_extension(basic_constraints).unwrap();

    let key_usage = KeyUsage::new()
        .digital_signature()
        .key_encipherment()
        .key_cert_sign()
        .build()
        .unwrap();
    builder.append_extension(key_usage).unwrap();

    let subject_alt_name = SubjectAlternativeName::new()
        .ip("127.0.0.1")
        .dns("localhost")
        .build(&builder.x509v3_context(None, None))
        .unwrap();
    builder.append_extension(subject_alt_name).unwrap();

    builder.sign(&key, MessageDigest::sha256()).unwrap();
    let certificate = builder.build();
    let certificate_pem = certificate.to_pem().unwrap();
    (key, certificate, certificate_pem)
}

fn write_certificate_file(certificate_pem: &[u8]) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = env::temp_dir().join(format!("nano-get-test-{timestamp}.pem"));
    fs::write(&path, certificate_pem).unwrap();
    path
}

fn read_request(stream: &mut impl Read) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];

    loop {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }

    String::from_utf8_lossy(&buffer).into_owned()
}

struct ProxyTestServer {
    base_url: String,
    request_lines: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl ProxyTestServer {
    fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn spawn_connect_proxy(target_port: u16) -> ProxyTestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_lines_for_thread = Arc::clone(&request_lines);
    let requests_for_thread = Arc::clone(&requests);

    let handle = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let request = read_request(&mut client);
        let request_line = request
            .lines()
            .next()
            .unwrap_or_default()
            .trim_end_matches('\r')
            .to_string();
        request_lines_for_thread.lock().unwrap().push(request_line);
        requests_for_thread.lock().unwrap().push(request);
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\nContent-Length: 0\r\n\r\n")
            .unwrap();

        let backend = TcpStream::connect(("127.0.0.1", target_port)).unwrap();
        tunnel_streams(client, backend);
    });

    ProxyTestServer {
        base_url: format!("http://127.0.0.1:{}", address.port()),
        request_lines,
        requests,
        handle: Some(handle),
    }
}

fn spawn_rejecting_proxy() -> ProxyTestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_lines_for_thread = Arc::clone(&request_lines);
    let requests_for_thread = Arc::clone(&requests);

    let handle = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let request = read_request(&mut client);
        let request_line = request
            .lines()
            .next()
            .unwrap_or_default()
            .trim_end_matches('\r')
            .to_string();
        request_lines_for_thread.lock().unwrap().push(request_line);
        requests_for_thread.lock().unwrap().push(request);
        client
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
    });

    ProxyTestServer {
        base_url: format!("http://127.0.0.1:{}", address.port()),
        request_lines,
        requests,
        handle: Some(handle),
    }
}

fn spawn_authenticating_connect_proxy(target_port: u16) -> ProxyTestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_lines_for_thread = Arc::clone(&request_lines);
    let requests_for_thread = Arc::clone(&requests);

    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let (mut client, _) = listener.accept().unwrap();
            let request = read_request(&mut client);
            let request_line = request
                .lines()
                .next()
                .unwrap_or_default()
                .trim_end_matches('\r')
                .to_string();
            request_lines_for_thread.lock().unwrap().push(request_line);
            requests_for_thread.lock().unwrap().push(request.clone());

            if request.contains("Proxy-Authorization: Basic cHJveHk6c2VjcmV0\r\n") {
                client
                    .write_all(b"HTTP/1.1 200 Connection Established\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
                let backend = TcpStream::connect(("127.0.0.1", target_port)).unwrap();
                tunnel_streams(client, backend);
                return;
            }

            client
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n",
                )
                .unwrap();
        }
    });

    ProxyTestServer {
        base_url: format!("http://127.0.0.1:{}", address.port()),
        request_lines,
        requests,
        handle: Some(handle),
    }
}

fn tunnel_streams(mut client: TcpStream, mut backend: TcpStream) {
    let mut client_reader = client.try_clone().unwrap();
    let mut backend_writer = backend.try_clone().unwrap();
    let forward = thread::spawn(move || {
        let _ = std::io::copy(&mut client_reader, &mut backend_writer);
        let _ = backend_writer.shutdown(Shutdown::Write);
    });

    let _ = std::io::copy(&mut backend, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    forward.join().unwrap();
}

fn with_ssl_cert_file<T>(cert_path: Option<&Path>, operation: impl FnOnce() -> T) -> T {
    static SSL_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = SSL_ENV_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap();
    let previous = env::var_os("SSL_CERT_FILE");

    match cert_path {
        Some(path) => env::set_var("SSL_CERT_FILE", path),
        None => env::remove_var("SSL_CERT_FILE"),
    }

    let result = operation();

    if let Some(previous) = previous {
        env::set_var("SSL_CERT_FILE", previous);
    } else {
        env::remove_var("SSL_CERT_FILE");
    }

    result
}
