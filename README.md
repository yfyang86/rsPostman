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
rspostman send POST https://httpbin.org/post --json '{"name": "Ann"}' -u user   # asks for the password
rspostman send https://example.com/upload -F note=hi -F file=@./photo.png

# collections and environments
rspostman collection create Shop
rspostman c set-var Shop base https://api.example.com
rspostman c add-request Shop "List orders" '{{base}}/orders?state={{state}}'
rspostman env create dev && rspostman env set dev state open && rspostman env use dev
rspostman env set dev api_token --secret        # asks for the value, masks it in listings
rspostman vault set api_token                   # or keep it encrypted: use as {{vault:api_token}}
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
rspostman benchmark https://api.example.com/users -n 100 -c 10        # percentiles, TTFB, req/s
rspostman benchmark https://api.example.com/users -t 30s --rate 50 -c 8 --warmup 5 \
    --assert 'p95<200ms' --assert 'error_rate<=1%'                    # fixed time and rate, pass/fail for CI
rspostman benchmark https://api.example.com/users -n 200 --report-json base.json
rspostman benchmark https://api.example.com/users -n 200 --compare base.json --report-html bench.html
rspostman benchmark --collection Shop -n 20 --folder "List orders"    # per-request percentiles
rspostman profile https://api.openai.com/v1/chat/completions \
    --model gpt-4o-mini --prompt "Say hello" --bearer env:OPENAI_API_KEY -n 5
