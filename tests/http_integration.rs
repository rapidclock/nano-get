mod support;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use nano_get::{
    get, get_bytes, head, CacheMode, Client, ConnectionPolicy, ParserStrictness, ProxyConfig,
    RedirectPolicy, Request,
};

use support::{spawn_http_server, spawn_persistent_http_server};

#[test]
fn get_returns_text_body() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()
    ]);
    let result = get(format!("{}/", server.base_url)).unwrap();
    assert_eq!(result, "hello");
    server.join();
}

#[test]
fn get_bytes_returns_binary_body() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n\xff\x00\x7f".to_vec(),
    ]);
    let result = get_bytes(format!("{}/bytes", server.base_url)).unwrap();
    assert_eq!(result, vec![0xff, 0x00, 0x7f]);
    server.join();
}

#[test]
fn client_helper_methods_get_bytes_head_and_execute_ref_work() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nX-Head: yes\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\ntwo".to_vec(),
    ]);
    let client = Client::builder().build();
    let bytes = client
        .get_bytes(format!("{}/bytes", server.base_url))
        .unwrap();
    let head_response = client.head(format!("{}/head", server.base_url)).unwrap();
    let request = Request::get(format!("{}/execute-ref", server.base_url)).unwrap();
    let response = client.execute_ref(&request).unwrap();

    assert_eq!(bytes, b"one");
    assert_eq!(head_response.header("x-head"), Some("yes"));
    assert_eq!(response.body_text().unwrap(), "two");
    server.join();
}

#[test]
fn head_returns_metadata_and_empty_body() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Test: yes\r\n\r\nhello".to_vec(),
    ]);
    let response = head(format!("{}/head", server.base_url)).unwrap();
    let request_lines = server.request_lines.lock().unwrap().clone();
    assert_eq!(response.status_code, 200);
    assert_eq!(response.header("x-test"), Some("yes"));
    assert!(response.body.is_empty());
    assert_eq!(request_lines, vec!["HEAD /head HTTP/1.1".to_string()]);
    server.join();
}

#[test]
fn helpers_follow_redirects() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndone".to_vec(),
    ]);
    let response = get(format!("{}/start", server.base_url)).unwrap();
    let request_lines = server.request_lines.lock().unwrap().clone();
    assert_eq!(response, "done");
    assert_eq!(
        request_lines,
        vec![
            "GET /start HTTP/1.1".to_string(),
            "GET /final HTTP/1.1".to_string()
        ]
    );
    server.join();
}

#[test]
fn request_does_not_follow_redirects_by_default() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: /elsewhere\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
    ]);
    let response = Request::get(format!("{}/start", server.base_url))
        .unwrap()
        .execute()
        .unwrap();
    assert_eq!(response.status_code, 301);
    assert_eq!(response.header("location"), Some("/elsewhere"));
    server.join();
}

#[test]
fn request_can_follow_redirects_when_configured() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /followed\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec(),
    ]);
    let response = Request::get(format!("{}/redirect", server.base_url))
        .unwrap()
        .with_redirect_policy(RedirectPolicy::follow(5))
        .execute()
        .unwrap();
    assert_eq!(response.body_text().unwrap(), "hello");
    server.join();
}

#[test]
fn redirect_limit_is_enforced() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 302 Found\r\nLocation: /two\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"HTTP/1.1 302 Found\r\nLocation: /three\r\nContent-Length: 0\r\n\r\n".to_vec(),
    ]);
    let error = Request::get(format!("{}/one", server.base_url))
        .unwrap()
        .with_redirect_policy(RedirectPolicy::follow(1))
        .execute()
        .unwrap_err();
    assert!(matches!(
        error,
        nano_get::NanoGetError::RedirectLimitExceeded(1)
    ));
    server.join();
}

#[test]
fn eof_delimited_bodies_are_supported() {
    let server = spawn_http_server(vec![b"HTTP/1.1 200 OK\r\n\r\neof body".to_vec()]);
    let response = get(format!("{}/eof", server.base_url)).unwrap();
    assert_eq!(response, "eof body");
    server.join();
}

