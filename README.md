# nano-get
[![Crates.io](https://img.shields.io/crates/v/nano-get.svg)](https://crates.io/crates/nano-get)
[![Docs.rs](https://docs.rs/nano-get/badge.svg)](https://docs.rs/nano-get)
[![Checker](https://github.com/rapidclock/nano-get/actions/workflows/checker.yml/badge.svg?branch=main)](https://github.com/rapidclock/nano-get/actions/workflows/checker.yml)

`nano-get` is a tiny `HTTP/1.1` client for `GET` and `HEAD`.

- Default build: zero external dependencies.
- HTTPS: enable the `https` feature to use the system OpenSSL library through the optional `openssl` crate.
- API style: simple helper functions for common calls, plus typed `Request`/`Response` and `Client`/`Session` APIs when you need more control.
- Edition: Rust `2021`.
- MSRV: Rust `1.71.0`.
- CI: tested on Rust `1.71.0` and latest `stable`.

## Installation

HTTP only:

```toml
[dependencies]
nano-get = "0.3.0"
```

HTTP + HTTPS:

```toml
[dependencies]
nano-get = { version = "0.3.0", features = ["https"] }
```

## Examples

The repository includes a graduated set of runnable Cargo examples under
[`examples/`](examples/README.md), starting with simple helper functions and ending with advanced
client configuration.

Representative commands:

- `cargo run --example simple-get`
- `cargo run --example request-builder`
- `cargo run --example advanced-client --features https`

## Simple API

The `get` helper is the main ergonomic entry point. It auto-follows redirects and returns the body as UTF-8 text.

```rust
fn main() -> Result<(), nano_get::NanoGetError> {
    let body = nano_get::get("http://example.com")?;
    println!("{body}");
    Ok(())
}
```

If you need raw bytes instead of text:

```rust
fn main() -> Result<(), nano_get::NanoGetError> {
    let body = nano_get::get_bytes("http://example.com")?;
    println!("received {} bytes", body.len());
    Ok(())
}
```

For `HEAD`:

```rust
fn main() -> Result<(), nano_get::NanoGetError> {
    let response = nano_get::head("http://example.com")?;
    println!("status = {}", response.status_code);
    println!("content-type = {:?}", response.header("content-type"));
    Ok(())
}
```

## HTTPS

With the `https` feature enabled, unified helpers route to TLS automatically:

```rust
fn main() -> Result<(), nano_get::NanoGetError> {
    let body = nano_get::get("https://example.com")?;
    println!("{body}");
    Ok(())
}
```

You can also force the protocol-specific helpers:

- `nano_get::get_http`
- `nano_get::get_http_bytes`
- `nano_get::head_http`
- `nano_get::get_https`
- `nano_get::get_https_bytes`
- `nano_get::head_https`

Without the `https` feature, attempting to request an `https://` URL returns a typed `NanoGetError::HttpsFeatureRequired`.

## Advanced API

Use `Request` when you need custom headers or manual redirect handling.

```rust
use nano_get::{RedirectPolicy, Request};

fn main() -> Result<(), nano_get::NanoGetError> {
    let mut request = Request::get("http://example.com")?
        .with_redirect_policy(RedirectPolicy::follow(5));
    request.add_header("Accept", "text/plain")?;

    let response = request.execute()?;
    println!("status = {}", response.status_code);
    println!("reason = {}", response.reason_phrase);
    println!("body = {}", response.body_text()?);
    Ok(())
}
```

Use `Client` when you need connection reuse, caching, or proxy support.

```rust
use nano_get::{CacheMode, Client, ConnectionPolicy, ProxyConfig, Request};

fn main() -> Result<(), nano_get::NanoGetError> {
    let proxy = ProxyConfig::new("http://127.0.0.1:8080")?;
    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .cache_mode(CacheMode::Memory)
        .proxy(proxy)
        .build();

    let response = client.execute(Request::get("https://example.com")?)?;
    println!("status = {}", response.status_code);
    Ok(())
}
```

## Authentication

`nano-get` exposes both ergonomic helpers for common cases and a generic challenge/response hook
for non-Basic schemes.

Challenge-driven Basic auth:

```rust
use nano_get::{Client, Request};

fn main() -> Result<(), nano_get::NanoGetError> {
    let client = Client::builder().basic_auth("user", "pass").build();
    let response = client.execute(Request::get("http://example.com/protected")?)?;
    println!("{}", response.status_code);
    Ok(())
}
```

Challenge-driven Basic proxy auth:

```rust
use nano_get::{Client, ProxyConfig, Request};

fn main() -> Result<(), nano_get::NanoGetError> {
    let proxy = ProxyConfig::new("http://127.0.0.1:8080")?;
    let client = Client::builder()
        .proxy(proxy)
        .basic_proxy_auth("proxy-user", "proxy-pass")
        .build();
    let response = client.execute(Request::get("http://example.com")?)?;
    println!("{}", response.status_code);
    Ok(())
}
```

Preemptive Basic auth is also available when you know the server or proxy requires credentials on
the first request:

- `ClientBuilder::preemptive_basic_auth`
- `ClientBuilder::preemptive_basic_proxy_auth`

Manual request-level overrides:

- `Request::authorization`
- `Request::proxy_authorization`
- `Request::basic_auth`
- `Request::proxy_basic_auth`

For non-Basic schemes, install a custom `AuthHandler` with `ClientBuilder::auth_handler` or
`ClientBuilder::proxy_auth_handler`.

Use `Session` when you want a dedicated persistent connection or pipelined safe requests:

```rust
use nano_get::{Client, ConnectionPolicy, Request};

fn main() -> Result<(), nano_get::NanoGetError> {
    let client = Client::builder()
        .connection_policy(ConnectionPolicy::Reuse)
        .build();
    let mut session = client.session();

    let responses = session.execute_pipelined(&[
        Request::get("http://example.com/one")?,
        Request::get("http://example.com/two")?,
    ])?;
    println!("{} {}", responses[0].status_code, responses[1].status_code);
    Ok(())
}
```

`Response` exposes:

- `version`
- `status_code`
- `reason_phrase`
- ordered `headers`
- optional `trailers`
- raw `body: Vec<u8>`
- helper methods like `header`, `trailer`, `body_text`, and `headers_named`

## Redirects

- Helper functions auto-follow redirects up to `10` hops.
- `Request::execute()` does not follow redirects unless you opt in with `RedirectPolicy::follow(...)`.
- Supported redirect statuses: `301`, `302`, `303`, `307`, `308`.

Origin `Authorization` headers are preserved only for same-scheme, same-host, same-port redirects.
`Proxy-Authorization` is scoped to the configured proxy and is never forwarded to the origin.

## Strict Header Rules

`nano-get` owns the wire-level headers that are required for RFC-correct request framing and proxy
behavior. These headers are rejected if you try to add them manually:

- Protocol-managed: `Host`, `Connection`, `Content-Length`, `Transfer-Encoding`, `Trailer`, `Upgrade`
- Hop-by-hop: `Keep-Alive`, `Proxy-Connection`, `TE`

End-to-end auth headers remain available through the explicit request/auth APIs.

## Caching

The built-in in-memory cache is opt-in through `CacheMode::Memory`.

Supported request directives:

- `max-age`
- `min-fresh`
- `max-stale`
- `only-if-cached`
- `no-cache`
- `no-store`

Supported response directives:

- `max-age`
- `must-revalidate`
- `proxy-revalidate`
- `no-cache`
- `no-store`
- `public`
- `private`

Additional cache behavior:

- `ETag` and `Last-Modified` validator revalidation
- `Vary`-keyed variants
- RFC-style `Age` handling in freshness calculations
- `HEAD` metadata refresh for cached `GET` responses
- cacheable `206 Partial Content` storage and safe partial combination
- conservative auth-aware caching rules

Deliberate exclusions:

- compression
- cookies
- async I/O
- HTTP/2 and HTTP/3

## Compliance

The crate’s compliance claim covers all client-applicable RFC 9110, RFC 9111, and RFC 9112
requirements for an HTTP/1.1 `GET`/`HEAD` user agent, within the documented claim boundary.

Auditable artifacts:

- [docs/compliance/http11-get-head-rfc-matrix.md](docs/compliance/http11-get-head-rfc-matrix.md)
- [docs/compliance/http11-get-head-requirement-test-index.md](docs/compliance/http11-get-head-requirement-test-index.md)

Local compliance/coverage commands:

- `cargo llvm-cov clean --workspace`
- `cargo llvm-cov --workspace --no-default-features --features http --lcov --output-path /tmp/http_cov.info`
- `python3 tools/check_line_coverage.py --lcov /tmp/http_cov.info --root . --require 100`
- `cargo llvm-cov clean --workspace`
- `cargo llvm-cov --workspace --all-features --lcov --output-path /tmp/all_cov.info`
- `python3 tools/check_line_coverage.py --lcov /tmp/all_cov.info --root . --require 100`

## Notes

- `nano-get` supports `HTTP/1.0` and `HTTP/1.1` responses.
- Supported body framing: `Content-Length`, `Transfer-Encoding: chunked`, and connection-close bodies.
- Supports persistent connections, request pipelining for `GET`/`HEAD`, in-memory caching, direct HTTP, direct HTTPS, HTTP proxies, HTTPS over HTTP `CONNECT` proxies, `401`/`407` auth challenge parsing, and Basic auth helpers.
- Conditional request helpers are available on `Request` for validators and ranges.
- Compression, cookies, async, HTTP/2, and non-core auth scheme implementations are intentionally out of scope for `0.3.0`.
