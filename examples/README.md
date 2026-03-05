# `nano-get` Examples

These examples are reference-oriented Cargo example targets for the root `nano-get` package.
They are ordered from the smallest helper-based usage to the most advanced client configuration.

## Examples

1. `simple-get`
   - Demonstrates `nano_get::get`
   - Run: `cargo run --example simple-get`
   - Network: HTTP only

2. `get-bytes`
   - Demonstrates `nano_get::get_bytes`
   - Run: `cargo run --example get-bytes`
   - Network: HTTP only

3. `head-request`
   - Demonstrates `nano_get::head` and response metadata access
   - Run: `cargo run --example head-request`
   - Network: HTTP only

4. `request-builder`
   - Demonstrates `Request`, custom headers, redirect policy, validators, and ranges
   - Run: `cargo run --example request-builder`
   - Network: HTTP only

5. `protocol-specific-helpers`
   - Demonstrates HTTP-only and HTTPS-only helper APIs
   - Run: `cargo run --example protocol-specific-helpers --features https`
   - Network: HTTP + HTTPS

6. `session-reuse-and-pipelining`
   - Demonstrates `Client`, `Session`, connection reuse, and pipelined GET requests
   - Run: `cargo run --example session-reuse-and-pipelining`
   - Network: HTTP only

7. `memory-cache`
   - Demonstrates `CacheMode::Memory` and request cache directives
   - Run: `cargo run --example memory-cache`
   - Network: HTTP only

8. `basic-auth`
   - Demonstrates built-in Basic auth helpers and request-level overrides
   - Run: `cargo run --example basic-auth`
   - Network: configurable protected endpoint via `NANO_GET_BASIC_AUTH_URL`

9. `custom-auth-handler`
   - Demonstrates the generic `AuthHandler` interface
   - Run: `cargo run --example custom-auth-handler`
   - Network: configurable protected endpoint via `NANO_GET_CUSTOM_AUTH_URL`

10. `proxy-and-proxy-auth`
   - Demonstrates `ProxyConfig`, proxy headers, and proxy auth helpers
   - Run: `cargo run --example proxy-and-proxy-auth`
   - Network: configurable proxy via `NANO_GET_PROXY_URL`

11. `advanced-client`
   - Demonstrates redirects, reuse, cache, optional proxy, and optional auth together
   - Run: `cargo run --example advanced-client --features https`
   - Network: HTTPS by default, optional proxy/auth env overrides

## Environment Variables

Some advanced examples rely on real endpoints to show the full behavior:

- `NANO_GET_BASIC_AUTH_URL`
- `NANO_GET_CUSTOM_AUTH_URL`
- `NANO_GET_PROXY_URL`
- `NANO_GET_PROXY_TARGET_URL`
- `NANO_GET_ADVANCED_URL`
- `NANO_GET_BASIC_AUTH_USER`
- `NANO_GET_BASIC_AUTH_PASS`
- `NANO_GET_PROXY_AUTH_USER`
- `NANO_GET_PROXY_AUTH_PASS`

When those variables are missing, the auth/proxy-focused examples print setup guidance and exit
successfully.
