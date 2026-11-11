# Sprint 3 – Retrospective and plan for sprint 4

## Done

The GUI covers the daily request / response loop, collections, environments, scripts, import
and history, all verified by Playwright.

## Observations

* The decision to make the GUI testable with Playwright paid for itself immediately: it
  exposed a request-hang bug in a third-party HTTP server that no unit test or CLI test had
  hit, because only a browser opens six connections in the same millisecond.
* Cross-checking GUI actions with the CLI inside the same test is cheap and catches
  persistence bugs.

## Sprint 4 plan (final sprint – details in `Sprints/sprint-4/PLAN.md`)

1. Remaining GUI screens: collection runner (with data file, live progress, reports), mock
   servers, WebSocket client, cookie manager, settings, code snippets, documentation preview.
2. Tauri v2 desktop shell (`crates/desktop`) that starts the backend in-process and shows the
   same UI in a native window.
3. Hardening pass: accessibility checks, empty/error states, a full regression run.
4. User documentation: README with feature matrix vs Postman, CLI reference, script API
   reference, known limitations.
