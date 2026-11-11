# Sprint 1 – Core engine + CLI foundation

## Decisions

* **Workspace layout**: Cargo workspace; `crates/core` (library, all logic) and `crates/cli`
  (`rspostman` binary). The GUI (sprint 3/4) is a thin layer on the same core.
* **GUI technology (decided now so the core fits)**: **Tauri v2** shell around a web UI.
  The same UI is also served by `rspostman gui` from an embedded local HTTP server, which is
  what Playwright drives in tests. egui was rejected because a canvas-rendered UI cannot be
  tested with Playwright in a meaningful way.
* **Native data format = Postman Collection v2.1 / Postman environment JSON.** Import/export
  is therefore loss-free for everything we model, and files stay interoperable with Postman
  and Newman.
* **Local only**: everything is stored under a workspace directory (`~/.rspostman`,
  `$RSPOSTMAN_HOME` or `--workspace`). No accounts, no sync.
* Core is synchronous (reqwest blocking + rustls); servers use `tiny_http`.

## Scope

| # | Feature | Acceptance |
|---|---------|------------|
| 1 | Data model: collection, folders, requests, examples, events, variables, auth | Round-trips a Postman v2.1 export; lenient on v2.0 quirks (string URL, string headers, object auth) |
| 2 | Variables: globals < collection < environment < data < local, `{{var}}`, nested, dynamic `{{$guid}}` etc. | Unit tests |
| 3 | HTTP execution: all methods, query/path variables, headers, bodies (raw, urlencoded, form-data incl. files, binary, GraphQL), redirects, timeout, proxy, TLS options, client certs | Integration tests against built-in echo server |
| 4 | Auth: inherit, none, Basic, Bearer, API key, Digest, OAuth 2 (token), OAuth 1, AWS SigV4, JWT bearer | Unit + integration tests |
| 5 | Cookie jar (persisted, domain/path/expiry matching) | Integration test |
| 6 | Local store: collections, environments, globals, history, cookies, settings | Unit tests |
| 7 | Import: Postman collection/environment, curl command. Export: collection/environment | Tests |
| 8 | Code generation: curl, wget, HTTPie, Python, JS fetch, Node axios, Go, Rust, PHP, Java, C#, PowerShell, raw HTTP | Snapshot-ish tests |
| 9 | CLI: `send`, `collection`, `env`, `globals`, `history`, `cookies`, `import`, `export`, `codegen`, `settings`, `echo` | CLI smoke test script |

## Out of scope (later sprints)

Scripts, runner, mock server, monitors, OpenAPI/HAR import, WebSocket (sprint 2); GUI (3, 4).
