# Sprint 4 – GUI completion, desktop shell, documentation

## Scope

| # | Feature | Acceptance |
|---|---------|------------|
| 1 | Collection runner screen: collection / folder, iterations, delay, data file, bail, keep-variables, live progress, per-request details, HTML / JUnit / JSON report links, stop | Playwright |
| 2 | Mock servers screen: start / stop, route table, live request log | Playwright |
| 3 | WebSocket client tabs: connect with handshake headers, send, message log, disconnect | Playwright |
| 4 | Cookie manager: list by domain, add, delete, clear | Playwright |
| 5 | Settings screen (timeouts, redirects, TLS, proxy, certificates, cookies, history), shared with the CLI | Playwright |
| 6 | Code snippet dialog for all 14 generators | Playwright |
| 7 | Collection menu: run collection / folder, mock, documentation (HTML view, Markdown download); drag-and-drop verified | Playwright |
| 8 | **Tauri v2 desktop shell** (`crates/desktop`): starts the backend in-process on a free port and opens the UI in a native window; icons; downloads to the Downloads folder | Builds; smoke test under Xvfb |
| 9 | Documentation: README (build, tour, feature matrix, what is *not* implemented, security notes), scripting reference, `scripts/test-all.sh` | Every command shown in the README was executed |

## Design notes

* Tool screens are ordinary tabs registered through three small extension points in
  `ui/js/state.js` (`tools`, `collectionActions`, `requestActions`), so the shell did not have
  to change.
* The desktop shell deliberately contains no application logic: the page is the same one the
  Playwright suite tests, and it gets no Tauri IPC capabilities.