#[test]
fn duplicate_headers_are_preserved() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Length: 2\r\n\r\nok"
            .to_vec(),
    ]);
    let response = Request::get(format!("{}/cookies", server.base_url))
        .unwrap()
        .execute()
        .unwrap();
    let cookies: Vec<_> = response
        .headers_named("set-cookie")
        .map(|header| header.value().to_string())
        .collect();
    assert_eq!(cookies, vec!["a=1".to_string(), "b=2".to_string()]);
    server.join();
}

#[test]
fn get_reports_invalid_utf8() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n\xff\xff".to_vec(),
    ]);
    let error = get(format!("{}/utf8", server.base_url)).unwrap_err();
    assert!(matches!(error, nano_get::NanoGetError::InvalidUtf8(_)));
    server.join();
}

#[test]
fn session_reuses_keep_alive_connections() {
    let server = spawn_persistent_http_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\none".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\ntwo".to_vec(),
    ]);

    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();

    let first = session
        .execute(Request::get(format!("{}/first", server.base_url)).unwrap())
        .unwrap();
    let second = session
        .execute(Request::get(format!("{}/second", server.base_url)).unwrap())
        .unwrap();

    assert_eq!(first.body_text().unwrap(), "one");
    assert_eq!(second.body_text().unwrap(), "two");
    assert_eq!(*server.connection_count.lock().unwrap(), 1);
    assert_eq!(
        server.request_lines.lock().unwrap().clone(),
        vec![
            "GET /first HTTP/1.1".to_string(),
            "GET /second HTTP/1.1".to_string()
        ]
    );
    server.join();
}

