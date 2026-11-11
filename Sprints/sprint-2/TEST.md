# Sprint 2 – Test & fix report

## How it was tested

| Layer | Where | Count |
|-------|-------|------:|
| Unit tests (cumulative): + sandbox/`pm` API (4 large scenario tests, ~120 assertions inside JS), runner data parsing, mock matching, WebSocket round trip, OpenAPI/Swagger/HAR import, docs | `crates/core/src/**` | 35 |
| HTTP integration | `crates/core/tests/http.rs` | 5 |
| CLI e2e, sprint 1 | `crates/cli/tests/cli.rs` | 3 |
| CLI e2e, sprint 2: runner + data file + reports + flow control + monitor; mock server + WebSocket + docs + OpenAPI | `crates/cli/tests/sprint2.rs` | 2 |

Result: **45 passed, 0 failed**; `cargo clippy --all-targets` clean.
The example collection used by the tests ships in `examples/`.

## Defects found and fixed during the sprint

| # | Found by | Problem | Fix |
|---|----------|---------|-----|
| 1 | CLI e2e | **Data loss risk**: running an exported collection *file* whose `_postman_id` equals a stored collection wrote the file's variables over the stored collection when scripts changed variables | File-loaded collections/environments are always detached (fresh id), so they can never be persisted over stored ones; regression test added |
| 2 | unit test | OpenAPI JSON documents were mis-detected as Postman collections (both have `info`) | OpenAPI/HAR detection now runs before the Postman check |
| 3 | unit test (hang) | WebSocket upgrade requests hung the server: reading the (non-existent) body of an upgrade request blocks until the peer closes | Body is not read for upgrade requests |
| 4 | CLI e2e | WebSocket close was reported as an error ("reset without closing handshake") | Echo server completes the closing handshake; the client treats a drop after its own close frame as a normal close |
| 5 | review | A script with an endless loop took >10 s to be stopped in debug builds | Loop limit lowered to 5 M iterations |

## Known limitations carried forward

* The sandbox is ES2022-ish JavaScript without Node/browser APIs. `require()` only knows
  `uuid` and `chai`; `pm.cookies.jar()` is read-only; `setInterval` is not available.
* `pm.sendRequest` is executed synchronously (callbacks run immediately, promises are already
  settled) – observable ordering can differ from Postman for code relying on interleaving.
* Mock server matches on method + path (+ query as a tie-breaker); request-body matching is
  not implemented.
