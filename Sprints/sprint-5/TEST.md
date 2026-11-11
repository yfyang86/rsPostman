# Sprint 5 – Test & fix report

## Regression run

| Layer | Count | Result |
|-------|------:|--------|
| Rust unit tests (`crates/core`) – +5 secrets, +1 file permissions, +2 benchmark / profile | 49 | pass |
| HTTP integration tests | 5 | pass |
| CLI end-to-end tests – new `tests/secrets.rs` (4 tests: history / output redaction, credential sources and secret variables, report redaction, `benchmark` + `profile`) | 9 | pass |
| Playwright GUI tests – new `secrets.spec.js` (5), +2 in `perf.spec.js` | 34 | pass |
| `cargo clippy --all-targets -- -D warnings`, desktop crate build | – | clean |

## Defects found by the new tests while implementing

| # | Found by | Problem | Fix |
|---|----------|---------|-----|
| 1 | CLI test | Run reports still contained a token: the *response's* final URL and redirect chain were not redacted | `redact_response` covers `url`, `redirects` and `Location` |
| 2 | CLI test | Giving `benchmark --quiet` the short flag `-q` collided with `-q/--query` and made the command panic at start-up | Reverted to the long flag only |
| 3 | unit test | An empty event stream was classified as "not a stream" because detection relied on seeing a `data:` line | Detection also uses the `text/event-stream` content type |
| 4 | review | `key` as a sensitive name redacted ordinary JSON fields such as `{"key": "color"}` | `key` is only treated as a credential in URL query strings (`?key=…`) |

## Behaviour changes to be aware of

* `rspostman history replay` of an entry that was sent with literal credentials now uses the
  saved request when there is one, and otherwise stops with an explanation. Entries that used
  `{{variables}}` replay as before.
* `-u USER` (no colon) used to mean an empty password; it now asks for one. Use `-u USER:` for
  an empty password.
* `benchmark` exits with status 1 when any response was HTTP 4xx/5xx (previously only on
  transport errors), and its latency figures are lower than before for HTTPS targets because
  connections are reused.
* Existing workspaces keep their directory permissions; files become `0600` the next time
  they are written. Existing history entries are not rewritten – run
  `rspostman history clear` to drop credentials recorded by earlier versions.

## Limits of the redaction (also in the README)

Name-based: credentials in unusually named fields, free-text bodies or URL paths are not
recognised. Collections, environments and the cookie jar themselves remain plain text.
The echo `/llm` endpoint sends its whole stream at once, so profiler timings against it say
nothing about real streaming behaviour; only the bookkeeping is tested.