#[test]
fn session_retries_when_reused_connection_is_stale_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connection_count = Arc::new(AtomicUsize::new(0));
    let connection_count_for_thread = Arc::clone(&connection_count);

    let handle = thread::spawn(move || {
        let (mut first_stream, _) = listener.accept().unwrap();
        connection_count_for_thread.fetch_add(1, Ordering::SeqCst);

        let mut request = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            let read = first_stream.read(&mut chunk).unwrap();
            assert!(read > 0, "client closed before first request completed");
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        first_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nfirst")
            .unwrap();
        drop(first_stream);

        let (mut second_stream, _) = listener.accept().unwrap();
        connection_count_for_thread.fetch_add(1, Ordering::SeqCst);
        request.clear();
        loop {
            let read = second_stream.read(&mut chunk).unwrap();
            assert!(read > 0, "client closed before retried request completed");
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        second_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecond")
            .unwrap();
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();

    let first = session
        .execute(Request::get(format!("{}/one", base_url)).unwrap())
        .unwrap();
    let second = session
        .execute(Request::get(format!("{}/two", base_url)).unwrap())
        .unwrap();

    assert_eq!(first.body_text().unwrap(), "first");
    assert_eq!(second.body_text().unwrap(), "second");
    assert_eq!(connection_count.load(Ordering::SeqCst), 2);
    handle.join().unwrap();
}

#[test]
fn head_responses_with_illegal_body_bytes_do_not_poison_reused_connections() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connection_count = Arc::new(AtomicUsize::new(0));
    let connection_count_for_thread = Arc::clone(&connection_count);

    let handle = thread::spawn(move || {
        let (mut first_stream, _) = listener.accept().unwrap();
        connection_count_for_thread.fetch_add(1, Ordering::SeqCst);

        let mut request = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            let read = first_stream.read(&mut chunk).unwrap();
            assert!(read > 0, "client closed before HEAD request completed");
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        first_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
            .unwrap();
        drop(first_stream);

        let (mut second_stream, _) = listener.accept().unwrap();
        connection_count_for_thread.fetch_add(1, Ordering::SeqCst);
        request.clear();
        loop {
            let read = second_stream.read(&mut chunk).unwrap();
            assert!(
                read > 0,
                "client closed before follow-up GET request completed"
            );
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        second_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .unwrap();
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();

    let head_response = session
        .execute(Request::head(format!("{}/head", base_url)).unwrap())
        .unwrap();
    let get_response = session
        .execute(Request::get(format!("{}/get", base_url)).unwrap())
        .unwrap();

    assert!(head_response.body.is_empty());
    assert_eq!(get_response.body_text().unwrap(), "ok");
    assert_eq!(connection_count.load(Ordering::SeqCst), 2);
    handle.join().unwrap();
}

#[test]
fn session_supports_pipelined_get_requests() {
    let server = spawn_persistent_http_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nalpha".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbeta".to_vec(),
    ]);

    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();
    let responses = session
        .execute_pipelined(&[
            Request::get(format!("{}/one", server.base_url)).unwrap(),
            Request::get(format!("{}/two", server.base_url)).unwrap(),
        ])
        .unwrap();

    assert_eq!(responses[0].body_text().unwrap(), "alpha");
    assert_eq!(responses[1].body_text().unwrap(), "beta");
    assert_eq!(*server.connection_count.lock().unwrap(), 1);
    server.join();
}

#[test]
fn memory_cache_serves_fresh_get_responses() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\n\r\ncache".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let request = Request::get(format!("{}/cached", server.base_url)).unwrap();

    let first = client.execute_ref(&request).unwrap();
    let second = client.execute_ref(&request).unwrap();

    assert_eq!(first.body_text().unwrap(), "cache");
    assert_eq!(second.body_text().unwrap(), "cache");
    assert_eq!(server.request_lines.lock().unwrap().len(), 1);
    server.join();
}

#[test]
fn no_store_requests_do_not_populate_the_cache() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 3\r\n\r\none".to_vec(),
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 3\r\n\r\ntwo".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let mut request = Request::get(format!("{}/no-store", server.base_url)).unwrap();
    request.add_header("Cache-Control", "no-store").unwrap();

    let first = client.execute_ref(&request).unwrap();
    let second = client.execute_ref(&request).unwrap();

    assert_eq!(first.body_text().unwrap(), "one");
    assert_eq!(second.body_text().unwrap(), "two");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn vary_headers_create_distinct_cache_variants() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nVary: Accept\r\nContent-Length: 4\r\n\r\ntext".to_vec(),
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nVary: Accept\r\nContent-Length: 4\r\n\r\njson".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let mut text_request = Request::get(format!("{}/vary", server.base_url)).unwrap();
    text_request.add_header("Accept", "text/plain").unwrap();
    let mut json_request = Request::get(format!("{}/vary", server.base_url)).unwrap();
    json_request
        .add_header("Accept", "application/json")
        .unwrap();

    let text = client.execute_ref(&text_request).unwrap();
    let json = client.execute_ref(&json_request).unwrap();
    let text_again = client.execute_ref(&text_request).unwrap();

    assert_eq!(text.body_text().unwrap(), "text");
    assert_eq!(json.body_text().unwrap(), "json");
    assert_eq!(text_again.body_text().unwrap(), "text");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn stale_cache_entries_revalidate_with_etags() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=0\r\nETag: \"v1\"\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
        b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nContent-Length: 0\r\n\r\n".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let request = Request::get(format!("{}/etag", server.base_url)).unwrap();

    let first = client.execute_ref(&request).unwrap();
    let second = client.execute_ref(&request).unwrap();
    let requests = server.requests.lock().unwrap().clone();

    assert_eq!(first.body_text().unwrap(), "hello");
    assert_eq!(second.body_text().unwrap(), "hello");
    assert!(requests[1].contains("If-None-Match: \"v1\"\r\n"));
    server.join();
}

#[test]
fn stale_cache_entries_revalidate_with_last_modified() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=0\r\nLast-Modified: Sun, 06 Nov 1994 08:49:37 GMT\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
        b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nContent-Length: 0\r\n\r\n".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let request = Request::get(format!("{}/last-modified", server.base_url)).unwrap();

    let first = client.execute_ref(&request).unwrap();
    let second = client.execute_ref(&request).unwrap();
    let requests = server.requests.lock().unwrap().clone();

    assert_eq!(first.body_text().unwrap(), "hello");
    assert_eq!(second.body_text().unwrap(), "hello");
    assert!(requests[1].contains("If-Modified-Since: Sun, 06 Nov 1994 08:49:37 GMT\r\n"));
    server.join();
}

