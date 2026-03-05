mod support;

use std::sync::Arc;

use nano_get::{
    AuthDecision, AuthHandler, AuthTarget, Client, Header, ProxyConfig, Request, Response, Url,
};

use support::spawn_handler_http_server;

struct StaticAuthHandler {
    header_name: &'static str,
    header_value: &'static str,
    scheme: &'static str,
}

impl AuthHandler for StaticAuthHandler {
    fn respond(
        &self,
        _target: AuthTarget,
        _url: &Url,
        challenges: &[nano_get::Challenge],
        _request: &Request,
        _response: &Response,
    ) -> Result<AuthDecision, nano_get::NanoGetError> {
        if challenges
            .iter()
            .any(|challenge| challenge.scheme.eq_ignore_ascii_case(self.scheme))
        {
            return Ok(AuthDecision::UseHeaders(vec![Header::new(
                self.header_name,
                self.header_value,
            )?]));
        }

        Ok(AuthDecision::NoMatch)
    }
}

#[test]
fn basic_auth_retries_on_401() {
    let server = spawn_handler_http_server(2, |request| {
        if request.contains("Authorization: Basic dXNlcjpwYXNz\r\n") {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()
        } else {
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"api\"\r\nContent-Length: 0\r\n\r\n".to_vec()
        }
    });

    let client = Client::builder().basic_auth("user", "pass").build();
    let response = client
        .execute(Request::get(format!("{}/auth", server.base_url)).unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "ok");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn generic_auth_handler_retries_on_401() {
    let server = spawn_handler_http_server(2, |request| {
        if request.contains("Authorization: Token secret\r\n") {
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nauthed".to_vec()
        } else {
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Token realm=\"api\"\r\nContent-Length: 0\r\n\r\n".to_vec()
        }
    });

    let handler = Arc::new(StaticAuthHandler {
        header_name: "Authorization",
        header_value: "Token secret",
        scheme: "Token",
    });
    let client = Client::builder().auth_handler(handler).build();
    let response = client
        .execute(Request::get(format!("{}/generic-auth", server.base_url)).unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "authed");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn repeated_401_returns_the_final_response_without_looping() {
    let server = spawn_handler_http_server(2, |_| {
        b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"api\"\r\nContent-Length: 0\r\n\r\n".to_vec()
    });

    let client = Client::builder().basic_auth("user", "pass").build();
    let response = client
        .execute(Request::get(format!("{}/still-401", server.base_url)).unwrap())
        .unwrap();

    assert_eq!(response.status_code, 401);
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn missing_www_authenticate_returns_raw_401() {
    let server = spawn_handler_http_server(1, |_| {
        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_vec()
    });

    let client = Client::builder().basic_auth("user", "pass").build();
    let response = client
        .execute(Request::get(format!("{}/missing-auth", server.base_url)).unwrap())
        .unwrap();

    assert_eq!(response.status_code, 401);
    assert_eq!(server.request_lines.lock().unwrap().len(), 1);
    server.join();
}

#[test]
fn malformed_www_authenticate_returns_an_error() {
    let server = spawn_handler_http_server(1, |_| {
        b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"oops\r\nContent-Length: 0\r\n\r\n".to_vec()
    });

    let client = Client::builder().basic_auth("user", "pass").build();
    let error = client
        .execute(Request::get(format!("{}/bad-auth", server.base_url)).unwrap())
        .unwrap_err();

    assert!(matches!(
        error,
        nano_get::NanoGetError::MalformedChallenge(_)
    ));
    server.join();
}

#[test]
fn manual_authorization_disables_automatic_retry() {
    let server = spawn_handler_http_server(1, |_| {
        b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"api\"\r\nContent-Length: 0\r\n\r\n".to_vec()
    });

    let client = Client::builder().basic_auth("user", "pass").build();
    let mut request = Request::get(format!("{}/manual-auth", server.base_url)).unwrap();
    request.authorization("Basic bWFudWFsOmNyZWRz").unwrap();

    let response = client.execute(request).unwrap();
    assert_eq!(response.status_code, 401);
    assert_eq!(server.request_lines.lock().unwrap().len(), 1);
    server.join();
}

#[test]
fn preemptive_basic_auth_sends_credentials_on_the_first_request() {
    let server = spawn_handler_http_server(1, |request| {
        assert!(request.contains("Authorization: Basic dXNlcjpwYXNz\r\n"));
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nfirst".to_vec()
    });

    let client = Client::builder()
        .preemptive_basic_auth("user", "pass")
        .build();
    let response = client
        .execute(Request::get(format!("{}/preemptive", server.base_url)).unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "first");
    server.join();
}

#[test]
fn same_authority_redirects_preserve_origin_auth() {
    let server = spawn_handler_http_server(2, |request| {
        if request.starts_with("GET /start HTTP/1.1\r\n") {
            assert!(request.contains("Authorization: Basic dXNlcjpwYXNz\r\n"));
            b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n".to_vec()
        } else {
            assert!(request.contains("Authorization: Basic dXNlcjpwYXNz\r\n"));
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nsaved".to_vec()
        }
    });

    let client = Client::builder()
        .redirect_policy(nano_get::RedirectPolicy::follow(5))
        .preemptive_basic_auth("user", "pass")
        .build();
    let response = client
        .execute(Request::get(format!("{}/start", server.base_url)).unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "saved");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn cross_authority_redirects_strip_origin_auth() {
    let destination = spawn_handler_http_server(1, |request| {
        assert!(!request.contains("Authorization:"));
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nopen".to_vec()
    });
    let redirect_location = format!("{}/final", destination.base_url);
    let redirector = spawn_handler_http_server(1, move |request| {
        assert!(request.contains("Authorization: Basic dXNlcjpwYXNz\r\n"));
        format!("HTTP/1.1 302 Found\r\nLocation: {redirect_location}\r\nContent-Length: 0\r\n\r\n")
            .into_bytes()
    });

    let client = Client::builder()
        .redirect_policy(nano_get::RedirectPolicy::follow(5))
        .preemptive_basic_auth("user", "pass")
        .build();
    let response = client
        .execute(Request::get(format!("{}/start", redirector.base_url)).unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "open");
    redirector.join();
    destination.join();
}

#[test]
fn basic_proxy_auth_retries_on_407() {
    let proxy = spawn_handler_http_server(2, |request| {
        if request.contains("Proxy-Authorization: Basic cHJveHk6c2VjcmV0\r\n") {
            assert!(request.starts_with("GET http://example.com/proxy HTTP/1.1\r\n"));
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nproxy".to_vec()
        } else {
            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n".to_vec()
        }
    });

    let client = Client::builder()
        .proxy(ProxyConfig::new(&proxy.base_url).unwrap())
        .basic_proxy_auth("proxy", "secret")
        .build();
    let response = client
        .execute(Request::get("http://example.com/proxy").unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "proxy");
    assert_eq!(proxy.request_lines.lock().unwrap().len(), 2);
    proxy.join();
}

#[test]
fn generic_proxy_auth_handler_retries_on_407() {
    let proxy = spawn_handler_http_server(2, |request| {
        if request.contains("Proxy-Authorization: Token secret\r\n") {
            b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nauthed".to_vec()
        } else {
            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Token realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n".to_vec()
        }
    });

    let handler = Arc::new(StaticAuthHandler {
        header_name: "Proxy-Authorization",
        header_value: "Token secret",
        scheme: "Token",
    });
    let client = Client::builder()
        .proxy(ProxyConfig::new(&proxy.base_url).unwrap())
        .proxy_auth_handler(handler)
        .build();
    let response = client
        .execute(Request::get("http://example.com/proxy").unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "authed");
    assert_eq!(proxy.request_lines.lock().unwrap().len(), 2);
    proxy.join();
}

#[test]
fn repeated_407_returns_the_final_response_without_looping() {
    let proxy = spawn_handler_http_server(2, |_| {
        b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n".to_vec()
    });

    let client = Client::builder()
        .proxy(ProxyConfig::new(&proxy.base_url).unwrap())
        .basic_proxy_auth("proxy", "secret")
        .build();
    let response = client
        .execute(Request::get("http://example.com/proxy").unwrap())
        .unwrap();

    assert_eq!(response.status_code, 407);
    assert_eq!(proxy.request_lines.lock().unwrap().len(), 2);
    proxy.join();
}

#[test]
fn missing_proxy_authenticate_returns_raw_407() {
    let proxy = spawn_handler_http_server(1, |_| {
        b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n".to_vec()
    });

    let client = Client::builder()
        .proxy(ProxyConfig::new(&proxy.base_url).unwrap())
        .basic_proxy_auth("proxy", "secret")
        .build();
    let response = client
        .execute(Request::get("http://example.com/proxy").unwrap())
        .unwrap();

    assert_eq!(response.status_code, 407);
    assert_eq!(proxy.request_lines.lock().unwrap().len(), 1);
    proxy.join();
}

#[test]
fn malformed_proxy_authenticate_returns_an_error() {
    let proxy = spawn_handler_http_server(1, |_| {
        b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"oops\r\nContent-Length: 0\r\n\r\n".to_vec()
    });

    let client = Client::builder()
        .proxy(ProxyConfig::new(&proxy.base_url).unwrap())
        .basic_proxy_auth("proxy", "secret")
        .build();
    let error = client
        .execute(Request::get("http://example.com/proxy").unwrap())
        .unwrap_err();

    assert!(matches!(
        error,
        nano_get::NanoGetError::MalformedChallenge(_)
    ));
    proxy.join();
}

#[test]
fn manual_proxy_authorization_disables_automatic_retry() {
    let proxy = spawn_handler_http_server(1, |_| {
        b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"proxy\"\r\nContent-Length: 0\r\n\r\n".to_vec()
    });

    let client = Client::builder()
        .proxy(ProxyConfig::new(&proxy.base_url).unwrap())
        .basic_proxy_auth("proxy", "secret")
        .build();
    let mut request = Request::get("http://example.com/proxy").unwrap();
    request
        .proxy_authorization("Basic bWFudWFsOnByb3h5")
        .unwrap();

    let response = client.execute(request).unwrap();
    assert_eq!(response.status_code, 407);
    assert_eq!(proxy.request_lines.lock().unwrap().len(), 1);
    proxy.join();
}

#[test]
fn preemptive_basic_proxy_auth_sends_credentials_on_the_first_request() {
    let proxy = spawn_handler_http_server(1, |request| {
        assert!(request.contains("Proxy-Authorization: Basic cHJveHk6c2VjcmV0\r\n"));
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nproxy".to_vec()
    });

    let client = Client::builder()
        .proxy(ProxyConfig::new(&proxy.base_url).unwrap())
        .preemptive_basic_proxy_auth("proxy", "secret")
        .build();
    let response = client
        .execute(Request::get("http://example.com/proxy").unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "proxy");
    proxy.join();
}
