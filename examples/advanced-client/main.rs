use std::env;
use std::error::Error;

use nano_get::{CacheMode, Client, ConnectionPolicy, ProxyConfig, RedirectPolicy, Request};

fn env_string(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn summarize(label: &str, response: &nano_get::Response) {
    println!(
        "{label}: {} {} ({} bytes)",
        response.status_code,
        response.reason_phrase,
        response.body.len()
    );
}

fn main() -> Result<(), Box<dyn Error>> {
    let url =
        env_string("NANO_GET_ADVANCED_URL").unwrap_or_else(|| "https://example.com".to_string());
    let mut builder = Client::builder()
        .redirect_policy(RedirectPolicy::follow(10))
        .connection_policy(ConnectionPolicy::Reuse)
        .cache_mode(CacheMode::Memory);

    if let Some(proxy_url) = env_string("NANO_GET_PROXY_URL") {
        builder = builder.proxy(ProxyConfig::new(proxy_url)?);
    }

    if let (Some(user), Some(pass)) = (
        env_string("NANO_GET_BASIC_AUTH_USER"),
        env_string("NANO_GET_BASIC_AUTH_PASS"),
    ) {
        builder = builder.basic_auth(user, pass);
    }

    if let (Some(user), Some(pass)) = (
        env_string("NANO_GET_PROXY_AUTH_USER"),
        env_string("NANO_GET_PROXY_AUTH_PASS"),
    ) {
        builder = builder.basic_proxy_auth(user, pass);
    }

    let client = builder.build();

    let mut request = Request::get(&url)?.with_redirect_policy(RedirectPolicy::follow(5));
    request.add_header("Accept", "text/html,application/xhtml+xml")?;
    request.add_header("Cache-Control", "max-age=0")?;

    let direct = client.execute_ref(&request)?;
    let mut session = client.session();
    let session_response = session.execute(request)?;

    println!("advanced-client example");
    println!("target URL: {url}");
    summarize("client.execute_ref", &direct);
    summarize("session.execute", &session_response);

    Ok(())
}