#[test]
fn request_no_cache_forces_revalidation_of_fresh_entries() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nETag: \"fresh\"\r\nContent-Length: 2\r\n\r\nok".to_vec(),
        b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nContent-Length: 0\r\n\r\n".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let request = Request::get(format!("{}/request-no-cache", server.base_url)).unwrap();
    let mut revalidate = Request::get(format!("{}/request-no-cache", server.base_url)).unwrap();
    revalidate.add_header("Cache-Control", "no-cache").unwrap();

    let first = client.execute_ref(&request).unwrap();
    let second = client.execute_ref(&revalidate).unwrap();
    let requests = server.requests.lock().unwrap().clone();

    assert_eq!(first.body_text().unwrap(), "ok");
    assert_eq!(second.body_text().unwrap(), "ok");
    assert!(requests[1].contains("If-None-Match: \"fresh\"\r\n"));
    server.join();
}

#[test]
fn only_if_cached_returns_504_when_the_cache_cannot_satisfy_the_request() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: no-cache\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let initial = Request::get(format!("{}/only-if-cached", server.base_url)).unwrap();
    let mut cached_only = Request::get(format!("{}/only-if-cached", server.base_url)).unwrap();
    cached_only
        .add_header("Cache-Control", "only-if-cached")
        .unwrap();

    let first = client.execute_ref(&initial).unwrap();
    let second = client.execute_ref(&cached_only).unwrap();

    assert_eq!(first.body_text().unwrap(), "hello");
    assert_eq!(second.status_code, 504);
    assert_eq!(server.request_lines.lock().unwrap().len(), 1);
    server.join();
}

#[test]
fn request_max_age_zero_forces_revalidation() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nETag: \"age\"\r\nContent-Length: 4\r\n\r\nbody".to_vec(),
        b"HTTP/1.1 304 Not Modified\r\nCache-Control: max-age=60\r\nContent-Length: 0\r\n\r\n".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let request = Request::get(format!("{}/max-age", server.base_url)).unwrap();
    let mut forced = Request::get(format!("{}/max-age", server.base_url)).unwrap();
    forced.add_header("Cache-Control", "max-age=0").unwrap();

    client.execute_ref(&request).unwrap();
    client.execute_ref(&forced).unwrap();

    let requests = server.requests.lock().unwrap().clone();
    assert!(requests[1].contains("If-None-Match: \"age\"\r\n"));
    server.join();
}

#[test]
fn max_stale_allows_serving_stale_cached_responses() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=0\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let initial = Request::get(format!("{}/max-stale", server.base_url)).unwrap();
    let mut stale_ok = Request::get(format!("{}/max-stale", server.base_url)).unwrap();
    stale_ok.add_header("Cache-Control", "max-stale").unwrap();

    let first = client.execute_ref(&initial).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = client.execute_ref(&stale_ok).unwrap();

    assert_eq!(first.body_text().unwrap(), "hello");
    assert_eq!(second.body_text().unwrap(), "hello");
    assert_eq!(server.request_lines.lock().unwrap().len(), 1);
    server.join();
}

#[test]
fn must_revalidate_blocks_max_stale_cache_reuse() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=0, must-revalidate\r\nContent-Length: 3\r\n\r\nold"
            .to_vec(),
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 3\r\n\r\nnew".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let initial = Request::get(format!("{}/must-revalidate", server.base_url)).unwrap();
    let mut stale_ok = Request::get(format!("{}/must-revalidate", server.base_url)).unwrap();
    stale_ok.add_header("Cache-Control", "max-stale").unwrap();

    let first = client.execute_ref(&initial).unwrap();
    let second = client.execute_ref(&stale_ok).unwrap();

    assert_eq!(first.body_text().unwrap(), "old");
    assert_eq!(second.body_text().unwrap(), "new");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn age_header_can_make_cached_entries_unsatisfiable_immediately() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nAge: 120\r\nContent-Length: 5\r\n\r\nstale"
            .to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let initial = Request::get(format!("{}/aged", server.base_url)).unwrap();
    let mut cached_only = Request::get(format!("{}/aged", server.base_url)).unwrap();
    cached_only
        .add_header("Cache-Control", "only-if-cached")
        .unwrap();

    let first = client.execute_ref(&initial).unwrap();
    let second = client.execute_ref(&cached_only).unwrap();

    assert_eq!(first.body_text().unwrap(), "stale");
    assert_eq!(second.status_code, 504);
    assert_eq!(server.request_lines.lock().unwrap().len(), 1);
    server.join();
}

