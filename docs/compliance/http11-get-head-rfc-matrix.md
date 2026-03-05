# HTTP/1.1 GET/HEAD RFC Matrix

This matrix documents the crate's intended compliance surface for the client-applicable parts of:

- RFC 9110: HTTP Semantics
- RFC 9111: HTTP Caching
- RFC 9112: HTTP/1.1

Claim boundary:

- Included: direct origin requests, HTTP proxy requests, HTTPS over HTTP `CONNECT`, `GET`, `HEAD`,
  redirects, validator revalidation, in-memory caching, challenge parsing, and origin/proxy auth
  retry plumbing.
- Excluded: cookies, content-coding decompression, HTTP/2, HTTP/3, async I/O, non-core auth scheme
  implementations, and `206` cache combination/storage.

Status values:

- `implemented`
- `not applicable`
- `out of scope`

## RFC 9110

| ID | Section | Level | Applicability | Summary | Status | Code | Tests |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 9110-method-get-head | 9.3.1, 9.3.2 | MUST | client | Support `GET` and `HEAD` semantics, including empty `HEAD` bodies. | implemented | `src/request.rs`, `src/response.rs` | `head_returns_metadata_and_empty_body`, `head_responses_ignore_declared_body` |
| 9110-redirects | 15.4 | SHOULD | client | Handle common redirect status codes and `Location` resolution. | implemented | `src/request.rs`, `src/client.rs`, `src/url/models.rs` | `helpers_follow_redirects`, `request_can_follow_redirects_when_configured`, `redirect_limit_is_enforced` |
| 9110-range-get | 14 | MAY | client | Allow byte-range requests and parse partial responses. | implemented | `src/request.rs`, `src/response.rs` | `range_requests_send_range_headers_and_parse_partial_content` |
| 9110-auth-challenge | 11.3, 11.6, 11.7 | MUST | client | Parse `WWW-Authenticate` and `Proxy-Authenticate` challenge syntax. | implemented | `src/auth.rs`, `src/response.rs` | `parses_multiple_challenges_in_one_field`, `malformed_www_authenticate_returns_an_error`, `malformed_proxy_authenticate_returns_an_error` |
| 9110-auth-retry-origin | 11.6 | SHOULD | client | Retry origin requests after `401` when the client can satisfy a challenge. | implemented | `src/client.rs`, `src/auth.rs` | `basic_auth_retries_on_401`, `generic_auth_handler_retries_on_401`, `repeated_401_returns_the_final_response_without_looping` |
| 9110-auth-retry-proxy | 11.7 | SHOULD | client | Retry proxy requests after `407` when the client can satisfy a challenge. | implemented | `src/client.rs`, `src/auth.rs` | `basic_proxy_auth_retries_on_407`, `generic_proxy_auth_handler_retries_on_407`, `repeated_407_returns_the_final_response_without_looping` |
| 9110-auth-scope | 11.6, 11.7, 15.4 | SHOULD | client | Keep origin and proxy credentials scoped correctly across redirects and proxies. | implemented | `src/client.rs` | `same_authority_redirects_preserve_origin_auth`, `cross_authority_redirects_strip_origin_auth`, `origin_authorization_is_not_sent_on_connect_requests`, `connect_proxy_auth_retries_before_starting_tls` |
| 9110-noncore-auth | 11 | MAY | client | Implement concrete non-Basic auth schemes. | out of scope | `src/auth.rs` | n/a |

## RFC 9111

