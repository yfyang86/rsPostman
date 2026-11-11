# Sprint 4 – Retrospective and backlog

## Outcome of the four sprints

| Sprint | Delivered |
|-------:|-----------|
| 1 | Core engine (Postman v2.1 model, variables, HTTP, 9 auth helpers, cookies, history, curl import, 14 code generators) and the CLI |
| 2 | `pm.*` scripting sandbox, Newman-style runner with reports, mock server, monitors, OpenAPI / Swagger / HAR import, docs generator, WebSocket client |
| 3 | Local API server, web UI for the request/response workflow, Playwright suite; replaced a faulty third-party HTTP server with a built-in one |
| 4 | Runner / mock / WebSocket / cookies / settings / code-snippet screens, Tauri desktop shell, documentation |

## What worked

* Postman's own file formats as the storage format; a synchronous core shared by all front ends.
* Hermetic tests at three levels (unit → CLI binary → browser). The most serious defects
  (collection overwritten by a file run; requests hanging in the HTTP server) were only found
  by the two outer levels.
* Writing the `pm` API in JavaScript on top of three native hooks.

## Backlog (not done, in rough priority order)

1. Syntax-highlighting code editor for bodies and scripts; variable autocomplete and hover preview.
2. OAuth 2 authorization-code / PKCE flow with a loopback redirect listener.
3. gRPC and GraphQL schema introspection.
4. `require()` for common sandbox libraries (lodash, crypto-js, ajv, moment) and a visualizer renderer.
5. Request-body matching and stateful scenarios for mock servers.
6. Restore open tabs after restart; multiple workspaces in the GUI.
7. NTLM / Hawk / EdgeGrid auth helpers; HTTP/3.
8. Signed installers (Tauri bundles) and CI for Windows / macOS.
9. Encrypt secrets at rest (they are plain JSON in the workspace today).