#[test]
fn authorization_requests_are_not_cached_without_explicit_cacheability() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 3\r\n\r\none".to_vec(),
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 3\r\n\r\ntwo".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let mut authed = Request::get(format!("{}/auth-cache", server.base_url)).unwrap();
    authed.authorization("Bearer secret").unwrap();
    let plain = Request::get(format!("{}/auth-cache", server.base_url)).unwrap();

    let first = client.execute_ref(&authed).unwrap();
    let second = client.execute_ref(&plain).unwrap();

    assert_eq!(first.body_text().unwrap(), "one");
    assert_eq!(second.body_text().unwrap(), "two");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn private_authenticated_responses_are_cached_for_the_same_auth_context() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: private, max-age=60\r\nContent-Length: 6\r\n\r\nsecret"
            .to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let mut request = Request::get(format!("{}/private-auth-cache", server.base_url)).unwrap();
    request.authorization("Bearer secret").unwrap();

    let first = client.execute_ref(&request).unwrap();
    let second = client.execute_ref(&request).unwrap();

    assert_eq!(first.body_text().unwrap(), "secret");
    assert_eq!(second.body_text().unwrap(), "secret");
    assert_eq!(server.request_lines.lock().unwrap().len(), 1);
    server.join();
}

#[test]
fn head_updates_cached_get_metadata() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nX-Version: one\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nX-Version: two\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let url = format!("{}/head-cache", server.base_url);

    let initial = client.execute(Request::get(&url).unwrap()).unwrap();
    let head_response = client.execute(Request::head(&url).unwrap()).unwrap();
    let refreshed = client.execute(Request::get(&url).unwrap()).unwrap();

    assert_eq!(initial.header("x-version"), Some("one"));
    assert_eq!(head_response.header("x-version"), Some("two"));
    assert_eq!(refreshed.header("x-version"), Some("two"));
    assert_eq!(refreshed.body_text().unwrap(), "hello");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn head_without_cached_get_does_not_seed_empty_get_cache_entries() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\nX-Head: yes\r\n\r\n"
            .to_vec(),
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let url = format!("{}/head-seed", server.base_url);

    let head_response = client.execute(Request::head(&url).unwrap()).unwrap();
    let get_response = client.execute(Request::get(&url).unwrap()).unwrap();

    assert_eq!(head_response.header("x-head"), Some("yes"));
    assert!(head_response.body.is_empty());
    assert_eq!(get_response.body_text().unwrap(), "hello");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn http_proxy_requests_use_absolute_form_targets() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nproxy".to_vec()
    ]);

    let proxy = ProxyConfig::new(&server.base_url).unwrap();
    let client = Client::builder().proxy(proxy).build();
    let response = client
        .execute(Request::get("http://example.com/path?via=proxy").unwrap())
        .unwrap();

    assert_eq!(response.body_text().unwrap(), "proxy");
    assert_eq!(
        server.request_lines.lock().unwrap().clone(),
        vec!["GET http://example.com/path?via=proxy HTTP/1.1".to_string()]
    );
    server.join();
}

#[test]
fn range_requests_send_range_headers_and_parse_partial_content() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 2-5/10\r\n\r\ncdef"
            .to_vec(),
    ]);

    let mut request = Request::get(format!("{}/range", server.base_url)).unwrap();
    request.range_bytes(Some(2), Some(5)).unwrap();
    let response = request.execute().unwrap();
    let requests = server.requests.lock().unwrap().clone();

    assert_eq!(response.status_code, 206);
    assert_eq!(response.header("content-range"), Some("bytes 2-5/10"));
    assert_eq!(response.body_text().unwrap(), "cdef");
    assert!(requests[0].contains("Range: bytes=2-5\r\n"));
    server.join();
}