| ID | Section | Level | Applicability | Summary | Status | Code | Tests |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 9111-store-cacheable | 3, 4 | MUST | client cache | Cache only cacheable responses and honor `no-store`. | implemented | `src/client.rs` | `memory_cache_serves_fresh_get_responses`, `no_store_requests_do_not_populate_the_cache`, `authorization_requests_are_not_cached_without_explicit_cacheability` |
| 9111-vary | 4.1 | MUST | client cache | Match `Vary`-selected request headers when reusing a stored response. | implemented | `src/client.rs` | `vary_headers_create_distinct_cache_variants` |
| 9111-age | 4.2.3 | MUST | client cache | Use current-age calculations, including `Age`, when deciding freshness. | implemented | `src/client.rs` | `age_header_can_make_cached_entries_unsatisfiable_immediately` |
| 9111-request-directives | 5.2.1 | MUST | client cache | Honor request directives such as `max-age`, `min-fresh`, `max-stale`, `only-if-cached`, `no-cache`, and `no-store`. | implemented | `src/client.rs` | `request_max_age_zero_forces_revalidation`, `max_stale_allows_serving_stale_cached_responses`, `only_if_cached_returns_504_when_the_cache_cannot_satisfy_the_request`, `request_no_cache_forces_revalidation_of_fresh_entries` |
| 9111-response-directives | 5.2.2 | MUST | client cache | Honor response directives such as `max-age`, `must-revalidate`, `proxy-revalidate`, `no-cache`, `no-store`, `public`, and `private`. | implemented | `src/client.rs` | `must_revalidate_blocks_max_stale_cache_reuse`, `private_authenticated_responses_are_cached_for_the_same_auth_context` |
| 9111-validation | 4.3 | MUST | client cache | Revalidate stale responses with validators when available. | implemented | `src/client.rs`, `src/request.rs` | `stale_cache_entries_revalidate_with_etags`, `stale_cache_entries_revalidate_with_last_modified` |
| 9111-head-update | 4.3.5 | SHOULD | client cache | Allow `HEAD` to refresh cached metadata for a stored `GET` representation. | implemented | `src/client.rs` | `head_updates_cached_get_metadata` |
| 9111-auth-aware | 3.5 | SHOULD | client cache | Reuse auth-bearing responses conservatively. | implemented | `src/client.rs` | `authorization_requests_are_not_cached_without_explicit_cacheability`, `private_authenticated_responses_are_cached_for_the_same_auth_context` |
| 9111-partial-combination | 3.4 | MAY | client cache | Combine partial responses or store `206` variants. | out of scope | `src/client.rs` | n/a |

## RFC 9112

| ID | Section | Level | Applicability | Summary | Status | Code | Tests |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 9112-request-line | 3, 3.2 | MUST | client | Serialize `GET`, `HEAD`, absolute-form proxy requests, and authority-form `CONNECT` correctly. | implemented | `src/http.rs`, `src/client.rs`, `src/url/models.rs` | `serializes_get_requests`, `serializes_head_requests`, `serializes_absolute_form_targets`, `serializes_connect_requests`, `http_proxy_requests_use_absolute_form_targets`, `https_requests_can_tunnel_through_http_proxies` |
| 9112-host | 3.2 | MUST | client | Send `Host` based on the effective request authority. | implemented | `src/request.rs`, `src/http.rs` | `default_headers_include_host`, `managed_headers_cannot_be_overridden` |
| 9112-framing | 6, 6.1, 6.2, 7.1 | MUST | client | Parse body framing using `Content-Length`, chunked transfer coding, or close-delimiting. | implemented | `src/response.rs` | `parses_content_length_response`, `parses_chunked_responses_and_trailers`, `parses_connection_close_bodies`, `eof_delimited_bodies_are_supported` |
| 9112-invalid-framing | 6.3, 6.4 | MUST | client | Reject malformed status lines, malformed headers, invalid duplicate `Content-Length`, and unsupported transfer codings. | implemented | `src/response.rs` | `rejects_invalid_status_lines`, `rejects_malformed_headers`, `rejects_mismatched_duplicate_content_lengths`, `rejects_unsupported_transfer_encodings`, `rejects_invalid_chunk_sizes` |
| 9112-interim | 15 | MUST | client | Ignore interim `1xx` responses until a final response arrives, except protocol switch handling. | implemented | `src/response.rs` | `skips_interim_responses_but_not_switching_protocols` |
| 9112-persistence | 9.3 | SHOULD | client | Reuse persistent connections when allowed and close them when the peer requires it. | implemented | `src/client.rs`, `src/response.rs` | `session_reuses_keep_alive_connections`, `keep_alive_is_honored_for_http_10` |
| 9112-pipelining | 9.3.2 | MAY | client | Pipeline safe requests over a persistent connection and preserve response order. | implemented | `src/client.rs` | `session_supports_pipelined_get_requests` |
| 9112-hop-by-hop | 7.6.1 | MUST | client | Keep hop-by-hop and protocol-managed headers under client control. | implemented | `src/request.rs`, `src/client.rs` | `rejects_protocol_managed_headers`, `rejects_hop_by_hop_headers`, `origin_authorization_is_not_sent_on_connect_requests` |

## Out-of-Scope Summary

The following are intentionally excluded from the crate's RFC-compliance claim for `v0.3.0`:

- non-core auth scheme implementations beyond the generic challenge framework plus built-in Basic
- cookies
- content-coding decompression
- `206 Partial Content` cache storage or combination
- HTTP/2 and HTTP/3
- asynchronous APIs
