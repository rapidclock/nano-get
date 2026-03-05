# Full HTTP/1.1 GET/HEAD Client Compliance Checklist

This checklist tracks what is still needed to move from the current scoped claim to a strict
"full client-side HTTP/1.1 GET/HEAD" claim.

Current scoped matrix:

- `docs/compliance/http11-get-head-rfc-matrix.md`

## A. Make the requirements inventory exhaustive

- [ ] Convert the current matrix from a selected-requirements view into an exhaustive list of all
      client-applicable RFC 9110 / 9111 / 9112 requirements for a GET/HEAD user agent.
- [ ] Record each applicable item with one status:
      - `implemented`
      - `not applicable`
      - `out of scope` (only for features explicitly excluded from the claim)
- [ ] Add exact RFC section references for every row.
- [ ] Add code and test traceability for every `implemented` row.
- [ ] Ensure every client-applicable MUST requirement is either `implemented` or explicitly removed
      from the advertised claim boundary.

## B. Close remaining core behavior gaps for an "entirety" claim

These are currently documented as out of scope in the matrix and are the main technical blockers
for an absolute "entire GET/HEAD HTTP/1.1 client-side behavior" statement.

- [ ] RFC 9111 section 3.4:
      - store cacheable `206 Partial Content` responses when valid for cache storage
      - combine partial responses into complete representations when requirements are met
- [ ] Add range/cache interaction coverage:
      - `If-Range` revalidation paths
      - multi-range and partial-response merge eligibility checks
      - failure/safety cases where partial content must not be combined

## C. Raise confidence from "works" to "conformance-grade"

- [ ] Add a dedicated compliance test index mapping each matrix requirement ID to one or more tests.
- [ ] Add malformed-wire corpus tests for parser robustness:
      - status line edge cases
      - header syntax edge cases
      - chunked framing edge cases
      - duplicate framing header edge cases
- [ ] Add long-lived connection edge tests for persistence and pipelining:
      - premature close handling
      - retry safety boundaries
      - partial read / EOF timing cases
- [ ] Add proxy conformance edge tests:
      - absolute-form forwarding
      - CONNECT tunnel setup with auth retries
      - credential scoping across redirect/proxy boundaries

## D. Lock the compliance claim

- [ ] Update the matrix and README so the claim text is identical in both places.
- [ ] Add a short "compliance scope" section in docs.rs/lib docs that links to the matrix.
- [ ] Add CI checks to prevent compliance drift:
      - matrix file presence
      - requirement IDs referenced by tests
- [ ] Perform one release-readiness pass where each checklist item is signed off with concrete
      evidence (test names + code paths).

## E. Optional/adjacent capabilities (not required for core HTTP/1.1 claim)

These are often expected in production clients but are separate from core HTTP/1.1 GET/HEAD
conformance.

- [ ] Non-core auth scheme implementations (Digest, Bearer semantics, Negotiate, NTLM)
- [ ] Cookie support (RFC 6265 family)
- [ ] Content-coding decompression (`gzip`, `br`, `deflate`)
- [ ] HTTP/2 and HTTP/3 protocol support
- [ ] Async API surface
