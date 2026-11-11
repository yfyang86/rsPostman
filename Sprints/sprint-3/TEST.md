# Sprint 3 – Test & fix report

## How it was tested

| Layer | Where | Count |
|-------|-------|------:|
| Rust unit + integration + CLI e2e (cumulative, incl. 3 new HTTP server tests) | `cargo test` | 48 |
| **Playwright** end-to-end tests in Chromium against the real binary | `e2e/*.spec.js` | 16 |

Each Playwright worker starts its own `rspostman gui` and `rspostman echo` on free ports with a
throw-away workspace (`e2e/fixtures.js`). Every test additionally fails on any uncaught page
error or `console.error`. Several tests cross-check the GUI against the CLI (e.g. a request
saved in the GUI is read back with `rspostman collection show`).

Run: `cargo build && cd e2e && playwright test`. Result: **48 + 16 passed**, stable over
repeated parallel runs; `cargo clippy --all-targets` clean.

## Defects found and fixed during the sprint

| # | Found by | Problem | Fix |
|---|----------|---------|-----|
| 1 | Playwright (intermittent 45 s page-load timeouts under parallel load) | **Requests could hang forever.** Traced with access logging and a reproduction script to a race in the `tiny_http` crate's connection thread pool: when a burst of new connections arrives while one pool thread is idle, a connection can be queued without any thread to serve it until another connection shows up. A browser opening 6 connections at once hit it roughly 1 in 3 page loads. | Replaced `tiny_http` with a small purpose-built HTTP/1.1 server in `core::server` (thread per connection, keep-alive, chunked request bodies, `Expect: 100-continue`, WebSocket upgrade). Regression test opens 40 connections before using any of them. Side effects: one dependency less, and non-ASCII header values now work (sprint 1 limitation #2 is gone) |
| 2 | Playwright | Deleting a folder that contains an open request left the tab's breadcrumb claiming it was still saved | Tabs are re-rendered when their item disappears; they become unsaved drafts so no work is lost |
| 3 | screenshot review | Key/value tables rendered the text "null" and misaligned columns | `Element.append(null)` inserts text – rows are now built with the null-safe helper |
| 4 | Playwright | Downloading a binary response suggested the file name `response.txt` | Extension is derived from the content type for non-text bodies |
| 5 | Playwright | Expected user errors (e.g. invalid import) were logged with `console.error` | Logged as warnings; the toast is the user-facing message |

## Known limitations carried forward

* Script editors are plain text areas (no syntax highlighting / autocomplete).
* OAuth 2 browser-based grants are not available; tokens from those flows must be pasted.
* Open tabs are not restored after a reload (saved requests of course are).
