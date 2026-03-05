use std::error::Error;

fn preview_text(text: &str, max_chars: usize) -> String {
    let mut preview: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

fn main() -> Result<(), Box<dyn Error>> {
    let http_url = "http://example.com";
    let https_url = "https://example.com";

    let http_text = nano_get::get_http(http_url)?;
    let http_bytes = nano_get::get_http_bytes(http_url)?;
    let http_head = nano_get::head_http(http_url)?;

    let https_text = nano_get::get_https(https_url)?;
    let https_bytes = nano_get::get_https_bytes(https_url)?;
    let https_head = nano_get::head_https(https_url)?;

    println!("HTTP text preview: {}", preview_text(&http_text, 60));
    println!("HTTP byte length: {}", http_bytes.len());
    println!("HTTP HEAD status: {}", http_head.status_code);

    println!("HTTPS text preview: {}", preview_text(&https_text, 60));
    println!("HTTPS byte length: {}", https_bytes.len());
    println!("HTTPS HEAD status: {}", https_head.status_code);

    Ok(())
}
