mod models;

pub use models::URL;
pub use models::ToUrl;
use models::Tuple;

pub fn parse_proto(s: String) -> (String, String) {
    let parts: Vec<&str> = s.split("://").collect();
    if parts.len() > 1 {
        (parts.first().unwrap().to_string(), parts[1].to_string())
    } else {
        ("http".to_string(), parts[0].to_string())
    }
}

pub fn parse_full_domain(s: String) -> (String, String) {
    if let Some(i) = s.find("/") {
        let fdom = &s[0..i];
        let rest = &s[i..];
        (fdom.to_string(), rest.to_string())
    } else {
        (s, "/".to_string())
    }
}

pub fn parse_host_and_port(s: String) -> (String, String) {
    if let Some(_) = s.find(":") {
        let tuple: Tuple<&str> = s.splitn(2, ":").collect();
        (tuple.left.to_string(), tuple.right.to_string())
    } else {
        (s, "80".to_string())
    }
}