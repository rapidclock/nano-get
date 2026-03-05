use std::error::Error;

use nano_get::{CacheMode, Client, Request};

fn main() -> Result<(), Box<dyn Error>> {
    let client = Client::builder().cache_mode(CacheMode::Memory).build();

    let first = Request::get("http://example.com")?;
    let mut second = Request::get("http://example.com")?;
    second.add_header("Cache-Control", "max-age=0, min-fresh=1")?;

    let first_response = client.execute(first)?;
    let second_response = client.execute(second)?;

    println!("memory cache example");
    println!(
        "first response: {} {}",
        first_response.status_code, first_response.reason_phrase
    );
    println!(
        "second response with request directives: {} {}",
        second_response.status_code, second_response.reason_phrase
    );
    println!("This example demonstrates cache configuration and cache-control request headers.");

    Ok(())
}
