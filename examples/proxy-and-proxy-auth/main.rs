use std::env;
use std::error::Error;

use nano_get::{Client, ProxyConfig, Request};

fn env_or_default(name: &str, fallback: &str) -> String {
    env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn print_setup() {
    println!("The proxy-and-proxy-auth example needs a reachable proxy.");
    println!("Set NANO_GET_PROXY_URL to an HTTP proxy URL, for example http://127.0.0.1:8080.");
    println!("Optional overrides:");
    println!("  NANO_GET_PROXY_TARGET_URL");
    println!("  NANO_GET_PROXY_AUTH_USER");
    println!("  NANO_GET_PROXY_AUTH_PASS");
}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(proxy_url) = env::var("NANO_GET_PROXY_URL").ok() else {
        print_setup();
        return Ok(());
    };

    let target_url = env_or_default("NANO_GET_PROXY_TARGET_URL", "http://example.com");
    let proxy_user = env_or_default("NANO_GET_PROXY_AUTH_USER", "proxy-user");
    let proxy_pass = env_or_default("NANO_GET_PROXY_AUTH_PASS", "proxy-pass");

    let mut proxy = ProxyConfig::new(proxy_url)?;
    proxy.add_header("X-Example-Proxy", "nano-get-demo")?;

    let challenge_driven = Client::builder()
        .proxy(proxy.clone())
        .basic_proxy_auth(proxy_user.clone(), proxy_pass.clone())
        .build()
        .execute(Request::get(&target_url)?)?;

    let preemptive = Client::builder()
        .proxy(proxy.clone())
        .preemptive_basic_proxy_auth(proxy_user.clone(), proxy_pass.clone())
        .build()
        .execute(Request::get(&target_url)?)?;

    let mut manual = Request::get(&target_url)?;
    manual.proxy_basic_auth(proxy_user, proxy_pass)?;
    let manual_response = Client::builder().proxy(proxy).build().execute(manual)?;

    println!("proxy-and-proxy-auth example");
    println!("challenge-driven status: {}", challenge_driven.status_code);
    println!("preemptive status: {}", preemptive.status_code);
    println!(
        "request-level override status: {}",
        manual_response.status_code
    );

    Ok(())
}
