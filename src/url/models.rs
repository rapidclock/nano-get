use std::fmt::{Display, Error, Formatter};
use std::io;
use std::iter::{FromIterator, IntoIterator};

use crate::url::{parse_full_domain, parse_host_and_port, parse_proto};

#[derive(Debug, Clone)]
pub struct URL {
    pub protocol: String,
    pub host: String,
    pub port: String,
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
        let (protocol, rest) = parse_proto(url.clone());
        let (full_domain, path) = parse_full_domain(rest.clone());
        let (host, port) = parse_host_and_port(full_domain.clone());
        URL {
            protocol,
            host,
            port,
            path,
            _absolute: url,
        }
    }

    pub fn get_full_url(&self) -> String {
        self._absolute.clone()
    }

    pub fn get_host_with_port(&self) -> String {
        self.host.clone() + ":" + &self.port
    }
}

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