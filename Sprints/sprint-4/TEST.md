# Sprint 4 – Test & fix report (final)

## Final regression run

| Layer | Where | Count | Result |
|-------|-------|------:|--------|
| Rust unit tests | `crates/core/src/**` | 38 | pass |
| HTTP integration tests | `crates/core/tests/http.rs` | 5 | pass |
| CLI end-to-end tests (real binary) | `crates/cli/tests/*.rs` | 5 | pass |
| Playwright GUI tests (Chromium, real binary) | `e2e/*.spec.js` | 23 | pass, repeated parallel runs |
| Lints | `cargo clippy --all-targets -- -D warnings` (incl. desktop crate) | – | clean |
| Desktop shell | `cargo build -p rspostman-desktop`, launched under Xvfb, screenshot + access log checked | – | window renders the UI, `/api/bootstrap` served |
| README | every command in the quick tour executed against the echo server | – | all work as documented |

`scripts/test-all.sh` runs the first five rows.

## Defects found and fixed during the sprint

| # | Found by | Problem | Fix |
|---|----------|---------|-----|
| 1 | Playwright | The runner and mock-server screens kept a stale collection list when a collection was created while they were open | Visible tool screens refresh on collection changes |
| 2 | screenshot review | Toolbar glyphs for "edit environment" and "theme" rendered tiny / inconsistently across fonts | Replaced by inline SVG icons |
| 3 | README verification | README claimed "~70" dynamic variables | Counted: 62; text corrected |

## What is verified how – and what is not

* The GUI behaviour is verified in Chromium. The desktop app uses the system webview
  (WebKitGTK / WebView2 / WKWebView) with the same page; it was built and smoke-tested on
  Linux only. Windows and macOS builds were **not** produced or tested in this environment.
* Downloads inside the desktop webview (exports, reports) are routed to the Downloads folder
  by a handler that compiles and is wired up, but was not exercised by an automated test.
* TLS client certificates, custom CA bundles, SOCKS proxies and HTTP/2 rely on `reqwest` /
  `rustls` and have no dedicated tests here (the test-suite is hermetic and uses plain HTTP on
  loopback).
* OAuth 2 token retrieval and AWS SigV4 are covered by unit tests / reference vectors, not
  against real identity providers or AWS.

See the "Not implemented" section of the README for the feature gaps relative to Postman.
