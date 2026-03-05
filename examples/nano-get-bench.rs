use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use nano_get::{Client, ConnectionPolicy, Request};

fn spawn_persistent_server(
    responses: usize,
    response_bytes: Vec<u8>,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(false).expect("set_nonblocking");
    let addr = listener.local_addr().expect("local_addr");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut pending = Vec::new();
        let mut chunk = [0u8; 4096];
        for _ in 0..responses {
            loop {
                if pending.windows(4).any(|w| w == b"\r\n\r\n") {
                    let end = pending
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .expect("request terminator")
                        + 4;
                    pending.drain(..end);
                    break;
                }
                let n = stream.read(&mut chunk).expect("read request");
                if n == 0 {
                    return;
                }
                pending.extend_from_slice(&chunk[..n]);
            }
            stream.write_all(&response_bytes).expect("write response");
        }
    });
    (format!("http://{}", addr), handle)
}

fn bench_get(iterations: usize) -> Duration {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
    let (base_url, handle) = spawn_persistent_server(iterations, response);

    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();

    let start = Instant::now();
    for i in 0..iterations {
        let req = Request::get(format!("{}/g{}", base_url, i)).expect("request");
        let resp = session.execute(req).expect("execute");
        assert_eq!(resp.body, b"hello");
    }
    let elapsed = start.elapsed();
    handle.join().expect("join server");
    elapsed
}

fn bench_head(iterations: usize) -> Duration {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Bench: 1\r\n\r\n".to_vec();
    let (base_url, handle) = spawn_persistent_server(iterations, response);

    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();

    let start = Instant::now();
    for i in 0..iterations {
        let req = Request::head(format!("{}/h{}", base_url, i)).expect("request");
        let resp = session.execute(req).expect("execute");
        assert!(resp.body.is_empty());
    }
    let elapsed = start.elapsed();
    handle.join().expect("join server");
    elapsed
}

fn bench_pipeline(total_requests: usize, pipeline_depth: usize) -> Duration {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
    let (base_url, handle) = spawn_persistent_server(total_requests, response);

    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();

    let start = Instant::now();
    let mut sent = 0usize;
    while sent < total_requests {
        let batch_size = (total_requests - sent).min(pipeline_depth);
        let mut batch = Vec::with_capacity(batch_size);
        for index in 0..batch_size {
            batch.push(Request::get(format!("{}/p{}", base_url, sent + index)).expect("request"));
        }
        let responses = session.execute_pipelined(&batch).expect("execute");
        assert_eq!(responses.len(), batch_size);
        for response in responses {
            assert_eq!(response.body, b"hello");
        }
        sent += batch_size;
    }
    let elapsed = start.elapsed();
    handle.join().expect("join server");
    elapsed
}

fn fmt_ops(iter: usize, d: Duration) -> f64 {
    iter as f64 / d.as_secs_f64()
}

fn main() {
    let warmup = 2_000;
    let iterations = 30_000;
    let pipeline_iterations = 32_000;
    let pipeline_depth = 8;

    let _ = bench_get(warmup);
    let _ = bench_head(warmup);
    let _ = bench_pipeline(warmup, pipeline_depth);

    let get_time = bench_get(iterations);
    let head_time = bench_head(iterations);
    let pipeline_time = bench_pipeline(pipeline_iterations, pipeline_depth);

    println!(
        "GET  {} ops in {:?} => {:.0} req/s",
        iterations,
        get_time,
        fmt_ops(iterations, get_time)
    );
    println!(
        "HEAD {} ops in {:?} => {:.0} req/s",
        iterations,
        head_time,
        fmt_ops(iterations, head_time)
    );
    println!(
        "PIPE {} ops in {:?} => {:.0} req/s (depth={})",
        pipeline_iterations,
        pipeline_time,
        fmt_ops(pipeline_iterations, pipeline_time),
        pipeline_depth
    );
}
