# Sprint 5 – Review of `dev`, credential handling, polish

Starting point: `dev` = sprint 4 + the contributed commit "Add LLM provider profiler and
request benchmark (CLI + GUI)".

## Part A – proof-reading the profiler / benchmark contribution

Findings of the review (all addressed in this sprint):

| # | Finding | Severity |
|---|---------|----------|
| A1 | Benchmark built a new HTTP client for every request: each sample included loading the system root certificates and a fresh TCP/TLS handshake, so the percentiles measured connection setup rather than the endpoint | wrong numbers |
| A2 | Responses with HTTP 4xx/5xx counted as successes; a benchmark of a failing endpoint reported "0 errors" and exit code 0 | misleading |
| A3 | GUI percentile table dropped a row whenever its MIN was 0 ms (`rows.filter(r => r[1])`), which is common on localhost | bug |
| A4 | GUI progress stayed at "0/N" until the run finished (the server only counted at the end) | bug |
| A5 | `profile -d <body>` was silently replaced by the generated chat body (only `--json` was respected) | bug |
| A6 | Echo `/llm` had an unreachable "fail" branch; there was no way to exercise the *degraded* verdict | dead code / untestable |
| A7 | A provider that ignores `stream: true` was reported as "empty stream" | unclear diagnosis |
| A8 | Unbounded `--concurrency` (one thread each); `Mutex::lock().unwrap()` could cascade a panic | robustness |
| A9 | URLs with `?api_key=…` were printed and written to reports verbatim; README example put the API key on the command line; the GUI offered only a plain-text header field for the key | credentials |
| A10 | No Rust tests for `benchmark()` / `profile_llm()` and no CLI tests for the two commands | coverage |

## Part B – credential handling

Problem statement: passwords, tokens and keys were shown in clear text in every input, had
to be passed on the command line, and were copied verbatim into the history, run reports
and verbose output; workspace files were world-readable.

| # | Change |
|---|--------|
| B1 | `core::secrets`: one place that knows what a credential is and redacts requests, wire snapshots, responses and URLs. `{{variable}}` references are never redacted |
| B2 | History stores `<redacted>` by default (`history_store_secrets` opts out); replay falls back to the saved request or explains why it cannot |
| B3 | Run / benchmark / profile reports and `-v` output are redacted (`--show-secrets`) |
| B4 | Workspace directories `0700`, files `0600` on Unix, incl. uploads and monitor logs; existing directories are not re-moded |
| B5 | CLI: `-u USER` asks for the password without echo (or `RSPOSTMAN_PASSWORD`, `USER:env:NAME`); `--bearer` / `--api-key` / `--auth-param` accept `env:NAME`, `@FILE`, `-`; `env set` / `globals set` / `collection set-var` can prompt and mark `--secret`; listings mask secret variables |
| B6 | GUI: masked inputs with reveal button for auth secrets, secret variables (auto-marked by name), sensitive header / form values, the profiler's API key; the sent-request view hides credentials until asked; history reopen explains redaction |

## Deliberately not in scope

Encrypting the workspace or using the OS keychain. It needs a decision on key management
(master passphrase vs. keychain, headless/CI behaviour, what happens to Postman-compatible
files) that should be made by the project owner – see `NEXT.md`.
