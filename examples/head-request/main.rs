use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let url = "http://example.com";
    let response = nano_get::head(url)?;

    println!("HEAD {url}");
    println!(
        "status: {} {}",
        response.status_code, response.reason_phrase
    );
    println!("content-type: {:?}", response.header("content-type"));
    println!("content-length: {:?}", response.header("content-length"));

    let server_headers = response
        .headers_named("server")
        .map(|header| header.value().to_string())
        .collect::<Vec<_>>();
    println!("server headers: {server_headers:?}");
    println!("body length after HEAD: {}", response.body.len());

    Ok(())
}
