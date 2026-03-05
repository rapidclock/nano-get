# GET/HEAD Compliance Requirement-to-Test Index

This index maps each requirement ID from
`docs/compliance/http11-get-head-rfc-matrix.md` to one or more tests.

`implemented` rows must map to concrete tests. `not applicable` rows are marked `n/a`.

| ID | Tests |
| --- | --- |
| 9110-9.3.1-get | `get_returns_text_body`, `get_bytes_returns_binary_body` |
| 9110-9.3.2-head | `head_returns_metadata_and_empty_body` |
| 9110-9.3.2-head-no-body | `head_responses_ignore_declared_body`, `head_returns_metadata_and_empty_body` |
| 9110-13.1-if-range | `if_range_requires_range_header`, `if_range_rejects_weak_etags`, `if_range_mismatch_with_only_if_cached_returns_504` |
| 9110-14-range-requests | `range_requests_send_range_headers_and_parse_partial_content`, `memory_cache_stores_206_segments_and_promotes_to_full_entry` |
| 9110-15.4-redirects | `helpers_follow_redirects`, `request_can_follow_redirects_when_configured`, `redirect_limit_is_enforced` |
| 9110-11-auth-challenge-parse | `parses_multiple_challenges_in_one_field`, `malformed_www_authenticate_returns_an_error`, `malformed_proxy_authenticate_returns_an_error` |
| 9110-11-origin-auth-retry | `basic_auth_retries_on_401`, `generic_auth_handler_retries_on_401`, `repeated_401_returns_the_final_response_without_looping` |
| 9110-11-proxy-auth-retry | `basic_proxy_auth_retries_on_407`, `generic_proxy_auth_handler_retries_on_407`, `repeated_407_returns_the_final_response_without_looping` |
| 9110-11-auth-scope | `same_authority_redirects_preserve_origin_auth`, `cross_authority_redirects_strip_origin_auth`, `origin_authorization_is_not_sent_on_connect_requests` |
| 9110-methods-non-get-head | n/a |
| 9110-server-semantics | n/a |
| 9111-3-store-cacheable | `memory_cache_serves_fresh_get_responses`, `no_store_requests_do_not_populate_the_cache` |
| 9111-3.4-store-206 | `partial_responses_are_combined_into_a_cacheable_full_representation`, `memory_cache_stores_206_segments_and_promotes_to_full_entry` |
| 9111-3.4-combine-206 | `partial_responses_are_combined_into_a_cacheable_full_representation`, `if_range_mismatch_with_only_if_cached_returns_504` |
| 9111-4.1-vary | `vary_headers_create_distinct_cache_variants` |
| 9111-4.2-freshness-lifetime | `request_max_age_zero_forces_revalidation`, `max_stale_allows_serving_stale_cached_responses` |
| 9111-4.2.3-current-age | `age_header_can_make_cached_entries_unsatisfiable_immediately` |
| 9111-4.3-validation | `stale_cache_entries_revalidate_with_etags`, `stale_cache_entries_revalidate_with_last_modified` |
| 9111-4.3.4-merge-304 | `stale_cache_entries_revalidate_with_etags`, `request_no_cache_forces_revalidation_of_fresh_entries` |
| 9111-4.3.5-head-refresh | `head_updates_cached_get_metadata` |
| 9111-5.2.1-request-directives | `request_max_age_zero_forces_revalidation`, `max_stale_allows_serving_stale_cached_responses`, `request_no_cache_forces_revalidation_of_fresh_entries`, `only_if_cached_returns_504_when_the_cache_cannot_satisfy_the_request` |
| 9111-5.2.2-response-directives | `must_revalidate_blocks_max_stale_cache_reuse`, `private_authenticated_responses_are_cached_for_the_same_auth_context`, `no_store_requests_do_not_populate_the_cache` |
| 9111-5.2.1.7-only-if-cached | `only_if_cached_returns_504_when_the_cache_cannot_satisfy_the_request`, `execute_one_only_if_cached_without_memory_cache_returns_gateway_timeout` |
| 9111-3.5-authenticated-storage | `authorization_requests_are_not_cached_without_explicit_cacheability`, `private_authenticated_responses_are_cached_for_the_same_auth_context` |
| 9111-shared-cache-requirements | n/a |
| 9112-3.2-request-target-forms | `serializes_get_requests`, `serializes_head_requests`, `serializes_absolute_form_targets`, `serializes_connect_requests`, `http_proxy_requests_use_absolute_form_targets`, `https_requests_can_tunnel_through_http_proxies` |
| 9112-3.2-host-header | `default_headers_include_host`, `managed_headers_cannot_be_overridden` |
| 9112-6-message-framing | `parses_content_length_response`, `parses_chunked_responses_and_trailers`, `parses_connection_close_bodies` |
| 9112-6.1-transfer-encoding | `rejects_unsupported_transfer_encodings`, `strict_mode_rejects_transfer_encoding_with_content_length` |
| 9112-6.2-content-length | `accepts_duplicate_content_lengths_with_equal_numeric_values`, `rejects_mismatched_duplicate_content_lengths`, `rejects_non_numeric_content_lengths` |
| 9112-6.3-invalid-framing-rejection | `rejects_invalid_status_lines`, `rejects_malformed_headers`, `rejects_invalid_chunk_sizes`, `strict_header_parsing_rejects_lf_only_and_obs_fold` |
| 9112-6.3-close-delimited-body | `eof_delimited_bodies_are_supported`, `parses_connection_close_bodies` |
| 9112-15-interim-responses | `read_response_head_skips_interim_responses`, `skips_interim_responses_but_not_switching_protocols` |
| 9112-9.3-persistent-connections | `session_reuses_keep_alive_connections`, `keep_alive_is_honored_for_http_10`, `execute_pipelined_handles_connection_close_on_final_response` |
| 9112-9.3.2-pipelining | `session_supports_pipelined_get_requests`, `execute_pipelined_validates_requests_before_network_io` |
| 9112-9.3.1-retry-unanswered-safe | `pipelining_retries_unanswered_requests_after_premature_close` |
| 9112-7.6.1-hop-by-hop | `rejects_protocol_managed_headers`, `rejects_hop_by_hop_headers`, `origin_authorization_is_not_sent_on_connect_requests` |
| 9112-server-only-behavior | n/a |
