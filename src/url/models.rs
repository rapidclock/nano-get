use std::fmt::{Display, Error, Formatter};
use std::io;
use std::iter::{FromIterator, IntoIterator};

use super::{parse_full_domain, parse_host_and_port, parse_proto};

/// This is used to represent the various parts of a URL.
#[derive(Debug, Clone)]
pub struct Url {
    /// represents the protocol used in the URL (defaults to http).
    pub protocol: String,
    /// represents the Host part of the URL.
    pub host: String,
    /// represents the port of the URL (if specified). Default is port 80 for http.
    pub port: String,
    /// everything after the / (/ is the default value).
    pub path: String,

    _absolute: String,
}

impl Display for Url {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        write!(f, "url: {},\nproto: {},\nhost: {},\nport: {},\npath: {}\n", self._absolute, self.protocol, self.host, self.port, self.path)
    }
}

impl Url {
    pub fn new(url: &str) -> Self {
        let url = url.to_string();
        let (protocol, rest) = parse_proto(url.clone(), None);
        let (full_domain, path) = parse_full_domain(rest, None);
        let (host, port) = parse_host_and_port(full_domain, Self::get_default_port_for_proto(&protocol));
        Url {
            protocol,
            host,
            port,
            path,
            _absolute: url,
        }
    }

    fn get_default_port_for_proto(proto: &str) -> Option<String> {
        match proto {
            "http" => Some("80".to_string()),
            "https" => Some("443".to_string()),
            _ => None
        }
    }

    /// returns the complete url (which was used to create the URL).
    pub fn get_full_url(&self) -> String {
        self.protocol.clone() + "://" + &self.host + ":" + &self.port + &self.path
    }

    /// returns the host:port of the url.
    pub fn get_host_with_port(&self) -> String {
        self.host.clone() + ":" + &self.port
    }
}

/// Represents the ability to be made into a URL.
pub trait ToUrl {
    fn to_url(&self) -> io::Result<Url>;
}


impl ToUrl for String {
    fn to_url(&self) -> io::Result<Url> {
        Ok(Url::new(self))
    }
}

impl ToUrl for &str {
    fn to_url(&self) -> io::Result<Url> {
        Ok(Url::new(self))
    }
}

impl ToUrl for Url {
    fn to_url(&self) -> io::Result<Url> {
        Ok(self.clone())
    }
}

impl ToUrl for &'_ Url {
    fn to_url(&self) -> io::Result<Url> {
        Ok((*self).clone())
    }
}

pub struct Tuple<T>
    where T: Clone {
    pub left: T,
    pub right: T,
}

impl<T> FromIterator<T> for Tuple<T>
    where T: Clone {
    fn from_iter<I: IntoIterator<Item=T>>(iter: I) -> Tuple<T> {
        let mut iterator = iter.into_iter();
        if let Some(left) = iterator.next() {
            if let Some(right) = iterator.next() {
                let left = left;
                let right = right;
                return Tuple { left, right };
            }
        }
        panic!("not enough elements");
    }
}