#[test]
fn strict_parser_rejects_lf_only_responses() {
    let server = support::spawn_handler_http_server(1, |_| {
        b"HTTP/1.1 200 OK\nContent-Length: 2\n\nok".to_vec()
    });
    let error = Client::builder()
        .build()
        .execute(Request::get(format!("{}/strict", server.base_url)).unwrap())
        .unwrap_err();
    assert!(matches!(
        error,
        nano_get::NanoGetError::MalformedStatusLine(_)
    ));
    server.join();
}

#[test]
fn lenient_parser_accepts_lf_only_responses() {
    let server = support::spawn_handler_http_server(1, |_| {
        b"HTTP/1.1 200 OK\nContent-Length: 2\n\nok".to_vec()
    });
    let response = Client::builder()
        .parser_strictness(ParserStrictness::Lenient)
        .build()
        .execute(Request::get(format!("{}/lenient", server.base_url)).unwrap())
        .unwrap();
    assert_eq!(response.body_text().unwrap(), "ok");
    server.join();
}

#[test]
fn if_range_requires_range_header() {
    let mut request = Request::get("http://example.com").unwrap();
    request.if_range("\"v1\"").unwrap();
    let error = Client::default().execute(request).unwrap_err();
    assert!(matches!(
        error,
        nano_get::NanoGetError::InvalidConditionalRequest(_)
    ));
}

#[test]
fn if_range_rejects_weak_etags() {
    let mut request = Request::get("http://example.com").unwrap();
    request.range_bytes(Some(0), Some(1)).unwrap();
    request.if_range("W/\"v1\"").unwrap();
    let error = Client::default().execute(request).unwrap_err();
    assert!(matches!(
        error,
        nano_get::NanoGetError::InvalidConditionalRequest(_)
    ));
}

#[test]
fn partial_responses_are_combined_into_a_cacheable_full_representation() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 206 Partial Content\r\nCache-Control: max-age=60\r\nETag: \"v1\"\r\nContent-Range: bytes 0-2/6\r\nContent-Length: 3\r\n\r\nabc".to_vec(),
        b"HTTP/1.1 206 Partial Content\r\nCache-Control: max-age=60\r\nETag: \"v1\"\r\nContent-Range: bytes 3-5/6\r\nContent-Length: 3\r\n\r\ndef".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let url = format!("{}/partial-combine", server.base_url);

    let mut first = Request::get(&url).unwrap();
    first.range_bytes(Some(0), Some(2)).unwrap();
    let mut second = Request::get(&url).unwrap();
    second.range_bytes(Some(3), Some(5)).unwrap();

    let first_response = client.execute(first).unwrap();
    let second_response = client.execute(second).unwrap();
    let full_response = client.execute(Request::get(&url).unwrap()).unwrap();

    assert_eq!(first_response.status_code, 206);
    assert_eq!(second_response.status_code, 206);
    assert_eq!(full_response.status_code, 200);
    assert_eq!(full_response.body_text().unwrap(), "abcdef");
    assert_eq!(server.request_lines.lock().unwrap().len(), 2);
    server.join();
}

#[test]
fn if_range_mismatch_with_only_if_cached_returns_504() {
    let server = spawn_http_server(vec![
        b"HTTP/1.1 200 OK\r\nCache-Control: max-age=60\r\nETag: \"v1\"\r\nContent-Length: 6\r\n\r\nabcdef".to_vec(),
    ]);

    let client = Client::builder().cache_mode(CacheMode::Memory).build();
    let url = format!("{}/if-range-only-if-cached", server.base_url);

    client.execute(Request::get(&url).unwrap()).unwrap();

    let mut ranged = Request::get(&url).unwrap();
    ranged.range_bytes(Some(0), Some(1)).unwrap();
    ranged.if_range("\"v2\"").unwrap();
    ranged
        .add_header("Cache-Control", "only-if-cached")
        .unwrap();

    let response = client.execute(ranged).unwrap();
    assert_eq!(response.status_code, 504);
    assert_eq!(server.request_lines.lock().unwrap().len(), 1);
    server.join();
}

