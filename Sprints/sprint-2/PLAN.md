# Sprint 2 – Scripting, runner, mock server, importers, WebSocket

## Scope

| # | Feature | Acceptance |
|---|---------|------------|
| 1 | JavaScript sandbox (embedded Boa engine, no Node needed) with the Postman scripting API: `pm.test`, `pm.expect` (chai BDD subset), `pm.response` / `pm.response.to.*`, `pm.request` (mutable in pre-request scripts), `pm.variables` / `environment` / `globals` / `collectionVariables` / `iterationData`, `pm.info`, `pm.cookies`, `pm.sendRequest`, `pm.execution.setNextRequest` / `skipRequest`, `pm.visualizer`, legacy `postman.*` / `tests[...]` / `responseBody`, `console.*`, top-level `await`, `setTimeout`, `atob`/`btoa` | Unit tests covering each API group, including failure messages |
| 2 | Script inheritance: collection → folder → request, pre-request and test | Runner test |
| 3 | Collection runner: folder selection, iterations, CSV/JSON data files, delay, bail, flow control, persisted variable changes | CLI e2e test |
| 4 | Reporters: CLI, JSON, JUnit XML, HTML | CLI e2e test |
| 5 | Mock server from saved examples (path variables, example selection headers, dynamic variables, CORS) | Unit + CLI e2e test |
| 6 | Monitors: scheduled local runs with a JSON-lines log | CLI e2e test |
| 7 | Importers: OpenAPI 3 / Swagger 2 (JSON + YAML), HAR | Unit + CLI e2e test |
| 8 | Documentation generator (Markdown, HTML) | Unit + CLI e2e test |
| 9 | WebSocket client (CLI interactive + one-shot) | Unit + CLI e2e test |

## Design notes

* The whole `pm` API is implemented in one JavaScript prelude (`crates/core/src/sandbox.js`);
  Rust only provides three native hooks (`__input`, `__send`, `__dynamic`) and exchanges state
  as JSON. Keeps the engine binding surface tiny and the API easy to extend.
* Every script runs in a fresh engine context with loop-iteration and recursion limits, so a
  runaway script cannot hang a run.
* The mock server, echo server and (next sprint) the GUI backend share `core::server`.

## Out of scope

gRPC, GraphQL schema introspection UI, Socket.IO/MQTT, Postman Flows, `require()` of npm
packages inside scripts (only `uuid` and `chai` shims are provided).
