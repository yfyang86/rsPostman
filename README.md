# rsPostman

A local-first, Postman-compatible API client written in Rust.

* **One core, three front ends** – a library (`rspostman-core`), a CLI (`rspostman`) and a GUI
  (web UI served by the binary, plus a Tauri desktop shell around the same UI).
* **Local only** – collections, environments, history, cookies and settings are plain JSON files
  in a workspace folder. No account, no cloud, no telemetry.
* **Postman formats are the native formats** – collections are stored as Postman Collection
  v2.1 and environments as Postman environment files, so import/export is loss-free and the
  files work with Postman and Newman.

```text
crates/core      engine: model, variables, HTTP, auth, scripts, runner, mock, importers …
crates/cli       `rspostman` command line (send, run, mock, monitor, ws, codegen, gui …)
crates/server    local JSON API + embedded web UI (used by `rspostman gui` and the desktop app)
crates/desktop   Tauri v2 desktop shell
ui/              the web UI (plain ES modules, no build step)
e2e/             Playwright end-to-end tests
examples/        sample collection and data file
Sprints/         development log: plan, test report and retrospective per sprint
```

## Build and run

```bash
cargo build --release            # builds the rspostman binary (CLI + GUI server)
```

```bash
./target/release/rspostman gui   # opens the GUI in your browser (http://127.0.0.1:8686)
```

Desktop app (needs the platform webview libraries, e.g. `libwebkit2gtk-4.1-dev` on
Debian/Ubuntu; WebView2 on Windows; nothing extra on macOS):

```bash
cargo build --release -p rspostman-desktop
```

```bash
./target/release/rspostman-desktop
```

The workspace folder is `~/.rspostman` by default; override it with `--workspace DIR` or
`RSPOSTMAN_HOME`. The CLI, the browser GUI and the desktop app can share one workspace.

## Quick tour of the CLI

```bash
# ad-hoc requests
rspostman send https://httpbin.org/get -q page=2 -H 'Accept: application/json'
rspostman send POST https://httpbin.org/post --json '{"name": "Ann"}' -u user:pass
rspostman send https://example.com/upload -F note=hi -F file=@./photo.png

# collections and environments
rspostman collection create Shop
rspostman c set-var Shop base https://api.example.com
rspostman c add-request Shop "List orders" '{{base}}/orders?state={{state}}'
rspostman env create dev && rspostman env set dev state open && rspostman env use dev
rspostman request Shop "List orders"

# import / export (Postman, OpenAPI 3 / Swagger 2, HAR, curl)
rspostman import ./petstore.yaml
rspostman import "curl -X POST https://example.com/api -d 'a=1'" --collection Scratch
rspostman export collection Shop -o shop.postman_collection.json

# run a collection with scripts and tests, like Newman
rspostman run Shop -e dev -d users.csv --report-junit report.xml --report-html report.html

# mock server from saved examples, scheduled monitor, WebSocket, code generation, docs
rspostman mock Shop --port 4010
rspostman monitor Shop --every 5m
rspostman ws wss://echo.example/socket -m hello
rspostman codegen python -c Shop -r "List orders"
rspostman docs Shop -f html -o shop.html

# performance: benchmark an endpoint or collection, profile an LLM provider
rspostman benchmark https://api.example.com/users -n 100 -c 10        # P10/P50/P90/P95/P99 + req/s
rspostman benchmark --collection Shop -n 20 --folder "List orders"    # per-request percentiles
rspostman profile https://api.openai.com/v1/chat/completions \
    --model gpt-4o-mini --prompt "Say hello" -H "Authorization: Bearer $KEY" -n 5
```

`rspostman <command> --help` documents every option. `rspostman echo` starts a small
httpbin-like server that is handy for trying things offline; the bundled example works with it:

```bash
rspostman echo &
rspostman run examples/workflow.postman_collection.json -d examples/users.csv
```

(The example contains one deliberately failing request in its `Broken` folder, so the run
reports two failed assertions and exits with status 1 – add `--folder Auth` for a green run.)

## Feature overview