```

`rspostman <command> --help` documents every option. `rspostman echo` starts a small
httpbin-like server that is handy for trying things offline; the bundled example works with it:

```bash
rspostman echo &
rspostman run examples/workflow.postman_collection.json -d examples/users.csv
```

(The example contains one deliberately failing request in its `Broken` folder, so the run
reports two failed assertions and exits with status 1 – add `--folder Auth` for a green run.)

### Benchmarking

```bash
rspostman benchmark [METHOD] URL | --collection C [--request R | --folder F]
```

| Option | Meaning |
|--------|---------|
| `-n N` / `-t 30s` | N iterations of each request, or run for a fixed time (`500ms`, `30s`, `2m`) |
| `-c N` | concurrent workers (max 256) |
| `--rate R` | target R requests per second over all workers; if the workers cannot keep up, the achieved rate in the report is lower |
| `--delay MS` | pause between the requests of each worker (alternative to `--rate`) |
| `--warmup N` | N unmeasured rounds first (connection setup, caches) |
| `--no-keepalive` | new connection for every request |
| `--assert COND` | threshold, repeatable. Metrics: `min mean max stddev p10 p50 p75 p90 p95 p99 p999`, the same with a `ttfb_` prefix, `error_rate` (%), `errors`, `http_errors`, `rps`, `requests`; operators `< <= > >=`; units `ms`, `s`, `%` |
| `--compare FILE` `--tolerance P` | compare with the JSON report of an earlier run; a latency metric regresses when it is more than P % (default 10) and more than 1 ms slower, throughput when it drops by more than P % (only judged when both runs lasted at least a second), the error rate when it rises by more than 0.5 points |
| `--report-json` / `--report-csv` / `--report-html` | full report (also the baseline format), every sample, standalone page with charts |

Exit status: with `--assert`, 0 when every threshold holds; without, 0 when every request was
answered with a status below 400. A regression against `--compare` always gives 1.

Latency is measured per request from just before it is sent until the body has been read;
"first byte" is the time until the response headers arrive. Short runs are noisy: compare
runs of at least several seconds, with the same options, on a quiet machine. The tool is a closed-loop client
on one machine: it shows how an endpoint responds to this client, not what a fleet of
independent users would experience, and at high rates the client itself becomes the limit.

## Feature overview

| Area | What is there |
|------|---------------|
| Requests | All HTTP methods, query + path variables, headers, bodies: raw (JSON/XML/HTML/text/JS), x-www-form-urlencoded, multipart form-data with files, binary, GraphQL; redirects, timeouts, proxy, custom CA, client certificates, HTTP/1.1 + HTTP/2 |
| Authorization | Inherit, No Auth, Basic, Bearer, API key, Digest, OAuth 1.0a, OAuth 2.0 (token use; client-credentials / password / refresh-token retrieval), AWS Signature v4, JWT bearer (HS256/384/512) |
| Variables | Globals, collection, environment, iteration data, local; nested `{{vars}}`; about 60 dynamic variables (`{{$guid}}`, `{{$timestamp}}`, `{{$randomEmail}}` …) |
| Scripts | Pre-request and test scripts at collection, folder and request level in an embedded JavaScript engine: `pm.test`, `pm.expect` (chai-style), `pm.response`, `pm.request`, all variable scopes, `pm.sendRequest`, `pm.execution.setNextRequest` / `skipRequest`, `pm.cookies`, `pm.info`, `pm.visualizer.set`, legacy `postman.*` / `tests[...]`, `console.*`. See [docs/SCRIPTING.md](docs/SCRIPTING.md) |
| Collections | Folders, drag-and-drop reordering, duplicate, descriptions, saved examples, collection/folder auth + scripts + variables |
| Runner | Folder selection, iterations, CSV/JSON data files, delay, bail, flow control, live progress; reports: console, JSON, JUnit XML, HTML |
| Performance | `benchmark`: a number of iterations or a fixed time, concurrent workers, target request rate, warm-up rounds, keep-alive on/off; reports latency percentiles (P10 … P99.9), time to first byte, requests/s, status codes, a latency distribution and a per-second timeline; pass/fail thresholds (`--assert 'p95<200ms'`) and comparison with a saved baseline for CI; JSON, CSV and standalone HTML reports; live charts in the GUI. Scripts are not run. `profile`: health of an OpenAI-compatible streaming endpoint (healthy / degraded / unhealthy), time to first token, output rate (tokens/s or events/s), token usage, captured completions |
| Vault | Secrets encrypted at rest (Argon2id + XChaCha20-Poly1305), unlocked with a passphrase, used as `{{vault:name}}`; never written to history, reports, snapshots or exposed to scripts |
| Credentials | Masked inputs with a reveal button, secret variables, hidden password prompt and `env:NAME` / `@FILE` sources on the CLI, redaction in history, reports and verbose output, owner-only workspace files – see [Handling credentials](#handling-credentials) |
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
* OS keychain integration and idle auto-lock for the vault; encryption of files other than the vault (see [Handling credentials](#handling-credentials)).

## Workspace layout

```text
~/.rspostman/
  collections/<id>.json     Postman Collection v2.1
  environments/<id>.json    Postman environment
  globals.json  cookies.json  settings.json  state.json
  vault.json                encrypted secrets (see "Handling credentials")
  history.jsonl             one exchange per line (credentials redacted)
  monitors/<collection>.jsonl
  uploads/                  files picked in the browser GUI
