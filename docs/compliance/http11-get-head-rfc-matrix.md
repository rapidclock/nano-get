# HTTP/1.1 GET/HEAD RFC Matrix

This matrix is an exhaustive inventory of client-applicable requirements for nano-get's HTTP/1.1
`GET`/`HEAD` compliance claim across:

- RFC 9110 (HTTP Semantics)
- RFC 9111 (HTTP Caching)
- RFC 9112 (HTTP/1.1)

Claim boundary:

- Included: user-agent behavior for `GET`/`HEAD`, redirects, range requests, validator handling,
  private in-memory caching (including `206` storage/combination), HTTP proxy forwarding,
  HTTPS-over-`CONNECT`, and parser/framing safety.
- Not applicable: server-origin requirements, intermediary-only shared-cache requirements, and
  method semantics outside `GET`/`HEAD`.

Status values:

- `implemented`
- `not applicable`

## RFC 9110 (Semantics)

| ID | Section | Level | Status | Summary | Code | Tests |
| --- | --- | --- | --- | --- | --- | --- |
| 9110-9.3.1-get | 9.3.1 | MUST | implemented | Support `GET` request semantics. | `src/request.rs`, `src/client.rs` | `get_returns_text_body`, `get_bytes_returns_binary_body` |
| 9110-9.3.2-head | 9.3.2 | MUST | implemented | Support `HEAD` request semantics. | `src/request.rs`, `src/client.rs` | `head_returns_metadata_and_empty_body` |
| 9110-9.3.2-head-no-body | 9.3.2 | MUST | implemented | Treat `HEAD` responses as bodyless regardless of framing headers. | `src/response.rs`, `src/client.rs` | `head_responses_ignore_declared_body`, `head_returns_metadata_and_empty_body`, `head_responses_with_illegal_body_bytes_do_not_poison_reused_connections` |
| 9110-13.1-if-range | 13.1, 13.1.5 | MUST | implemented | Enforce safe `If-Range` request combinations and matching behavior. | `src/request.rs`, `src/client.rs` | `if_range_requires_range_header`, `if_range_rejects_weak_etags`, `if_range_mismatch_with_only_if_cached_returns_504` |
| 9110-14-range-requests | 14 | MAY | implemented | Support byte range requests and partial response handling for `GET`. | `src/request.rs`, `src/client.rs`, `src/response.rs` | `range_requests_send_range_headers_and_parse_partial_content`, `memory_cache_stores_206_segments_and_promotes_to_full_entry` |
| 9110-15.4-redirects | 15.4 | SHOULD | implemented | Follow redirects when configured and resolve `Location` targets correctly. | `src/client.rs`, `src/url/models.rs` | `helpers_follow_redirects`, `request_can_follow_redirects_when_configured`, `redirect_limit_is_enforced`, `resolve_treats_scheme_colon_prefix_as_absolute_uri_reference` |
| 9110-11-auth-challenge-parse | 11.3, 11.6, 11.7 | MUST | implemented | Parse `WWW-Authenticate` and `Proxy-Authenticate` challenges. | `src/auth.rs`, `src/response.rs` | `parses_multiple_challenges_in_one_field`, `malformed_www_authenticate_returns_an_error`, `malformed_proxy_authenticate_returns_an_error` |
| 9110-11-origin-auth-retry | 11.6 | SHOULD | implemented | Retry origin requests after `401` when a configured handler can satisfy challenges. | `src/client.rs`, `src/auth.rs` | `basic_auth_retries_on_401`, `generic_auth_handler_retries_on_401`, `repeated_401_returns_the_final_response_without_looping` |
| 9110-11-proxy-auth-retry | 11.7 | SHOULD | implemented | Retry proxy requests/tunnels after `407` when a configured handler can satisfy challenges. | `src/client.rs`, `src/auth.rs` | `basic_proxy_auth_retries_on_407`, `generic_proxy_auth_handler_retries_on_407`, `repeated_407_returns_the_final_response_without_looping` |
| 9110-11-auth-scope | 11.6, 11.7, 15.4 | SHOULD | implemented | Keep origin/proxy credentials scoped to the correct authority and forwarding path. | `src/client.rs` | `same_authority_redirects_preserve_origin_auth`, `cross_authority_redirects_strip_origin_auth`, `origin_authorization_is_not_sent_on_connect_requests` |
| 9110-methods-non-get-head | 9.3 | MUST | not applicable | Semantics for methods other than `GET`/`HEAD`. | n/a | n/a |
| 9110-server-semantics | 6, 8, 10, 15 | MUST | not applicable | Origin-server representation generation and response-production duties. | n/a | n/a |

