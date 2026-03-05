use std::error::Error;

use nano_get::{Client, ConnectionPolicy, Request};

fn summarize(label: &str, response: &nano_get::Response) {
    println!(
        "{label}: {} {} ({} bytes)",
        response.status_code,
        response.reason_phrase,
        response.body.len()
    );
}

fn main() -> Result<(), Box<dyn Error>> {
    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();

    let first = session.execute(Request::get("http://example.com/")?)?;
    let second = session.execute(Request::get("http://example.com/index.html")?)?;
    let pipelined = session.execute_pipelined(&[
        Request::get("http://example.com/")?,
        Request::get("http://example.com/index.html")?,
    ])?;

    println!("This example demonstrates the Session API shape for reuse and pipelining.");
    summarize("sequential #1", &first);
    summarize("sequential #2", &second);
    summarize("pipeline #1", &pipelined[0]);
    summarize("pipeline #2", &pipelined[1]);

    Ok(())
}
