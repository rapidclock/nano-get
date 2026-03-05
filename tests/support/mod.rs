#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub struct TestServer {
    pub base_url: String,
    pub request_lines: Arc<Mutex<Vec<String>>>,
    pub requests: Arc<Mutex<Vec<String>>>,
    pub connection_count: Arc<Mutex<usize>>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    pub fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

pub enum Interaction {
    Once(Vec<u8>),
    Persistent(Vec<Vec<u8>>),
}

pub fn spawn_http_server(responses: Vec<Vec<u8>>) -> TestServer {
    spawn_scripted_http_server(responses.into_iter().map(Interaction::Once).collect())
}

pub fn spawn_persistent_http_server(responses: Vec<Vec<u8>>) -> TestServer {
    spawn_scripted_http_server(vec![Interaction::Persistent(responses)])
}

pub fn spawn_handler_http_server<F>(expected_requests: usize, mut handler: F) -> TestServer
where
    F: FnMut(String) -> Vec<u8> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let connection_count = Arc::new(Mutex::new(0usize));
    let request_lines_for_thread = Arc::clone(&request_lines);
    let requests_for_thread = Arc::clone(&requests);
    let connection_count_for_thread = Arc::clone(&connection_count);

    let handle = thread::spawn(move || {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().unwrap();
            *connection_count_for_thread.lock().unwrap() += 1;

            let mut pending = Vec::new();
            let request = read_request(&mut stream, &mut pending);
            record_request(
                &request_lines_for_thread,
                &requests_for_thread,
                request.clone(),
            );
            let response = handler(request);
            stream.write_all(&response).unwrap();
        }
    });

    TestServer {
        base_url: format!("http://127.0.0.1:{}", address.port()),
        request_lines,
        requests,
        connection_count,
        handle: Some(handle),
    }
}

pub fn spawn_scripted_http_server(interactions: Vec<Interaction>) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let connection_count = Arc::new(Mutex::new(0usize));
    let request_lines_for_thread = Arc::clone(&request_lines);
    let requests_for_thread = Arc::clone(&requests);
    let connection_count_for_thread = Arc::clone(&connection_count);

    let handle = thread::spawn(move || {
        for interaction in interactions {
            let (mut stream, _) = listener.accept().unwrap();
            *connection_count_for_thread.lock().unwrap() += 1;

            match interaction {
                Interaction::Once(response) => {
                    let mut pending = Vec::new();
                    let request = read_request(&mut stream, &mut pending);
                    record_request(&request_lines_for_thread, &requests_for_thread, request);
                    stream.write_all(&response).unwrap();
                }
                Interaction::Persistent(responses) => {
                    let mut pending = Vec::new();
                    for response in responses {
                        let request = read_request(&mut stream, &mut pending);
                        if request.is_empty() {
                            break;
                        }
                        record_request(&request_lines_for_thread, &requests_for_thread, request);
                        stream.write_all(&response).unwrap();
                    }
                }
            }
        }
    });

    TestServer {
        base_url: format!("http://127.0.0.1:{}", address.port()),
        request_lines,
        requests,
        connection_count,
        handle: Some(handle),
    }
}

fn record_request(
    request_lines: &Arc<Mutex<Vec<String>>>,
    requests: &Arc<Mutex<Vec<String>>>,
    request: String,
) {
    let request_line = request
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r')
        .to_string();
    request_lines.lock().unwrap().push(request_line);
    requests.lock().unwrap().push(request);
}

fn read_request(stream: &mut impl Read, pending: &mut Vec<u8>) -> String {
    let mut chunk = [0u8; 512];

    loop {
        if let Some(end) = pending.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = end + 4;
            let request = pending[..end].to_vec();
            pending.drain(..end);
            return String::from_utf8_lossy(&request).into_owned();
        }

        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        pending.extend_from_slice(&chunk[..read]);
    }

    if pending.is_empty() {
        String::new()
    } else {
        let request = pending.split_off(0);
        String::from_utf8_lossy(&request).into_owned()
    }
}
