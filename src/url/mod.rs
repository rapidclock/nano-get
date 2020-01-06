pub mod models;

pub use self::models::{Url, ToUrl, Tuple};

pub fn parse_proto(s: String, default_proto: Option<String>) -> (String, String) {
    let parts: Vec<&str> = s.split("://").collect();
    if parts.len() > 1 {
        ((*parts.first().unwrap()).to_string(), parts[1].to_string())
    } else {
        match default_proto {
            Some(proto) => (proto, parts[0].to_string()),
            None => ("http".to_string(), parts[0].to_string())
        }
    }
}

pub fn parse_full_domain(s: String, default_path: Option<String>) -> (String, String) {
    if let Some(i) = s.find('/') {
        let fdom = &s[0..i];
        let rest = &s[i..];
        (fdom.to_string(), rest.to_string())
    } else {
        match default_path {
            Some(path) => (s, path),
            None => (s, "/".to_string())
        }
    }
}

pub fn parse_host_and_port(s: String, default_port: Option<String>) -> (String, String) {
    if s.find(':').is_some() {
        let tuple: Tuple<&str> = s.splitn(2, ':').collect();
        (tuple.left.to_string(), tuple.right.to_string())
    } else {
        match default_port {
            Some(port) => (s, port),
            None => (s, "80".to_string())
        }
    }
}