```

Everything is human-readable JSON. Collections can go under version control – as long as
they reference credentials through `{{variables}}` instead of containing them (see below).

## Handling credentials

### The vault – secrets encrypted at rest

Passwords, tokens and keys belong in the **vault**: a file in the workspace (`vault.json`)
in which every value is encrypted. You unlock it with a passphrase and use a secret anywhere a
variable works – URL, header, body, auth field, or as the value of an environment variable –
by writing `{{vault:name}}`.

```bash
rspostman vault set github_token                 # creates the vault on first use; hidden prompts
rspostman send https://api.github.com/user --bearer '{{vault:github_token}}'
rspostman vault list                             # names only, no passphrase needed
rspostman vault get github_token                 # print one secret
rspostman vault passwd                           # change the passphrase (re-encrypts everything)
```

In the GUI, the padlock next to the environment selector opens the vault screen (create,
unlock, store, delete, lock, change passphrase). The CLI asks for the passphrase when a
request needs the vault; for scripts and CI set `RSPOSTMAN_VAULT_PASSPHRASE`.

What the vault guarantees:

* **On disk** values are sealed with XChaCha20-Poly1305 under a key derived from the
  passphrase with Argon2id (19 MiB, 2 passes, random salt). Each value has its own random
  nonce and is bound to its name, so entries cannot be swapped or altered unnoticed. Secret
  *names* are stored in clear. The passphrase is stored nowhere.
* **In use** a vault secret is decrypted only to build the request that is sent. It is removed
  from the request snapshot (so neither the GUI's "Request" tab nor `-v --show-secrets` shows
  it), from the history (including a response that echoes it back), from run reports and from
  error messages. Collections, environments and history entries only ever contain the
  `{{vault:name}}` reference, so they can be shared and replayed.
* **Scripts** cannot read the vault: `pm.variables.get` / `replaceIn` do not resolve
  `{{vault:…}}`. Mock servers and the variable preview do not resolve it either.
* **Locked means not sent.** If the vault is locked or a secret is missing, the request fails
  with an explanation instead of sending the placeholder text to a server.
* The GUI can store and replace secrets but never displays a stored value again.

What it does not do:

* There is **no recovery**: a lost passphrase means the vault's contents are lost.
* The vault is unlocked per `rspostman` process. The GUI keeps it unlocked (key in the
  backend's memory) until you press *Lock now* or quit; there is no idle auto-lock yet.
* Generated code snippets contain the real secret – that is what a snippet is for.
* Secrets shorter than 4 characters are not scrubbed from responses and error messages.
* It protects files at rest, not a compromised machine: malware running as your user while
  the vault is unlocked, or a server that receives the secret, is out of scope.
* The OS keychain is not used; the passphrase is typed or supplied through the environment.

### Everything else

For credentials that are not in the vault, rsPostman keeps them out of sight and out of the
places they tend to leak into:

* **Typing them.** In the GUI, passwords, tokens and keys are shown as dots (auth fields,
  secret variables, and header / form values whose name looks like a credential such as
  `Authorization` or `X-Api-Key`); an eye button reveals a value. A value that is only a
  `{{variable}}` reference is shown as typed. On the command line nothing has to be typed in
  clear text or end up in the shell history:

  ```bash
  rspostman send https://api.example.com/me -u bob            # hidden prompt (or RSPOSTMAN_PASSWORD)
  rspostman send https://api.example.com/me -u bob:env:API_PW # from an environment variable
  rspostman send https://api.example.com/me --bearer env:API_TOKEN
  rspostman send https://api.example.com/me --bearer @token.txt
  rspostman env set prod api_token --secret                   # hidden prompt, or pipe the value in
  ```

* **Secret variables.** Environment, global and collection variables can be marked *secret*
  (Postman's `"type": "secret"`; the GUI ticks it automatically for names such as `password`
  or `api_token`). Secret values are masked in the GUI and in `env show` / `globals list` /
  `collection show` unless `--show-secrets` is given.
* **History.** Literal credentials are stored as `<redacted>`: auth helper secrets,
  `Authorization` / `Cookie` / API-key style headers, sensitive query parameters, password and
  token fields of JSON and form bodies, `user:password@` in URLs, `Set-Cookie` values and token
  fields of JSON responses. `{{variable}}` references are kept, so such entries can still be
  replayed. Turn `history_store_secrets` on in the settings to keep everything.
* **Reports and output.** Run reports (JSON, JUnit, HTML), benchmark / profile reports and
  `-v` output are redacted the same way (`--show-secrets` prints them). The "Request" tab of
  the response viewer hides credentials until *Show credentials* is pressed.
* **Files.** The workspace directory is created with mode `0700` and every file in it is
  written with mode `0600` (Unix), so other accounts on the machine cannot read it.

Still plain text on disk: whatever you type *literally* into a collection or environment, and
the cookie jar. Put such values in the vault and reference them instead. Exported environments
contain their values. For non-vault credentials the redaction is name-based: one in an
unusually named field, a free-text body or a URL path is not recognised (vault secrets are
removed by value, wherever they appear).

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
