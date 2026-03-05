use std::error::Error;
use std::time::{Duration, UNIX_EPOCH};

use nano_get::{RedirectPolicy, Request};

fn preview_text(text: &str, max_chars: usize) -> String {
    let mut preview: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

fn main() -> Result<(), Box<dyn Error>> {
    let url = "http://example.com";
    let mut request = Request::get(url)?.with_redirect_policy(RedirectPolicy::follow(5));

    request.add_header("Accept", "text/html,application/xhtml+xml")?;
    request.add_header("Cache-Control", "max-age=0")?;
    request.if_none_match("\"demo-etag\"")?;
    request.if_modified_since(UNIX_EPOCH + Duration::from_secs(784_111_777))?;
    request.range_bytes(Some(0), Some(255))?;

    // Protocol-managed headers such as Host and Connection are intentionally rejected by the
    // library. This example only uses safe end-to-end request headers.
    let response = request.execute()?;

    println!("manual request to {url}");
    println!(
        "status: {} {}",
        response.status_code, response.reason_phrase
    );
    println!("content-type: {:?}", response.header("content-type"));
    println!("body preview: {}", preview_text(response.body_text()?, 80));

    Ok(())
}
