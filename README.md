# nano-get
[![Crates.io](https://img.shields.io/crates/v/nano-get.svg)](https://crates.io/crates/nano-get)
[![Docs.rs](https://docs.rs/nano-get/badge.svg)](https://docs.rs/nano-get)

A tiny implementation of HTTP GET using only the standard library by default.

If you require `https`, please enable the `"https"` feature flag like:
```
nano-get = { version = "0.2.1", features = ["https"] }
```

Enabling the `https` flag, uses the rust [openssl](https://crates.io/crates/openssl) crate.
 
The OpenSSL Crate assumes that you have OpenSSL in your environment.

Please _note_ that this may not be the best or most efficient implementation of the HTTP GET. 
The whole purpose is to have a basic functioning HTTP GET implementation and avoid having to 
import a gazzilion other packages, when all you want is a regular GET method for something simple.

More features might be added later, with the primary goal being to reduce the final binary size 
by not having too many dependencies other than this crate.

Currently the only other dependency is the [openssl](https://crates.io/crates/openssl) crate if you
enable the `"https"` feature flag for this crate. The default use of this crate has zero external dependencies,
other than the standard library.

## Feature Flags
* `https` : This enables https based on the Rust [openssl](https://crates.io/crates/openssl) crate

## Example Usages

If all you care about is making a get request, then you can call the `nano_get::get()` method like below.
```rust
extern crate nano_get;
use nano_get::get;

fn main() {
    let response = get("http://dummy.restapiexample.com/api/v1/employees");
    println!("{}", response);
}
```
An example with the `https` feature flag enabled:
```rust
extern crate nano_get;
use nano_get::get;

fn main() {
    let response = get("https://google.com");
    println!("{}", response);
}
```

For more fine-grained control of the request/response, you can construct a request.

```rust
extern crate nano_get;
use nano_get::get;

fn main() {
    let mut request = Request::default_get_request("http://dummy.restapiexample.com/api/v1/employees").unwrap();
    request.add_header("test", "abracadabra");
    let response = request.execute().unwrap();
    println!("{}", response.status);
    println!("{}", response.body);
}
```

## Models
The basic models in this crate are:
* Url
* Request
* Response

## Executing HTTP(s) Requests:

There are two ways to execute the HTTP(s) requests.

### Basic Get
The basic version, demonstrated by the use of the `nano_get::get` function, which takes a url
and returns the body of the response.

#### Example
```rust
extern crate nano_get;
use nano_get::get;

fn main() {
    let response = nano_get::get("https://www.google.com");
    println!("{}", response);
}
```

### Request-Response based
Another more fine-grained method exists by using the `nano_get::Request` object.
This gives you access to request headers, optional request body and the execution returns a
`nano_get::Response` object. This allows inspection of HTTP Response codes, response body, etc.

#### Example
```rust
extern crate nano_get;
use nano_get::{Request, Response};

fn main() {
    let mut request = Request::default_get_request("http://example.com/").unwrap();
    let response: Response = request.execute().unwrap();
    println!("{}", response.body);
}
```
For details, check the `Request` and `Response` structure documentation.

## Async:
As of writing this, this crate does not using async/await features of Rust.
However, this does not stop the user from using this library in their application in a async context.

A dummy example is shown below. This uses the free REST API at [jsonplaceholder](https://jsonplaceholder.typicode.com) to retreive 100 albums (100 HTTPS GET requests) concurrently using tokio/futures async/await utils.

This example is not a benchmark and only meant to demonstrate how to write an async wrapper around the crate's get method.
This is also not meant to be demonstrative of idiomatic uses of the async libraries.

**Cargo.toml snippet**
```toml
[dependencies]
nano-get = {version = "0.2.1", features = ["https"] }
tokio = { version = "0.2.6", features = ["rt-threaded"] }
futures = "0.3.1"
```

**main.rs**
```rust
extern crate futures;
extern crate nano_get;
extern crate tokio;

use std::time::Instant;

use tokio::runtime::{Runtime, Builder};
use futures::future::try_join_all;
use nano_get::get;

fn main() {
    let mut runtime: Runtime = Builder::new().threaded_scheduler().build().unwrap();
    runtime.block_on(async {
        let base_url = "https://jsonplaceholder.typicode.com/albums";
        let mut handles = Vec::with_capacity(100);
        let start = Instant::now();
        for i in 1..=100 {
            let url = format!("{}/{}", base_url, i);
            handles.push(tokio::spawn(get_url(url)));
        }
        let responses: Vec<String> = try_join_all(handles).await.unwrap();
        let duration = start.elapsed();
        println!("# : {}\n{}", responses.len(), responses.last().unwrap());
        println!("Time elapsed in http get is: {:?}", duration);
        println!("Average time for get is: {}s", duration.as_secs_f64() / (responses.len() as f64));
    });
}

async fn get_url(url: String) -> String {
    get(url)
}
```

**example output**
```text
# : 100
{
  "userId": 10,
  "id": 100,
  "title": "enim repellat iste"
}
Time elapsed in http get is: 3.671184788s
Average time for get is: 0.03671184788s
```
