use std::error::Error;

fn preview_text(text: &str, max_chars: usize) -> String {
    let mut preview: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

fn main() -> Result<(), Box<dyn Error>> {
    let url = "http://example.com";
    let body = nano_get::get(url)?;

    println!("GET {url}");
    println!("received {} UTF-8 characters", body.chars().count());
    println!("preview: {}", preview_text(&body, 80));

    Ok(())
}
