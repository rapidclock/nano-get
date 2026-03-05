use std::error::Error;

fn preview_bytes(bytes: &[u8], max_len: usize) -> String {
    let shown = bytes
        .iter()
        .take(max_len)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > max_len {
        format!("{shown} ...")
    } else {
        shown
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let url = "http://example.com";
    let body = nano_get::get_bytes(url)?;

    println!("GET {url}");
    println!("received {} bytes", body.len());
    println!("hex preview: {}", preview_bytes(&body, 16));

    Ok(())
}