| Area | What is there |
|------|---------------|
| Requests | All HTTP methods, query + path variables, headers, bodies: raw (JSON/XML/HTML/text/JS), x-www-form-urlencoded, multipart form-data with files, binary, GraphQL; redirects, timeouts, proxy, custom CA, client certificates, HTTP/1.1 + HTTP/2 |
| Authorization | Inherit, No Auth, Basic, Bearer, API key, Digest, OAuth 1.0a, OAuth 2.0 (token use; client-credentials / password / refresh-token retrieval), AWS Signature v4, JWT bearer (HS256/384/512) |
| Variables | Globals, collection, environment, iteration data, local; nested `{{vars}}`; about 60 dynamic variables (`{{$guid}}`, `{{$timestamp}}`, `{{$randomEmail}}` …) |
| Scripts | Pre-request and test scripts at collection, folder and request level in an embedded JavaScript engine: `pm.test`, `pm.expect` (chai-style), `pm.response`, `pm.request`, all variable scopes, `pm.sendRequest`, `pm.execution.setNextRequest` / `skipRequest`, `pm.cookies`, `pm.info`, `pm.visualizer.set`, legacy `postman.*` / `tests[...]`, `console.*`. See [docs/SCRIPTING.md](docs/SCRIPTING.md) |
| Collections | Folders, drag-and-drop reordering, duplicate, descriptions, saved examples, collection/folder auth + scripts + variables |
| Runner | Folder selection, iterations, CSV/JSON data files, delay, bail, flow control, live progress; reports: console, JSON, JUnit XML, HTML |
| Performance | `benchmark`: latency percentiles P10/P50/P90/P95/P99, req/s, throughput, per-request stats for collections, concurrency; `profile`: LLM streaming health (healthy / degraded / unhealthy), time-to-first-token, output rate (tokens/s or events/s), token usage, captured completions |
| Mock servers | Serve saved examples; path variables, example selection by header, dynamic variables in bodies, CORS, request log |
| Monitors | `rspostman monitor` runs a collection on an interval and appends results to a JSON-lines log |
| Import / export | Postman Collection v2.0/v2.1, Postman environments/globals, OpenAPI 3, Swagger 2 (JSON/YAML), HAR, curl; export Postman v2.1 |
| Code generation | curl, raw HTTP, HTTPie, Wget, Python, JavaScript fetch, Node axios, Go, Rust, Java OkHttp, C#, PHP, Ruby, PowerShell |
| Other | History, persistent cookie jar + manager, WebSocket client, API documentation generator (Markdown/HTML), light and dark theme |

### Not implemented

rsPostman covers Postman's local, single-user feature set. These parts are **not** there:

* Everything cloud-based (by design): accounts, sync, team workspaces, comments, public API network.
* gRPC, Socket.IO, MQTT; GraphQL schema introspection / autocomplete.
* Postman Flows, API Builder / schema governance, Postbot.
* Auth helpers: NTLM, Hawk, Akamai EdgeGrid, ASAP; OAuth 2 browser-based grants (paste the token instead).
* In scripts: `require()` of external libraries (only `uuid` and `chai` shims), `pm.cookies.jar()` writes, `setInterval`, the visualizer *renderer* (templates are stored, not rendered).
* HTTP/3. Mock servers match on method + path (+ query as tie-breaker), not on request bodies.
* Script editors are plain text areas (no syntax highlighting or autocomplete).

## Workspace layout

```text
~/.rspostman/
  collections/<id>.json     Postman Collection v2.1
  environments/<id>.json    Postman environment
  globals.json  cookies.json  settings.json  state.json
  history.jsonl             one exchange per line
  monitors/<collection>.jsonl
  uploads/                  files picked in the browser GUI
```

Everything is human-readable and safe to put under version control. Secrets you type into
auth fields or variables are stored in these files as plain text – treat the folder accordingly.

## Security notes

* `rspostman gui` and the desktop app listen on `127.0.0.1` only. The API rejects requests
  whose `Host` is not a loopback name and requires a custom header on every state-changing
  call, so other web pages open in your browser cannot drive it.
* Scripts run in a sandboxed JavaScript engine without file-system or process access; they can
  make HTTP requests through `pm.sendRequest` (that is their purpose) and are stopped by
  loop/recursion limits if they run away. Only run collections you trust, as with Postman.

## Testing

```bash
scripts/test-all.sh
```

runs the Rust test-suite (unit, integration, CLI end-to-end) and the Playwright GUI tests.
Individually:

```bash
cargo test
```

```bash
cargo build && cd e2e && npx playwright test
```

(`npm install` in `e2e/` first if Playwright is not installed globally.)

## License

MIT
