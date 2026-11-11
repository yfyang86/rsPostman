# Sprints

Each sprint lives in `Sprints/sprint-N/` and follows the same loop:

1. `PLAN.md`  – scope and acceptance criteria (written before coding)
2. dev        – code lands on the `sprint-N` git branch
3. `TEST.md`  – what was tested, what failed, what was fixed
4. `NEXT.md`  – retrospective and plan for the next sprint
5. commit and merge `sprint-N` into the main development branch (`--no-ff`)

| Sprint | Theme |
|-------:|-------|
| 1 | Core engine + CLI foundation (collections, environments, HTTP, auth, cookies, history, curl import, codegen) |
| 2 | Scripting (`pm.*`), collection runner, reporters, mock server, monitors, importers, WebSocket |
| 3 | GUI: local API server + web UI, Playwright end-to-end tests |
| 4 | GUI feature completion, Tauri desktop shell, polish, docs |
| 5 | Review of the profiler / benchmark contribution; credential handling (masking, redaction, prompts, file permissions) |
| 6 | Local vault: secrets encrypted at rest, `{{vault:name}}` |
| 7 | Benchmark: duration / rate / warm-up, TTFB, distribution and timeline, thresholds, baseline comparison, CSV / HTML reports, live charts |
| 8 | User manual with screenshots, in-app Help Center, screenshot generator |
