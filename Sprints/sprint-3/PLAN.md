# Sprint 3 – GUI: local API server, web UI, Playwright tests

## Decisions

* The GUI is a **web UI served by the `rspostman` binary itself** (`rspostman gui`). It is
  written as plain ES modules – no bundler, no npm dependencies, embedded into the binary at
  compile time – so `cargo build` is the only build step. The Tauri desktop shell (sprint 4)
  loads the very same UI from the very same backend, which means every Playwright test also
  covers the desktop app's behaviour.
* The backend (`crates/server`) is a thin JSON API over `rspostman-core`; CLI and GUI share one
  workspace, so a collection edited in the GUI can be run with `rspostman run` and vice versa.
* Security of the local server: loopback only, `Host` header check (DNS rebinding), and a
  custom header required on every state-changing request (blocks cross-site requests).

## Scope

| # | Feature | Acceptance |
|---|---------|------------|
| 1 | API server: bootstrap, collections, environments, globals, settings, send, history, cookies, import/export, codegen, runner, mocks, WebSocket, docs, uploads | Used by the UI; e2e tests |
| 2 | App shell: toolbar, environment selector, resizable sidebar (collections / history / environments), tabs with dirty markers, status bar, light + dark theme, keyboard shortcuts | Playwright |
| 3 | Request editor: method, URL ↔ params sync, path variables, headers, auth helpers (all core types), body modes (raw + beautify, urlencoded, form-data with file upload, binary, GraphQL), pre-request / test scripts with snippets, description | Playwright |
| 4 | Response viewer: status / time / size, pretty / raw / preview, headers, cookies, test results, console, sent request, download, copy, saved examples | Playwright |
| 5 | Collections: create, folders, add / rename / duplicate / delete, drag-and-drop move, filter, collection + folder editor (auth, variables, scripts), export | Playwright |
| 6 | Environments + globals editor, active environment shared with the CLI | Playwright |
| 7 | Import dialog (collection, environment, OpenAPI, HAR, curl) and curl paste into the URL bar | Playwright |
| 8 | History: list, reopen with response, delete, clear | Playwright |

Runner, mock server, WebSocket, cookie manager, settings and code-snippet **screens** are
sprint 4 (their API endpoints already exist).
