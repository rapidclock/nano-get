use std::env;
use std::error::Error;
use std::sync::Arc;

use nano_get::{
    AuthDecision, AuthHandler, AuthTarget, Challenge, Client, Header, Request, Response, Url,
};

struct DemoTokenAuth;

impl AuthHandler for DemoTokenAuth {
    fn respond(
        &self,
        _target: AuthTarget,
        _url: &Url,
        challenges: &[Challenge],
        _request: &Request,
        _response: &Response,
    ) -> Result<AuthDecision, nano_get::NanoGetError> {
        let supports_token = challenges
            .iter()
            .any(|challenge| challenge.scheme.eq_ignore_ascii_case("token"));

        if supports_token {
            return Ok(AuthDecision::UseHeaders(vec![Header::new(
                "Authorization",
                "Token example-secret",
            )?]));
        }

        Ok(AuthDecision::NoMatch)
    }
}

fn print_setup() {
    println!("The custom-auth-handler example needs a real protected endpoint.");
    println!("Set NANO_GET_CUSTOM_AUTH_URL to a URL that challenges with a custom scheme.");
    println!("The example handler looks for a scheme named `Token`.");
}

fn main() -> Result<(), Box<dyn Error>> {
    let Some(url) = env::var("NANO_GET_CUSTOM_AUTH_URL").ok() else {
        print_setup();
        return Ok(());
    };

    let client = Client::builder()
        .auth_handler(Arc::new(DemoTokenAuth))
        .build();
    let response = client.execute(Request::get(&url)?)?;

    println!("custom-auth-handler example");
    println!(
        "status: {} {}",
        response.status_code, response.reason_phrase
    );
    println!(
        "parsed WWW-Authenticate challenges: {}",
        response.www_authenticate_challenges()?.len()
    );

    Ok(())
}