## RFC 9111 (Caching)

| ID | Section | Level | Status | Summary | Code | Tests |
| --- | --- | --- | --- | --- | --- | --- |
| 9111-3-store-cacheable | 3, 4 | MUST | implemented | Store only cacheable responses and honor request/response `no-store`. | `src/client.rs` | `memory_cache_serves_fresh_get_responses`, `no_store_requests_do_not_populate_the_cache` |
| 9111-3.4-store-206 | 3.4 | MAY | implemented | Store cacheable `206 Partial Content` responses in the cache model. | `src/client.rs` | `partial_responses_are_combined_into_a_cacheable_full_representation`, `memory_cache_stores_206_segments_and_promotes_to_full_entry` |
| 9111-3.4-combine-206 | 3.4 | MAY | implemented | Combine partial ranges only when validator/range preconditions are satisfied. | `src/client.rs` | `partial_responses_are_combined_into_a_cacheable_full_representation`, `if_range_mismatch_with_only_if_cached_returns_504` |
| 9111-4.1-vary | 4.1 | MUST | implemented | Use `Vary` to select cache variants. | `src/client.rs` | `vary_headers_create_distinct_cache_variants` |
| 9111-4.2-freshness-lifetime | 4.2.1, 4.2.2 | MUST | implemented | Compute freshness lifetime from explicit directives and heuristic fallback rules. | `src/client.rs`, `src/date.rs` | `request_max_age_zero_forces_revalidation`, `max_stale_allows_serving_stale_cached_responses` |
| 9111-4.2.3-current-age | 4.2.3 | MUST | implemented | Compute current age including apparent age/`Age` contributions. | `src/client.rs` | `age_header_can_make_cached_entries_unsatisfiable_immediately` |
| 9111-4.3-validation | 4.3 | MUST | implemented | Revalidate stale entries with validators and preserve conditional semantics. | `src/client.rs`, `src/request.rs` | `stale_cache_entries_revalidate_with_etags`, `stale_cache_entries_revalidate_with_last_modified` |
| 9111-4.3.4-merge-304 | 4.3.4 | MUST | implemented | Merge `304 Not Modified` metadata into selected stored responses safely. | `src/client.rs` | `stale_cache_entries_revalidate_with_etags`, `request_no_cache_forces_revalidation_of_fresh_entries` |
| 9111-4.3.5-head-refresh | 4.3.5 | SHOULD | implemented | Use compatible `HEAD` responses to refresh stored `GET` metadata. | `src/client.rs` | `head_updates_cached_get_metadata` |
| 9111-5.2.1-request-directives | 5.2.1 | MUST | implemented | Honor supported request directives (`max-age`, `min-fresh`, `max-stale`, `no-cache`, `no-store`, `only-if-cached`). | `src/client.rs` | `request_max_age_zero_forces_revalidation`, `max_stale_allows_serving_stale_cached_responses`, `request_no_cache_forces_revalidation_of_fresh_entries`, `only_if_cached_returns_504_when_the_cache_cannot_satisfy_the_request` |
| 9111-5.2.2-response-directives | 5.2.2 | MUST | implemented | Honor supported response directives (`max-age`, `must-revalidate`, `proxy-revalidate`, `no-cache`, `no-store`, `public`, `private`). | `src/client.rs` | `must_revalidate_blocks_max_stale_cache_reuse`, `private_authenticated_responses_are_cached_for_the_same_auth_context`, `no_store_requests_do_not_populate_the_cache` |
| 9111-5.2.1.7-only-if-cached | 5.2.1.7 | MUST | implemented | Return a gateway-timeout style response when `only-if-cached` cannot be satisfied. | `src/client.rs` | `only_if_cached_returns_504_when_the_cache_cannot_satisfy_the_request`, `execute_one_only_if_cached_without_memory_cache_returns_gateway_timeout` |
| 9111-3.5-authenticated-storage | 3.5 | MUST | implemented | Apply conservative rules for authenticated response storage/reuse. | `src/client.rs` | `authorization_requests_are_not_cached_without_explicit_cacheability`, `private_authenticated_responses_are_cached_for_the_same_auth_context` |
| 9111-shared-cache-requirements | 3.5, 5.2.2.10 | MUST | not applicable | Shared-cache-only obligations (e.g., `s-maxage` behavior). | n/a | n/a |

