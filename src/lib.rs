pub mod url;
pub mod http;


#[cfg(test)]
mod tests {
    use super::*;
    use url;

    #[test]
    fn test_proto_parse_http() {
        let url_str = "http://example.com/?a=1&b=2&c=3".to_string();
        let (a, b) = url::parse_proto(url_str);
        println!("{}, {}", a, b);
        assert_eq!(a, "http".to_string());
    }

    #[test]
    fn test_proto_parse_https() {
        let url_str = "https://example.com/?a=1&b=2&c=3".to_string();
        let (a, b) = url::parse_proto(url_str);
        println!("{}, {}", a, b);
        assert_eq!(a, "https".to_string());
    }

    #[test]
    fn test_proto_parse_ftp() {
        let url_str = "ftp://example.com/?a=1&b=2&c=3".to_string();
        let (a, b) = url::parse_proto(url_str);
        println!("{}, {}", a, b);
        assert_eq!(a, "ftp".to_string());
    }

    #[test]
    fn test_proto_parse_none() {
        let url_str = "example.com/?a=1&b=2&c=3".to_string();
        let (a, b) = url::parse_proto(url_str);
        println!("{}, {}", a, b);
        assert_eq!(a, "http".to_string());
    }
}
