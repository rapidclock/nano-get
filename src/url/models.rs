use std::fmt::{Display, Error, Formatter};
use std::io;
use std::iter::{FromIterator, IntoIterator};

use crate::url::{parse_full_domain, parse_host_and_port, parse_proto};

/// This is used to represent the various parts of a URL.
#[derive(Debug, Clone)]
pub struct URL {
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

impl Display for URL {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        write!(f, "url: {},\nproto: {},\nhost: {},\nport: {},\npath: {}\n", self._absolute, self.protocol, self.host, self.port, self.path)
    }
}

impl URL {
    pub fn new(url: &str) -> Self {
        let url = url.to_string();
        let (protocol, rest) = parse_proto(url.clone(), None);
        let (full_domain, path) = parse_full_domain(rest.clone(), None);
        let (host, port) = parse_host_and_port(full_domain, Self::get_default_port_for_proto(&protocol));
        URL {
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
    fn to_url(&self) -> io::Result<URL>;
}


impl ToUrl for String {
    fn to_url(&self) -> io::Result<URL> {
        Ok(URL::new(self))
    }
}

impl ToUrl for &str {
    fn to_url(&self) -> io::Result<URL> {
        Ok(URL::new(self))
    }
}

impl ToUrl for URL {
    fn to_url(&self) -> io::Result<URL> {
        Ok(self.clone())
    }
}

impl ToUrl for &'_ URL {
    fn to_url(&self) -> io::Result<URL> {
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
                let left = left.clone();
                let right = right.clone();
                return Tuple { left, right };
            }
        }
        panic!("not enough elements");
    }
}