#[test]
fn pipelining_retries_unanswered_requests_after_premature_close() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connection_count = Arc::new(AtomicUsize::new(0));
    let connection_count_for_thread = Arc::clone(&connection_count);
    let handle = thread::spawn(move || {
        let (mut first_stream, _) = listener.accept().unwrap();
        connection_count_for_thread.fetch_add(1, Ordering::SeqCst);

        let mut first_request_bytes = Vec::new();
        let mut chunk = [0u8; 512];
        while first_request_bytes
            .windows(4)
            .filter(|window| *window == b"\r\n\r\n")
            .count()
            < 2
        {
            let read = first_stream.read(&mut chunk).unwrap();
            assert!(
                read > 0,
                "client closed connection before both pipelined requests were sent"
            );
            first_request_bytes.extend_from_slice(&chunk[..read]);
        }

        first_stream
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 5\r\n\r\nfirst")
            .unwrap();
        drop(first_stream);

        let (mut second_stream, _) = listener.accept().unwrap();
        connection_count_for_thread.fetch_add(1, Ordering::SeqCst);
        let mut second_request_bytes = Vec::new();
        loop {
            let read = second_stream.read(&mut chunk).unwrap();
            assert!(
                read > 0,
                "client closed retry connection before sending the unanswered request"
            );
            second_request_bytes.extend_from_slice(&chunk[..read]);
            if second_request_bytes
                .windows(4)
                .any(|window| window == b"\r\n\r\n")
            {
                break;
            }
        }
        second_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecond")
            .unwrap();
    });

    let base_url = format!("http://127.0.0.1:{port}");

    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();
    let responses = session
        .execute_pipelined(&[
            Request::get(format!("{}/one", base_url)).unwrap(),
            Request::get(format!("{}/two", base_url)).unwrap(),
        ])
        .unwrap();

    assert_eq!(responses[0].body_text().unwrap(), "first");
    assert_eq!(responses[1].body_text().unwrap(), "second");
    assert_eq!(connection_count.load(Ordering::SeqCst), 2);
    handle.join().unwrap();
}

#[test]
fn pipelining_retries_unanswered_requests_when_peer_closes_without_connection_close_header() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connection_count = Arc::new(AtomicUsize::new(0));
    let connection_count_for_thread = Arc::clone(&connection_count);
    let handle = thread::spawn(move || {
        let (mut first_stream, _) = listener.accept().unwrap();
        connection_count_for_thread.fetch_add(1, Ordering::SeqCst);

        let mut first_request_bytes = Vec::new();
        let mut chunk = [0u8; 512];
        while first_request_bytes
            .windows(4)
            .filter(|window| *window == b"\r\n\r\n")
            .count()
            < 2
        {
            let read = first_stream.read(&mut chunk).unwrap();
            assert!(
                read > 0,
                "client closed connection before both pipelined requests were sent"
            );
            first_request_bytes.extend_from_slice(&chunk[..read]);
        }

        first_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nfirst")
            .unwrap();
        drop(first_stream);

        let (mut second_stream, _) = listener.accept().unwrap();
        connection_count_for_thread.fetch_add(1, Ordering::SeqCst);
        let mut second_request_bytes = Vec::new();
        loop {
            let read = second_stream.read(&mut chunk).unwrap();
            assert!(
                read > 0,
                "client closed retry connection before sending the unanswered request"
            );
            second_request_bytes.extend_from_slice(&chunk[..read]);
            if second_request_bytes
                .windows(4)
                .any(|window| window == b"\r\n\r\n")
            {
                break;
            }
        }

        second_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecond")
            .unwrap();
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();
    let responses = session
        .execute_pipelined(&[
            Request::get(format!("{}/one", base_url)).unwrap(),
            Request::get(format!("{}/two", base_url)).unwrap(),
        ])
        .unwrap();

    assert_eq!(responses[0].body_text().unwrap(), "first");
    assert_eq!(responses[1].body_text().unwrap(), "second");
    assert_eq!(connection_count.load(Ordering::SeqCst), 2);
    handle.join().unwrap();
}

#[cfg(not(feature = "https"))]
#[test]
fn https_urls_require_the_feature_flag() {
    let error = get("https://example.com").unwrap_err();
    assert!(matches!(
        error,
        nano_get::NanoGetError::HttpsFeatureRequired
    ));
}