## RFC 9112 (HTTP/1.1 Message Syntax and Routing)

| ID | Section | Level | Status | Summary | Code | Tests |
| --- | --- | --- | --- | --- | --- | --- |
| 9112-3.2-request-target-forms | 3.2 | MUST | implemented | Serialize origin-form, absolute-form (proxy), and authority-form (`CONNECT`) targets correctly. | `src/http.rs`, `src/client.rs`, `src/url/models.rs` | `serializes_get_requests`, `serializes_head_requests`, `serializes_absolute_form_targets`, `serializes_connect_requests`, `http_proxy_requests_use_absolute_form_targets`, `https_requests_can_tunnel_through_http_proxies` |
| 9112-3.2-host-header | 3.2 | MUST | implemented | Send and protect protocol-managed `Host` authority data. | `src/request.rs`, `src/http.rs` | `default_headers_include_host`, `managed_headers_cannot_be_overridden` |
| 9112-6-message-framing | 6 | MUST | implemented | Parse response bodies using valid HTTP/1.1 framing rules. | `src/response.rs` | `parses_content_length_response`, `parses_chunked_responses_and_trailers`, `parses_connection_close_bodies` |
| 9112-6.1-transfer-encoding | 6.1 | MUST | implemented | Enforce supported transfer-coding behavior and reject invalid transfer coding combinations. | `src/response.rs` | `rejects_unsupported_transfer_encodings`, `strict_mode_rejects_transfer_encoding_with_content_length` |
| 9112-6.2-content-length | 6.2 | MUST | implemented | Handle duplicate/invalid `Content-Length` according to framing safety rules. | `src/response.rs` | `accepts_duplicate_content_lengths_with_equal_numeric_values`, `rejects_mismatched_duplicate_content_lengths`, `rejects_non_numeric_content_lengths` |
| 9112-6.3-invalid-framing-rejection | 6.3, 6.4 | MUST | implemented | Reject malformed status lines, headers, and chunked wire syntax. | `src/response.rs` | `rejects_invalid_status_lines`, `rejects_malformed_headers`, `rejects_invalid_chunk_sizes`, `strict_header_parsing_rejects_lf_only_and_obs_fold`, `rejects_excessive_header_count`, `rejects_overly_long_lines` |
| 9112-6.3-close-delimited-body | 6.3 | MUST | implemented | Support close-delimited response bodies when framing requires it. | `src/response.rs` | `eof_delimited_bodies_are_supported`, `parses_connection_close_bodies` |
| 9112-15-interim-responses | 15 | MUST | implemented | Skip interim `1xx` responses until a final response is received. | `src/http.rs`, `src/response.rs` | `read_response_head_skips_interim_responses`, `skips_interim_responses_but_not_switching_protocols` |
| 9112-9.3-persistent-connections | 9.3 | SHOULD | implemented | Reuse persistent connections and honor peer closure signals. | `src/client.rs`, `src/response.rs` | `session_reuses_keep_alive_connections`, `session_retries_when_reused_connection_is_stale_closed`, `head_responses_with_illegal_body_bytes_do_not_poison_reused_connections`, `keep_alive_is_honored_for_http_10`, `execute_pipelined_handles_connection_close_on_final_response` |
| 9112-9.3.2-pipelining | 9.3.2 | MAY | implemented | Pipeline safe requests while preserving in-order response handling. | `src/client.rs` | `session_supports_pipelined_get_requests`, `execute_pipelined_validates_requests_before_network_io` |
| 9112-9.3.1-retry-unanswered-safe | 9.3.1, 9.3.2 | SHOULD | implemented | Retry unanswered safe requests after premature close without replaying answered requests. | `src/client.rs` | `pipelining_retries_unanswered_requests_after_premature_close`, `pipelining_retries_unanswered_requests_when_peer_closes_without_connection_close_header` |
| 9112-7.6.1-hop-by-hop | 7.6.1 | MUST | implemented | Keep hop-by-hop/protocol-managed header fields under implementation control. | `src/request.rs`, `src/client.rs` | `rejects_protocol_managed_headers`, `rejects_hop_by_hop_headers`, `origin_authorization_is_not_sent_on_connect_requests` |
| 9112-server-only-behavior | 2, 6, 9 | MUST | not applicable | Sender/intermediary server obligations outside client-user-agent behavior. | n/a | n/a |
