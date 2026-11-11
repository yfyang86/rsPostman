# Sprint 2 – Retrospective and plan for sprint 3

## Done

Everything in scope. The CLI is now a Newman-class runner (`rspostman run`) plus mock server,
monitor, importers, docs and WebSocket client.

## Observations

* Writing the `pm` API in JavaScript instead of binding it natively paid off: the whole
  sandbox passed its tests on the first run, and the Rust side is ~300 lines.
* End-to-end tests through the real binary keep finding bugs that unit tests cannot
  (defects 1 and 4). The GUI needs the same treatment → Playwright from day one.

## Sprint 3 plan (summary – details in `Sprints/sprint-3/PLAN.md`)

1. `crates/server`: local JSON API over the core (collections, environments, send, history,
   cookies, settings, import/export, codegen) + embedded static web UI. Started by
   `rspostman gui`.
2. Web UI (no build step, plain ES modules): sidebar (collections tree, history,
   environments), request tabs, request editor (params, auth, headers, body, scripts),
   response viewer (pretty/raw/preview, headers, cookies, tests, console), environment
   selector and editor, save/rename/delete, import.
3. Playwright test-suite driving the UI in Chromium against the echo server.

Sprint 4 then adds the remaining GUI surfaces (runner, mock servers, cookies manager, code
snippets, WebSocket, docs, settings), the Tauri desktop shell and the user documentation.
