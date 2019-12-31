# nano-get
[![Crates.io](https://img.shields.io/crates/v/nano-get.svg)](https://crates.io/crates/nano-get)
[![Docs.rs](https://docs.rs/nano-get/badge.svg)](https://docs.rs/nano-get)

A very tiny &amp; basic implementation of HTTP GET using only the standard library

Please _note_ that this is not the best or most efficient implementation of the HTTP GET. The whole purpose is to have a basic functioning HTTP GET implementation and avoid having to import a gazzilion other packages, when all you want is a regular GET method for something simple.

Do not use if you rely on https (as there is no encryption _yet_ as part of this library). I would go further and advice _NOT_ to use this in a production environment.

More features might be added later, with the primary goal being to reduce the final binary size by not having (possible any) dependencies other than what is in the standard library.

So Async, manual headers, more fine grained control of the request/response, will all come later(hopefully).

## Example Usage
```rust
use nano_get::get;

fn main() {
    let response = get("http://dummy.restapiexample.com/api/v1/employees");
    println!("{}", response);
}
```
