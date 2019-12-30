# nano-get
A very tiny &amp; basic implementation of HTTP GET using only the standard library


## Example Usage
```rust
use nano_get::http;

fn main() {
    let response = http::get("http://dummy.restapiexample.com/api/v1/employees");
    println!("{}", response);
}
```
