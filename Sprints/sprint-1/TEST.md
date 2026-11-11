# Sprint 1 – Test & fix report

## How it was tested

| Layer | Where | Count |
|-------|-------|------:|
| Unit tests (model, variables, cookies, auth vectors, curl parser, codegen, store, import) | `crates/core/src/**` | 24 |
| HTTP integration tests against the built-in echo server | `crates/core/tests/http.rs` | 5 |
| CLI end-to-end tests driving the real binary | `crates/cli/tests/cli.rs` | 3 (≈90 assertions) |

Run with `cargo test`. Result at the end of the sprint: **32 passed, 0 failed**, `cargo clippy --all-targets` clean.

Reference vectors used: AWS SigV4 "GET iam ListUsers" documentation example, RFC 2617 digest
example (`response="6629fae4…"`), the jwt.io HS256 sample token.

## Defects found and fixed during the sprint

| # | Found by | Problem | Fix |
|---|----------|---------|-----|
| 1 | unit test | Digest test compared against a mis-typed HA1 constant; additionally the client nonce was random so no real vector could be checked | `digest_header_with(.., cnonce)` so the RFC 2617 response can be asserted exactly |
| 2 | integration test | Requests with non-ASCII header values were dropped by the echo server (tiny_http only accepts ASCII headers) — the client was correct | Tests use ASCII header values; non-ASCII is still covered for query, path and bodies |
| 3 | CLI test | Wrong history index in the test itself | Test fixed |
| 4 | review | form-data rows carried a meaningless content-type expression | Removed; content type is derived from the file extension |

## Known limitations carried forward

* Auth helpers not implemented: NTLM, Hawk, Akamai EdgeGrid, ASAP. Selecting an unknown type is an
  error rather than a silently unauthenticated request.
* OAuth 2 token retrieval covers the non-interactive grants (client credentials, password,
  refresh token); browser based grants need the token pasted in.
* HTTP/3 is not supported (HTTP/1.1 and HTTP/2 are).
