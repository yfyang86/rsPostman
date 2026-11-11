# Sprint 1 – Retrospective and plan for sprint 2

## Done

All nine scope items of `PLAN.md` are implemented and tested. The core is a plain synchronous
library (`rspostman-core`) and the CLI a thin layer over it, so the GUI can reuse everything.

## What went well / what to change

* Using the Postman v2.1 JSON as the storage format removed a whole import/export mapping layer.
* A built-in echo server made the test-suite hermetic (no network needed). Keep doing that:
  sprint 2's mock server and runner will be tested the same way.
* `Session` (scopes + cookie jar + settings) is the right seam for scripts: scripts only need to
  mutate `session.scopes`.

## Sprint 2 plan (summary – details in `Sprints/sprint-2/PLAN.md`)

1. JavaScript sandbox with the `pm.*` API (pre-request and test scripts, `pm.test`, `pm.expect`,
   variables, `pm.sendRequest`, `pm.execution.setNextRequest`, `console`).
2. Collection runner (Newman-like): iterations, CSV/JSON data files, delay, bail, folder
   selection, flow control; reporters: CLI, JSON, JUnit XML, HTML.
3. Mock server generated from saved examples.
4. Monitors: scheduled local runs.
5. Importers: OpenAPI 3 / Swagger 2 (JSON + YAML), HAR.
6. Collection documentation generator (Markdown / HTML).
7. WebSocket client.
