# Changelog

All notable changes to rsPostman are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.2.0] - 2026-11-11

Release notes: [docs/releases/v0.2.0.md](docs/releases/v0.2.0.md)

### Added

- **Local vault** for secrets encrypted at rest (Argon2id key derivation,
  XChaCha20-Poly1305 per value) in `vault.json`, referenced from requests as
  `{{vault:name}}`. Available from the CLI (`rspostman vault …`) and a Vault
  screen in the GUI. Passphrases can be supplied through
  `RSPOSTMAN_VAULT_PASSPHRASE` / `RSPOSTMAN_VAULT_NEW_PASSPHRASE`.
- **Benchmark**: run by duration or iteration count, target request rate,
  warm-up, keep-alive toggle, time to first byte, latency percentiles
  (p50/p75/p90/p95/p99/p99.9), latency histogram and timeline, pass/fail
  thresholds, comparison against a saved baseline, and CSV / HTML reports.
  The GUI shows live charts while a run is in progress and can cancel it.
- **LLM provider profiler** (CLI and GUI) measuring latency and streaming
  behaviour of chat-completion style endpoints.
- **User manual** (`docs/user-manual.md`) with screenshots, and an in-app
  **Help Center** (toolbar button or `F1`) with topic list, search and
  context help buttons on each tool screen.
- `scripts/screenshots.js` regenerates every manual screenshot from a seeded
  workspace.
- `history_store_secrets` setting (off by default) to control whether
  credentials are kept in history.
- `RSPOSTMAN_LOG=1` access log and `RSPOSTMAN_UI_DIR` (serve the UI from
  disk) for the GUI server.

### Changed

- Credentials are masked in the GUI, prompted for without echo in the CLI,
  and redacted (`<redacted>`) from history, run reports and verbose
  output, including response URLs and redirect chains.
- Workspace files are written with mode `0600` and directories with `0700`.
- The GUI server requires the `X-Requested-With: rspostman` header on
  non-GET requests in addition to the loopback and `Host` checks.
- The toolbar collapses to icons on narrow windows instead of overflowing.

### Fixed

- Latency histogram collapsing into a single bucket for tightly grouped
  samples.
- Throughput regressions being reported for runs too short to measure them.
- `benchmark --quiet` colliding with another `-q` short flag.
- Server-sent event streams are now detected by content type.

### Security

- Vault values never appear in history, reports or logs: they are scrubbed
  by value as well as by name.
- Links using `javascript:` or `data:` schemes in Help Center documents are
  rendered as plain text.

## [0.1.0] - initial release

### Added

- Core engine and `rspostman` CLI: collections (Postman Collection v2.1),
  environments and globals, variable scopes and dynamic variables, HTTP
  requests with all common body types, authorization helpers, cookies,
  history, curl import and code generation.
- `pm.*` scripting sandbox for pre-request and test scripts, collection
  runner with data files and reporters, mock server, monitors, importers
  (Postman, OpenAPI, HAR, curl) and WebSocket client.
- GUI: local API server with an embedded web UI, plus a Tauri desktop shell.
- Playwright end-to-end tests for the GUI.

[0.2.0]: https://github.com/yfyang86/rsPostman/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/yfyang86/rsPostman/releases/tag/v0.1.0
