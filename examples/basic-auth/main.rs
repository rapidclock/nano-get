use std::env;
use std::error::Error;

use nano_get::{Client, Request};

fn env_or_default(name: &str, fallback: &str) -> String {
    env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn print_setup() {
    println!("The basic-auth example needs a real protected endpoint.");
    println!("Set NANO_GET_BASIC_AUTH_URL to a URL that challenges with HTTP Basic auth.");
    println!("Optional overrides:");
    println!("  NANO_GET_BASIC_AUTH_USER");
    println!("  NANO_GET_BASIC_AUTH_PASS");
}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(url) = env::var("NANO_GET_BASIC_AUTH_URL").ok() else {
        print_setup();
        return Ok(());
    };

    let user = env_or_default("NANO_GET_BASIC_AUTH_USER", "demo-user");
    let pass = env_or_default("NANO_GET_BASIC_AUTH_PASS", "demo-pass");

    let challenge_driven = Client::builder()
        .basic_auth(user.clone(), pass.clone())
        .build()
        .execute(Request::get(&url)?)?;

    let preemptive = Client::builder()
        .preemptive_basic_auth(user.clone(), pass.clone())
        .build()
        .execute(Request::get(&url)?)?;

    let mut request = Request::get(&url)?;
    request.basic_auth(user, pass)?;
    let manual = request.execute()?;

    println!("basic-auth example");
    println!("challenge-driven status: {}", challenge_driven.status_code);
    println!("preemptive status: {}", preemptive.status_code);
    println!("request-level override status: {}", manual.status_code);

    Ok(())
}
