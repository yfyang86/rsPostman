//! Local backend of the rsPostman GUI: a JSON API over `rspostman-core` plus the embedded web UI.
//!
//! The server only listens on a loopback address. Requests are additionally checked for a
//! local `Host` header (DNS-rebinding protection) and every state-changing request must carry
//! the `X-Requested-With: rspostman` header, which browsers refuse to send cross-origin without
//! a CORS preflight that this server never answers.

use anyhow::{anyhow, bail, Context, Result};
use include_dir::{include_dir, Dir};
use rspostman_core::engine::{self, Exchange, Session};
use rspostman_core::import::Imported;
use rspostman_core::mock::{MockOptions, MockRoute};
use rspostman_core::model::{Auth, Collection, Environment, Request};
use rspostman_core::profile::{self, BenchReport, ProfileReport};
use rspostman_core::runner::{self, RunEvent, RunOptions, RunSummary};
use rspostman_core::server::{serve, Req, Resp, ServerHandle};
use rspostman_core::ws::WsClient;
use rspostman_core::Workspace;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

static UI: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../ui");

struct MockEntry {
    handle: ServerHandle,
    collection_id: String,
    collection_name: String,
    routes: Vec<MockRoute>,
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Default)]
struct RunEntry {
    done: bool,
    total: usize,
    iterations: usize,
    executions: Vec<Value>,
    summary: Option<RunSummary>,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct ProfileEntry {
    done: bool,
    runs_total: usize,
    runs_done: usize,
    report: Option<ProfileReport>,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
struct BenchEntry {
    done: bool,
    /// Planned number of requests (0 for a duration-based run).
    total: usize,
    names: Vec<String>,
    opts: rspostman_core::bench::BenchOptions,
    started: Option<std::time::Instant>,
    /// Samples so far, for interim results while the run is in progress.
    live: Vec<rspostman_core::bench::BenchSample>,
    report: Option<BenchReport>,
    comparison: Option<rspostman_core::bench::Comparison>,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

pub struct App {
    ws: Workspace,
    port: std::sync::atomic::AtomicU16,
    sockets: Mutex<HashMap<String, WsClient>>,
    mocks: Mutex<HashMap<String, MockEntry>>,
    runs: Mutex<HashMap<String, Arc<Mutex<RunEntry>>>>,
    profiles: Mutex<HashMap<String, Arc<Mutex<ProfileEntry>>>>,
    benchmarks: Mutex<HashMap<String, Arc<Mutex<BenchEntry>>>>,
    /// The unlocked vault (key in memory) – `None` while locked. Dropped on lock and on exit.
    vault: Mutex<Option<rspostman_core::vault::Vault>>,
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn not_found(what: &str) -> anyhow::Error {
    anyhow!("404:{what} not found")
}

/// Exchange as sent to the UI: adds a text rendering of the body.
fn exchange_json(ex: &Exchange) -> Value {
    let mut v = serde_json::to_value(ex).unwrap_or(Value::Null);
    if let (Some(r), Some(o)) = (&ex.response, v.get_mut("response").and_then(|r| r.as_object_mut())) {
        let is_text = r.is_text() || std::str::from_utf8(&r.body).is_ok();
        o.insert("is_text".into(), json!(is_text));
        o.insert("content_type".into(), json!(r.content_type()));
        o.insert("body_text".into(), json!(if is_text { r.text() } else { String::new() }));
        o.insert("headers_size".into(), json!(r.headers_size()));
    }
    if let (Some(p), Some(o)) = (&ex.request, v.get_mut("request").and_then(|r| r.as_object_mut())) {
        o.insert("body_preview".into(), json!(p.body.preview()));
    }
    v
}

fn execution_summary(x: &runner::Execution) -> Value {
    let ex = &x.exchange;
    json!({
        "iteration": x.iteration,
        "path": x.path,
        "id": ex.id,
        "item_id": ex.item_id,
        "name": ex.name,
        "method": ex.request.as_ref().map(|p| p.method.clone()),
        "url": ex.request.as_ref().map(|p| rspostman_core::secrets::redact_url(&p.url)),
        "status": ex.response.as_ref().map(|r| r.status),
        "status_text": ex.response.as_ref().map(|r| r.status_text.clone()),
        "time_ms": ex.response.as_ref().map(|r| r.time_ms),
        "size": ex.response.as_ref().map(|r| r.size),
        "error": ex.error,
        "script_error": ex.script_error,
        "skipped": ex.skipped,
        "failed": ex.failed(),
        "tests": ex.tests,
        "console": ex.console,
    })
}

#[derive(Deserialize)]
struct SendBody {
    #[serde(default)]
    collection_id: Option<String>,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    name: String,
    request: Request,
    /// Request-level scripts as currently edited in the UI.
    #[serde(default)]
    prerequest: String,
    #[serde(default)]
    test: String,
    #[serde(default)]
    environment_id: Option<String>,
    #[serde(default = "yes")]
    history: bool,
}
fn yes() -> bool {
    true
}

impl App {
    pub fn new(ws: Workspace) -> Arc<App> {
        Arc::new(App { ws, port: Default::default(), sockets: Default::default(), mocks: Default::default(), runs: Default::default(), profiles: Default::default(), benchmarks: Default::default(), vault: Default::default() })
    }

    fn collection(&self, id: &str) -> Result<Collection> {
        self.ws.collection(id)?.ok_or_else(|| not_found("collection"))
    }

    fn session(&self, collection: Option<&Collection>, environment_id: &Option<String>) -> Result<Session> {
        let mut s = match environment_id.as_deref() {
            // an explicit empty id means "no environment"
            Some("") => {
                let mut s = Session::load(&self.ws, collection, None)?;
                s.environment = None;
                s.scopes.environment.clear();
                s
            }
            Some(id) => Session::load(&self.ws, collection, Some(id))?,
            None => Session::load(&self.ws, collection, None)?,
        };
        if let Some(v) = self.vault.lock().map_err(|_| anyhow!("lock poisoned"))?.as_ref() {
            s.unlock_vault(v)?;
        }
        Ok(s)
    }

    fn vault_status(&self) -> Result<Value> {
        let unlocked = self.vault.lock().map_err(|_| anyhow!("lock poisoned"))?.is_some();
        Ok(json!({
            "exists": rspostman_core::vault::exists(&self.ws),
            "unlocked": unlocked,
            "names": rspostman_core::vault::names(&self.ws)?,
        }))
    }

    fn bootstrap(&self) -> Result<Value> {
        Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "workspace": self.ws.root.display().to_string(),
            "vault": self.vault_status()?,
            "collections": self.ws.collections()?,
            "environments": self.ws.environments()?,
            "globals": self.ws.globals()?,
            "state": self.ws.state()?,
            "settings": self.ws.settings()?,
            "languages": rspostman_core::codegen::LANGUAGES.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
            "auth_types": rspostman_core::auth::AUTH_TYPES,
        }))
    }

    fn send(&self, b: SendBody) -> Result<Value> {
        let stored = match &b.collection_id {
            Some(id) => self.ws.collection(id)?,
            None => None,
        };
        let mut ex;
        let mut session;
        match (stored, &b.item_id) {
            (Some(mut c), Some(item_id)) if c.find(item_id).is_some() => {
                // Run with the request as currently edited (possibly unsaved), inheriting auth
                // and scripts from the stored parents.
                let mut edited = c.clone();
                {
                    let item = edited.find_mut(item_id).expect("checked above");
                    item.request = Some(b.request.clone());
                    item.set_script("prerequest", &b.prerequest);
                    item.set_script("test", &b.test);
                }
                session = self.session(Some(&c), &b.environment_id)?;
                ex = session.send_item(&edited, item_id);
                if !b.name.is_empty() {
                    ex.name = b.name.clone();
                }
                session.persist(&self.ws, Some(&mut c))?;
            }
            (Some(mut c), _) => {
                // unsaved draft that belongs to a collection: collection-level auth, scripts, variables
                session = self.session(Some(&c), &b.environment_id)?;
                let mut scripts = engine::Scripts::default();
                for (listen, own, list) in [("prerequest", &b.prerequest, &mut scripts.prerequest), ("test", &b.test, &mut scripts.test)] {
                    let top = rspostman_core::model::script_of(&c.event, listen);
                    list.extend([top, own.clone()].into_iter().filter(|s| !s.trim().is_empty()));
                }
                let auth: Option<Auth> = b.request.auth.clone().or_else(|| c.auth.clone());
                let name = c.info.name.clone();
                ex = session.run(&b.name, None, &name, &b.request, auth.as_ref(), &scripts);
                session.persist(&self.ws, Some(&mut c))?;
            }
            (None, _) => {
                session = self.session(None, &b.environment_id)?;
                let scripts = engine::Scripts {
                    prerequest: [b.prerequest.clone()].into_iter().filter(|s| !s.trim().is_empty()).collect(),
                    test: [b.test.clone()].into_iter().filter(|s| !s.trim().is_empty()).collect(),
                };
                ex = session.run(&b.name, None, "", &b.request, b.request.auth.as_ref(), &scripts);
                session.persist(&self.ws, None)?;
            }
        }
        if b.history {
            let mut source = b.request.clone();
            // keep request-level scripts replayable from the history
            let _ = &mut source;
            engine::record(&self.ws, &session, &ex, Some(&source), b.collection_id.as_deref())?;
        }
        let mut v = exchange_json(&ex);
        v["scripts"] = json!({"prerequest": b.prerequest, "test": b.test});
        Ok(v)
    }

    fn start_run(self: &Arc<Self>, body: Value) -> Result<Value> {
        let collection_id = body["collection_id"].as_str().ok_or_else(|| anyhow!("collection_id is required"))?.to_string();
        let mut c = self.collection(&collection_id)?;
        let env: Option<String> = body.get("environment_id").and_then(|e| e.as_str()).map(|s| s.to_string());
        let mut session = self.session(Some(&c), &env)?;
        let data_text = body["data"].as_str().unwrap_or("");
        let opts = RunOptions {
            folder: body["folder"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()),
            item_ids: body["item_ids"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()),
            iterations: body["iterations"].as_u64().filter(|n| *n > 0).map(|n| n as usize),
            data: if data_text.trim().is_empty() { vec![] } else { runner::parse_data(data_text)? },
            delay_ms: body["delay_ms"].as_u64().unwrap_or(0),
            bail: body["bail"].as_bool().unwrap_or(false),
        };
        let persist = body["persist"].as_bool().unwrap_or(true);
        let id = rspostman_core::model::new_id();
        let entry = Arc::new(Mutex::new(RunEntry::default()));
        let cancel = entry.lock().map(|e| e.cancel.clone()).map_err(|_| anyhow!("lock poisoned"))?;
        self.runs.lock().map_err(|_| anyhow!("lock poisoned"))?.insert(id.clone(), entry.clone());
        let app = self.clone();
        std::thread::spawn(move || {
            let result = runner::run(&c, &mut session, &opts, &mut |ev| {
                if let Ok(mut e) = entry.lock() {
                    match ev {
                        RunEvent::Start { requests, iterations } => {
                            e.total = requests * iterations;
                            e.iterations = iterations;
                        }
                        RunEvent::Iteration { .. } => {}
                        RunEvent::Request(x) => e.executions.push(execution_summary(x)),
                    }
                }
                !cancel.load(Ordering::SeqCst)
            });
            let persisted = if persist { session.persist(&app.ws, Some(&mut c)) } else { Ok(()) };
            if let Ok(mut e) = entry.lock() {
                match result {
                    Ok(s) => e.summary = Some(s),
                    Err(err) => e.error = Some(err.to_string()),
                }
                if let Err(err) = persisted {
                    e.error.get_or_insert(format!("could not save variables: {err}"));
                }
                e.done = true;
            }
        });
        Ok(json!({ "id": id }))
    }

    fn run_status(&self, id: &str, since: usize) -> Result<Value> {
        let entry = self.runs.lock().map_err(|_| anyhow!("lock poisoned"))?.get(id).cloned().ok_or_else(|| not_found("run"))?;
        let e = entry.lock().map_err(|_| anyhow!("lock poisoned"))?;
        Ok(json!({
            "id": id,
            "done": e.done,
            "total": e.total,
            "iterations": e.iterations,
            "count": e.executions.len(),
            "executions": e.executions.iter().skip(since).collect::<Vec<_>>(),
            "error": e.error,
            "summary": e.summary.as_ref().map(|s| json!({
                "stats": s.stats, "duration_ms": s.duration_ms, "aborted": s.aborted, "success": s.success(),
                "collection_name": s.collection_name, "environment": s.environment, "started": s.started,
            })),
        }))
    }

    fn start_profile(self: &Arc<Self>, body: Value) -> Result<Value> {
        let mut request: Request = serde_json::from_value(body["request"].clone()).context("400:invalid request")?;
        // Generated OpenAI-compatible chat body unless the request already carries one.
        let has_body = request.body.as_ref().map(|b| b.mode == "raw" && b.raw.as_deref().map(|r| !r.trim().is_empty()).unwrap_or(false)).unwrap_or(false);
        if !has_body {
            let mut chat = json!({
                "model": body["model"].as_str().filter(|s| !s.is_empty()).unwrap_or("gpt-4o-mini"),
                "messages": [{"role": "user", "content": body["prompt"].as_str().unwrap_or("Say hello in a few words.")}],
                "stream": true,
            });
            if let Some(n) = body["max_tokens"].as_u64().filter(|n| *n > 0) {
                chat["max_tokens"] = json!(n);
            }
            request.method = "POST".into();
            request.body = Some(rspostman_core::model::Body::raw(&chat.to_string(), "json"));
        }
        let iterations = body["iterations"].as_u64().filter(|n| *n > 0).unwrap_or(3) as usize;
        let delay_ms = body["delay_ms"].as_u64().unwrap_or(0);
        let collection = match body["collection_id"].as_str() {
            Some(id) if !id.is_empty() => self.ws.collection(id)?,
            _ => None,
        };
        let env = body.get("environment_id").and_then(|e| e.as_str()).map(|s| s.to_string());
        let session = self.session(collection.as_ref(), &env)?;
        let auth = request.auth.clone().or_else(|| collection.and_then(|c| c.auth.clone()));
        let prepared = rspostman_core::http::prepare(&request, auth.as_ref(), &session.scopes).map_err(|e| anyhow!("400:{e}"))?;
        let settings = session.settings;
        let id = rspostman_core::model::new_id();
        let entry = Arc::new(Mutex::new(ProfileEntry { runs_total: iterations, ..Default::default() }));
        let cancel = entry.lock().map(|e| e.cancel.clone()).map_err(|_| anyhow!("lock poisoned"))?;
        self.profiles.lock().map_err(|_| anyhow!("lock poisoned"))?.insert(id.clone(), entry.clone());
        std::thread::spawn(move || {
            let report = profile::profile_llm(&prepared, &settings, iterations, delay_ms, Some(&cancel));
            let runs_done = report.samples.len();
            if let Ok(mut e) = entry.lock() {
                e.runs_done = runs_done;
                e.report = Some(report);
                e.done = true;
            }
        });
        Ok(json!({ "id": id }))
    }

    fn profile_status(&self, id: &str) -> Result<Value> {
        let entry = self.profiles.lock().map_err(|_| anyhow!("lock poisoned"))?.get(id).cloned().ok_or_else(|| not_found("profile"))?;
        let e = entry.lock().map_err(|_| anyhow!("lock poisoned"))?;
        Ok(json!({
            "id": id,
            "done": e.done,
            "runs_total": e.runs_total,
            "runs_done": e.runs_done,
            "report": e.report,
            "error": e.error,
        }))
    }

    fn start_benchmark(self: &Arc<Self>, body: Value) -> Result<Value> {
        use rspostman_core::bench;
        let opts = bench::BenchOptions {
            iterations: body["iterations"].as_u64().filter(|n| *n > 0).unwrap_or(10) as usize,
            duration_ms: body["duration_ms"].as_u64().filter(|n| *n > 0),
            concurrency: (body["concurrency"].as_u64().filter(|n| *n > 0).unwrap_or(1) as usize).min(bench::MAX_CONCURRENCY),
            delay_ms: body["delay_ms"].as_u64().unwrap_or(0),
            rate: body["rate"].as_f64().filter(|r| r.is_finite() && *r > 0.0),
            warmup: body["warmup"].as_u64().unwrap_or(0).min(1000) as usize,
            keep_alive: body["keep_alive"].as_bool().unwrap_or(true),
            thresholds: body["thresholds"].as_array().map(|a| a.iter().filter_map(|t| t.as_str()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()).unwrap_or_default(),
        };
        bench::validate_thresholds(&opts.thresholds).map_err(|e| anyhow!("400:{e}"))?;
        let env = body.get("environment_id").and_then(|e| e.as_str()).map(|s| s.to_string());
        let mut tasks: Vec<profile::BenchTask> = vec![];
        let adhoc: Option<Request> = serde_json::from_value(body["request"].clone()).ok();
        match (body["collection_id"].as_str().filter(|s| !s.is_empty()), adhoc) {
            (Some(cid), _) => {
                let c = self.collection(cid)?;
                let session = self.session(Some(&c), &env)?;
                let all = c.requests();
                let list: Vec<(String, &rspostman_core::model::Item)> = match (body["request_id"].as_str().filter(|s| !s.is_empty()), body["folder_id"].as_str().filter(|s| !s.is_empty())) {
                    (Some(rid), _) => {
                        let it = c.find(rid).ok_or_else(|| anyhow!("404:request `{rid}` not found"))?;
                        let path = all.iter().find(|(_, i)| i.id == it.id).map(|(p, _)| p.clone()).unwrap_or_else(|| it.name.clone());
                        vec![(path, it)]
                    }
                    (None, Some(fid)) => c.requests_in(fid),
                    (None, None) => all,
                };
                if list.is_empty() {
                    bail!("400:collection has no requests to benchmark");
                }
                for (path, item) in &list {
                    let Some(r) = &item.request else { continue };
                    let auth = c.effective_auth(&item.id);
                    let prepared = rspostman_core::http::prepare(r, auth.as_ref(), &session.scopes).map_err(|e| anyhow!("400:{e}"))?;
                    tasks.push(profile::BenchTask { name: path.clone(), prepared });
                }
            }
            (None, Some(request)) => {
                let session = self.session(None, &env)?;
                let prepared = rspostman_core::http::prepare(&request, request.auth.as_ref(), &session.scopes).map_err(|e| anyhow!("400:{e}"))?;
                tasks.push(profile::BenchTask { name: format!("{} {}", prepared.method, rspostman_core::secrets::redact_url(&prepared.url)), prepared });
            }
            (None, None) => bail!("400:give a collection_id or a request"),
        }
        let settings = self.session(None, &env)?.settings;
        let total = if opts.duration_ms.is_some() { 0 } else { tasks.len() * opts.iterations };
        let id = rspostman_core::model::new_id();
        let entry = Arc::new(Mutex::new(BenchEntry { total, names: tasks.iter().map(|t| t.name.clone()).collect(), opts: opts.clone(), ..Default::default() }));
        let cancel = entry.lock().map(|e| e.cancel.clone()).map_err(|_| anyhow!("lock poisoned"))?;
        self.benchmarks.lock().map_err(|_| anyhow!("lock poisoned"))?.insert(id.clone(), entry.clone());
        let duration_ms = opts.duration_ms;
        std::thread::spawn(move || {
            // live samples for the UI's interim results
            let progress = entry.clone();
            let on_sample = move |sample: &bench::BenchSample| {
                if let Ok(mut e) = progress.lock() {
                    if e.started.is_none() {
                        e.started = Some(std::time::Instant::now() - std::time::Duration::from_millis(sample.started_ms + sample.time_ms));
                    }
                    e.live.push(sample.clone());
                }
            };
            let report = bench::benchmark(&tasks, &settings, &opts, Some(&cancel), &on_sample);
            if let Ok(mut e) = entry.lock() {
                e.live = vec![];
                e.report = Some(report);
                e.done = true;
            }
        });
        Ok(json!({ "id": id, "total": total, "duration_ms": duration_ms }))
    }

    fn benchmark_status(&self, id: &str) -> Result<Value> {
        use rspostman_core::bench;
        let entry = self.benchmarks.lock().map_err(|_| anyhow!("lock poisoned"))?.get(id).cloned().ok_or_else(|| not_found("benchmark"))?;
        let e = entry.lock().map_err(|_| anyhow!("lock poisoned"))?;
        // What the UI needs: the statistics, plus only the failed samples (not every sample).
        let slim = |r: &BenchReport| {
            let mut r = r.clone();
            r.samples.retain(|s| s.error.is_some());
            r.samples.truncate(50);
            r
        };
        let (report, done_count) = match &e.report {
            Some(r) => (Some(slim(r)), r.total_requests),
            None if e.live.is_empty() => (None, 0),
            None => {
                // interim: computed from the samples so far (the most recent 300k on very long runs)
                let from = e.live.len().saturating_sub(300_000);
                let elapsed = e.started.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
                // thresholds are judged on the finished run only
                let opts = bench::BenchOptions { thresholds: vec![], ..e.opts.clone() };
                (Some(slim(&bench::summarize(&e.names, e.live[from..].to_vec(), &opts, None, elapsed, 0, None))), e.live.len())
            }
        };
        Ok(json!({
            "id": id,
            "done": e.done,
            "total": e.total,
            "duration_ms": e.opts.duration_ms,
            "elapsed_ms": e.started.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0),
            "done_count": done_count,
            "report": report,
            "comparison": e.comparison,
            "error": e.error,
        }))
    }

    fn import(&self, text: &str) -> Result<Value> {
        Ok(match rspostman_core::import::parse(text)? {
            Imported::Collection(c) => {
                let c = self.ws.import_collection(c)?;
                json!({"kind": "collection", "collection": c})
            }
            Imported::Environment(mut e) => {
                if self.ws.environment(&e.id)?.is_some() {
                    e.id = rspostman_core::model::new_id();
                }
                self.ws.save_environment(&e)?;
                json!({"kind": "environment", "environment": e})
            }
            Imported::Globals(g) => {
                let mut cur = self.ws.globals()?;
                for v in g.values.iter().filter(|v| v.enabled) {
                    cur.set(&v.key, &v.value);
                }
                self.ws.save_globals(&cur)?;
                json!({"kind": "globals", "globals": cur})
            }
            Imported::Request(r) => json!({"kind": "request", "request": r}),
        })
    }

    fn api(self: &Arc<Self>, req: &Req) -> Result<Resp> {
        let seg = req.segments();
        let ok = |v: Value| Ok(Resp::json(200, &v));
        let lock_err = || anyhow!("lock poisoned");
        match (req.method.as_str(), &seg[1..]) {
            ("GET", ["bootstrap"]) => ok(self.bootstrap()?),

            // ---------------------------------------------------------- collections
            ("GET", ["collections"]) => ok(json!(self.ws.collections()?)),
            ("POST", ["collections"]) => {
                let body: Value = req.json()?;
                let name = body["name"].as_str().unwrap_or("").trim();
                if name.is_empty() {
                    bail!("400:collection name is required");
                }
                let mut c = Collection::new(name);
                c.info.description = body["description"].as_str().map(|s| s.to_string());
                self.ws.save_collection(&c)?;
                ok(json!(c))
            }
            ("GET", ["collections", id]) => ok(json!(self.collection(id)?)),
            ("PUT", ["collections", id]) => {
                let mut c: Collection = req.json()?;
                self.collection(id)?;
                c.info.id = id.to_string();
                c.normalize();
                self.ws.save_collection(&c)?;
                ok(json!(c))
            }
            ("DELETE", ["collections", id]) => {
                self.collection(id)?;
                self.ws.delete_collection(id)?;
                ok(json!({"deleted": true}))
            }
            ("POST", ["collections", id, "duplicate"]) => {
                let mut c = self.collection(id)?;
                c.info.id = rspostman_core::model::new_id();
                c.info.name = format!("{} Copy", c.info.name);
                self.ws.save_collection(&c)?;
                ok(json!(c))
            }
            ("GET", ["collections", id, "export"]) => {
                let c = self.collection(id)?;
                let file: String = c.info.name.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
                Ok(Resp::new(200, "application/json", c.to_json()).with("Content-Disposition", &format!("attachment; filename=\"{file}.postman_collection.json\"")))
            }
            ("GET", ["collections", id, "docs"]) => {
                let c = self.collection(id)?;
                Ok(match req.param("format") {
                    Some("html") => Resp::new(200, "text/html; charset=utf-8", rspostman_core::docs::html(&c)),
                    _ => Resp::new(200, "text/markdown; charset=utf-8", rspostman_core::docs::markdown(&c)),
                })
            }

            // ---------------------------------------------------------- environments
            ("GET", ["environments"]) => ok(json!(self.ws.environments()?)),
            ("POST", ["environments"]) => {
                let body: Value = req.json()?;
                let name = body["name"].as_str().unwrap_or("").trim();
                if name.is_empty() {
                    bail!("400:environment name is required");
                }
                let mut e = Environment::new(name);
                if let Ok(values) = serde_json::from_value(body["values"].clone()) {
                    e.values = values;
                }
                self.ws.save_environment(&e)?;
                ok(json!(e))
            }
            ("PUT", ["environments", id]) => {
                let mut e: Environment = req.json()?;
                self.ws.environment(id)?.ok_or_else(|| not_found("environment"))?;
                e.id = id.to_string();
                self.ws.save_environment(&e)?;
                ok(json!(e))
            }
            ("DELETE", ["environments", id]) => {
                self.ws.environment(id)?.ok_or_else(|| not_found("environment"))?;
                self.ws.delete_environment(id)?;
                ok(json!({"deleted": true}))
            }
            ("GET", ["environments", id, "export"]) => {
                let e = self.ws.environment(id)?.ok_or_else(|| not_found("environment"))?;
                let file: String = e.name.chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' }).collect();
                Ok(Resp::new(200, "application/json", e.to_json()).with("Content-Disposition", &format!("attachment; filename=\"{file}.postman_environment.json\"")))
            }
            ("GET", ["globals"]) => ok(json!(self.ws.globals()?)),
            ("PUT", ["globals"]) => {
                let mut g: Environment = req.json()?;
                g.scope = Some("globals".into());
                g.name = "Globals".into();
                self.ws.save_globals(&g)?;
                ok(json!(g))
            }
            ("PUT", ["state"]) => {
                let st: rspostman_core::store::State = req.json()?;
                if let Some(id) = &st.active_environment {
                    self.ws.environment(id)?.ok_or_else(|| not_found("environment"))?;
                }
                self.ws.save_state(&st)?;
                ok(json!(st))
            }
            ("GET", ["settings"]) => ok(json!(self.ws.settings()?)),
            ("PUT", ["settings"]) => {
                let s: rspostman_core::http::Settings = req.json()?;
                self.ws.save_settings(&s)?;
                ok(json!(s))
            }

            // ---------------------------------------------------------- sending
            ("POST", ["send"]) => ok(self.send(req.json()?)?),
            ("POST", ["codegen"]) => {
                let body: Value = req.json()?;
                let request: Request = serde_json::from_value(body["request"].clone()).context("400:invalid request")?;
                let collection = match body["collection_id"].as_str() {
                    Some(id) => self.ws.collection(id)?,
                    None => None,
                };
                let env = body.get("environment_id").and_then(|e| e.as_str()).map(|s| s.to_string());
                let session = self.session(collection.as_ref(), &env)?;
                let auth = request.auth.clone().or_else(|| match (&collection, body["item_id"].as_str()) {
                    (Some(c), Some(item)) => {
                        // inherited auth: look at the parents of the stored item
                        let mut probe = c.clone();
                        if let Some(i) = probe.find_mut(item) {
                            if let Some(r) = i.request.as_mut() {
                                r.auth = None;
                            }
                        }
                        probe.effective_auth(item)
                    }
                    (Some(c), None) => c.auth.clone(),
                    _ => None,
                });
                let prepared = rspostman_core::http::prepare(&request, auth.as_ref(), &session.scopes).map_err(|e| anyhow!("400:{e}"))?;
                let code = rspostman_core::codegen::generate(body["lang"].as_str().unwrap_or("curl"), &prepared).map_err(|e| anyhow!("400:{e}"))?;
                ok(json!({ "code": code }))
            }
            ("POST", ["resolve"]) => {
                // preview of the URL after variable substitution
                let body: Value = req.json()?;
                let collection = match body["collection_id"].as_str() {
                    Some(id) => self.ws.collection(id)?,
                    None => None,
                };
                let env = body.get("environment_id").and_then(|e| e.as_str()).map(|s| s.to_string());
                let mut session = self.session(collection.as_ref(), &env)?;
                session.scopes.vault.clear(); // a preview must not reveal vault secrets
                let text = body["text"].as_str().unwrap_or("");
                ok(json!({"resolved": session.scopes.resolve(text), "unresolved": session.scopes.unresolved(text)}))
            }
            ("POST", ["oauth2", "token"]) => {
                let body: Value = req.json()?;
                let auth: Auth = serde_json::from_value(body["auth"].clone()).context("400:invalid auth")?;
                let collection = match body["collection_id"].as_str() {
                    Some(id) => self.ws.collection(id)?,
                    None => None,
                };
                let env = body.get("environment_id").and_then(|e| e.as_str()).map(|s| s.to_string());
                let session = self.session(collection.as_ref(), &env)?;
                let token = rspostman_core::auth::oauth2_fetch_token(&auth, &session.scopes, &session.settings).map_err(|e| anyhow!("400:{e}"))?;
                ok(token)
            }
            ("POST", ["upload"]) => {
                // body: raw file bytes; ?name=file.ext -> stored under <workspace>/uploads
                let name: String = req.param("name").unwrap_or("upload.bin").chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' { ch } else { '_' }).collect();
                let dir = self.ws.root.join("uploads");
                rspostman_core::store::create_private_dir(&dir)?;
                let path = dir.join(format!("{}-{name}", &rspostman_core::model::new_id()[..8]));
                rspostman_core::store::write_private(&path, &req.body)?;
                ok(json!({"path": path.display().to_string(), "size": req.body.len()}))
            }

            // ---------------------------------------------------------- history
            ("GET", ["history"]) => {
                let limit: usize = req.param("limit").and_then(|l| l.parse().ok()).unwrap_or(200);
                let all = self.ws.history()?;
                let rows: Vec<Value> = all
                    .iter()
                    .rev()
                    .take(limit)
                    .map(|e| {
                        json!({
                            "id": e.id, "timestamp": e.timestamp, "name": e.name,
                            "method": e.request.as_ref().map(|p| p.method.clone()).or_else(|| e.source.as_ref().map(|s| s.method.clone())),
                            "url": e.request.as_ref().map(|p| p.url.clone()).or_else(|| e.source.as_ref().map(|s| s.url.raw.clone())),
                            "status": e.response.as_ref().map(|r| r.status),
                            "time_ms": e.response.as_ref().map(|r| r.time_ms),
                            "error": e.error,
                        })
                    })
                    .collect();
                ok(json!(rows))
            }
            ("GET", ["history", id]) => {
                let e = self.ws.history()?.into_iter().find(|e| &e.id == id).ok_or_else(|| not_found("history entry"))?;
                let ex = Exchange { id: e.id.clone(), item_id: e.item_id.clone(), name: e.name.clone().unwrap_or_default(), request: e.request.clone(), response: e.response.clone(), error: e.error.clone(), ..Default::default() };
                ok(json!({"id": e.id, "timestamp": e.timestamp, "name": e.name, "collection_id": e.collection_id, "item_id": e.item_id, "source": e.source, "exchange": exchange_json(&ex)}))
            }
            ("DELETE", ["history", id]) => ok(json!({"deleted": self.ws.delete_history(id)?})),
            ("DELETE", ["history"]) => {
                self.ws.clear_history()?;
                ok(json!({"deleted": true}))
            }

            // ---------------------------------------------------------- vault
            // Secret values only ever travel browser -> server; no endpoint returns one.
            ("GET", ["vault"]) => ok(self.vault_status()?),
            ("POST", ["vault", "create"]) => {
                let body: Value = req.json()?;
                let v = rspostman_core::vault::Vault::create(&self.ws, body["passphrase"].as_str().unwrap_or("")).map_err(|e| anyhow!("400:{e}"))?;
                *self.vault.lock().map_err(|_| lock_err())? = Some(v);
                ok(self.vault_status()?)
            }
            ("POST", ["vault", "unlock"]) => {
                let body: Value = req.json()?;
                let v = rspostman_core::vault::Vault::unlock(&self.ws, body["passphrase"].as_str().unwrap_or("")).map_err(|e| anyhow!("400:{e}"))?;
                *self.vault.lock().map_err(|_| lock_err())? = Some(v);
                ok(self.vault_status()?)
            }
            ("POST", ["vault", "lock"]) => {
                *self.vault.lock().map_err(|_| lock_err())? = None;
                ok(self.vault_status()?)
            }
            ("POST", ["vault", "passphrase"]) => {
                let body: Value = req.json()?;
                // the current passphrase is checked again even when the vault is unlocked
                let mut v = rspostman_core::vault::Vault::unlock(&self.ws, body["current"].as_str().unwrap_or("")).map_err(|e| anyhow!("400:{e}"))?;
                v.change_passphrase(body["new"].as_str().unwrap_or("")).map_err(|e| anyhow!("400:{e}"))?;
                *self.vault.lock().map_err(|_| lock_err())? = Some(v);
                ok(self.vault_status()?)
            }
            ("PUT", ["vault", "secrets", name]) => {
                let body: Value = req.json()?;
                let value = body["value"].as_str().ok_or_else(|| anyhow!("400:value is required"))?;
                let mut guard = self.vault.lock().map_err(|_| lock_err())?;
                let v = guard.as_mut().ok_or_else(|| anyhow!("400:the vault is locked"))?;
                v.set(name, value).map_err(|e| anyhow!("400:{e}"))?;
                drop(guard);
                ok(self.vault_status()?)
            }
            ("DELETE", ["vault", "secrets", name]) => {
                let mut guard = self.vault.lock().map_err(|_| lock_err())?;
                let v = guard.as_mut().ok_or_else(|| anyhow!("400:the vault is locked"))?;
                if !v.remove(name)? {
                    return Err(not_found("secret"));
                }
                drop(guard);
                ok(self.vault_status()?)
            }

            // ---------------------------------------------------------- cookies
            ("GET", ["cookies"]) => ok(json!(self.ws.cookies()?.cookies)),
            ("POST", ["cookies"]) => {
                let body: Value = req.json()?;
                let raw = body["url"].as_str().unwrap_or("");
                let raw = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
                let u = url::Url::parse(&raw).map_err(|e| anyhow!("400:invalid URL: {e}"))?;
                let mut jar = self.ws.cookies()?;
                let c = jar.store_header(body["cookie"].as_str().unwrap_or(""), &u).ok_or_else(|| anyhow!("400:cannot parse cookie (expected `name=value; attributes`)"))?;
                self.ws.save_cookies(&jar)?;
                ok(json!(c))
            }
            ("DELETE", ["cookies"]) => {
                let mut jar = self.ws.cookies()?;
                let n = match req.param("domain") {
                    Some(d) => jar.remove(d, req.param("name")),
                    None => std::mem::take(&mut jar.cookies).len(),
                };
                self.ws.save_cookies(&jar)?;
                ok(json!({ "removed": n }))
            }

            // ---------------------------------------------------------- import
            ("POST", ["import"]) => {
                let body: Value = req.json()?;
                ok(self.import(body["text"].as_str().unwrap_or("")).map_err(|e| anyhow!("400:{e:#}"))?)
            }

            // ---------------------------------------------------------- runner
            ("POST", ["runs"]) => ok(self.start_run(req.json()?).map_err(|e| if e.to_string().starts_with("404:") { e } else { anyhow!("400:{e:#}") })?),
            ("GET", ["runs", id]) => ok(self.run_status(id, req.param("since").and_then(|s| s.parse().ok()).unwrap_or(0))?),
            ("POST", ["runs", id, "cancel"]) => {
                let entry = self.runs.lock().map_err(|_| lock_err())?.get(*id).cloned().ok_or_else(|| not_found("run"))?;
                entry.lock().map_err(|_| lock_err())?.cancel.store(true, Ordering::SeqCst);
                ok(json!({"cancelled": true}))
            }
            ("GET", ["runs", id, "report"]) => {
                let entry = self.runs.lock().map_err(|_| lock_err())?.get(*id).cloned().ok_or_else(|| not_found("run"))?;
                let e = entry.lock().map_err(|_| lock_err())?;
                let s = e.summary.as_ref().ok_or_else(|| anyhow!("400:run has not finished"))?;
                Ok(match req.param("format") {
                    Some("junit") => Resp::new(200, "application/xml", rspostman_core::report::junit(s)).with("Content-Disposition", "attachment; filename=\"run-report.xml\""),
                    Some("json") => Resp::new(200, "application/json", rspostman_core::report::json(s)).with("Content-Disposition", "attachment; filename=\"run-report.json\""),
                    _ => Resp::new(200, "text/html; charset=utf-8", rspostman_core::report::html(s)),
                })
            }

            // ---------------------------------------------------------- profile & benchmark
            ("POST", ["profile"]) => ok(self.start_profile(req.json()?).map_err(|e| anyhow!("400:{e:#}"))?),
            ("GET", ["profile", id]) => ok(self.profile_status(id)?),
            ("POST", ["profile", id, "cancel"]) => {
                let entry = self.profiles.lock().map_err(|_| lock_err())?.get(*id).cloned().ok_or_else(|| not_found("profile"))?;
                entry.lock().map_err(|_| lock_err())?.cancel.store(true, Ordering::SeqCst);
                ok(json!({"cancelled": true}))
            }
            ("POST", ["benchmark"]) => ok(self.start_benchmark(req.json()?).map_err(|e| anyhow!("400:{e:#}"))?),
            ("GET", ["benchmark", id]) => ok(self.benchmark_status(id)?),
            ("POST", ["benchmark", id, "cancel"]) => {
                let entry = self.benchmarks.lock().map_err(|_| lock_err())?.get(*id).cloned().ok_or_else(|| not_found("benchmark"))?;
                entry.lock().map_err(|_| lock_err())?.cancel.store(true, Ordering::SeqCst);
                ok(json!({"cancelled": true}))
            }
            ("POST", ["benchmark", id, "compare"]) => {
                // body: { baseline: <JSON report of an earlier run>, tolerance: percent }
                let body: Value = req.json()?;
                let baseline: BenchReport = serde_json::from_value(body["baseline"].clone()).map_err(|_| anyhow!("400:the baseline is not a benchmark JSON report"))?;
                if baseline.total_requests == 0 {
                    bail!("400:the baseline is not a benchmark JSON report (it contains no requests)");
                }
                let entry = self.benchmarks.lock().map_err(|_| lock_err())?.get(*id).cloned().ok_or_else(|| not_found("benchmark"))?;
                let mut e = entry.lock().map_err(|_| lock_err())?;
                let current = e.report.as_ref().ok_or_else(|| anyhow!("400:the benchmark has not finished"))?;
                let c = rspostman_core::bench::compare(&baseline, current, body["tolerance"].as_f64().unwrap_or(10.0));
                e.comparison = Some(c.clone());
                ok(json!(c))
            }
            ("GET", ["benchmark", id, "report"]) => {
                let entry = self.benchmarks.lock().map_err(|_| lock_err())?.get(*id).cloned().ok_or_else(|| not_found("benchmark"))?;
                let e = entry.lock().map_err(|_| lock_err())?;
                let r = e.report.as_ref().ok_or_else(|| anyhow!("400:the benchmark has not finished"))?;
                Ok(match req.param("format") {
                    Some("csv") => Resp::new(200, "text/csv; charset=utf-8", rspostman_core::bench::csv(r)).with("Content-Disposition", "attachment; filename=\"benchmark.csv\""),
                    Some("json") => Resp::new(200, "application/json", serde_json::to_string_pretty(r)?).with("Content-Disposition", "attachment; filename=\"benchmark.json\""),
                    _ => Resp::new(200, "text/html; charset=utf-8", rspostman_core::bench_report::html(r, e.comparison.as_ref())),
                })
            }

            // ---------------------------------------------------------- mock servers
            ("GET", ["mocks"]) => {
                let mocks = self.mocks.lock().map_err(|_| lock_err())?;
                let mut rows: Vec<Value> = mocks.iter().map(|(id, m)| mock_json(id, m)).collect();
                rows.sort_by_key(|r| r["port"].as_u64());
                ok(json!(rows))
            }
            ("POST", ["mocks"]) => {
                let body: Value = req.json()?;
                let c = self.collection(body["collection_id"].as_str().unwrap_or(""))?;
                let env = body.get("environment_id").and_then(|e| e.as_str()).map(|s| s.to_string());
                let mut session = self.session(Some(&c), &env)?;
                session.scopes.vault.clear(); // mock responses must not serve vault secrets
                let log = Arc::new(Mutex::new(Vec::<String>::new()));
                let sink = log.clone();
                let (handle, routes) = rspostman_core::mock::start(
                    &c,
                    session.scopes,
                    "127.0.0.1",
                    body["port"].as_u64().unwrap_or(0) as u16,
                    MockOptions { delay_ms: body["delay_ms"].as_u64().unwrap_or(0), no_cors: false },
                    Some(Box::new(move |line| {
                        if let Ok(mut l) = sink.lock() {
                            l.push(format!("{} {line}", chrono::Local::now().format("%H:%M:%S")));
                            let extra = l.len().saturating_sub(500);
                            l.drain(..extra);
                        }
                    })),
                )
                .map_err(|e| anyhow!("400:{e}"))?;
                let id = rspostman_core::model::new_id();
                let entry = MockEntry { handle, collection_id: c.id().to_string(), collection_name: c.info.name.clone(), routes, log };
                let v = mock_json(&id, &entry);
                self.mocks.lock().map_err(|_| lock_err())?.insert(id, entry);
                ok(v)
            }
            ("GET", ["mocks", id]) => {
                let mocks = self.mocks.lock().map_err(|_| lock_err())?;
                let m = mocks.get(*id).ok_or_else(|| not_found("mock server"))?;
                ok(mock_json(id, m))
            }
            ("DELETE", ["mocks", id]) => {
                let m = self.mocks.lock().map_err(|_| lock_err())?.remove(*id).ok_or_else(|| not_found("mock server"))?;
                m.handle.stop();
                ok(json!({"stopped": true}))
            }

            // ---------------------------------------------------------- websocket
            ("POST", ["ws"]) => {
                let body: Value = req.json()?;
                let env = body.get("environment_id").and_then(|e| e.as_str()).map(|s| s.to_string());
                let session = self.session(None, &env)?;
                let headers: Vec<(String, String)> = body["headers"]
                    .as_array()
                    .map(|a| a.iter().filter(|h| !h["disabled"].as_bool().unwrap_or(false) && !h["key"].as_str().unwrap_or("").is_empty()).map(|h| (session.scopes.resolve(h["key"].as_str().unwrap_or("")), session.scopes.resolve(h["value"].as_str().unwrap_or("")))).collect())
                    .unwrap_or_default();
                let client = WsClient::connect(&session.scopes.resolve(body["url"].as_str().unwrap_or("")), &headers).map_err(|e| anyhow!("400:{e}"))?;
                let id = rspostman_core::model::new_id();
                self.sockets.lock().map_err(|_| lock_err())?.insert(id.clone(), client);
                ok(json!({ "id": id }))
            }
            ("GET", ["ws", id]) => {
                let sockets = self.sockets.lock().map_err(|_| lock_err())?;
                let c = sockets.get(*id).ok_or_else(|| not_found("connection"))?;
                let since = req.param("since").and_then(|s| s.parse().ok()).unwrap_or(0);
                ok(json!({"open": c.is_open(), "url": c.url, "events": c.events_since(since)}))
            }
            ("POST", ["ws", id, "send"]) => {
                let body: Value = req.json()?;
                let sockets = self.sockets.lock().map_err(|_| lock_err())?;
                let c = sockets.get(*id).ok_or_else(|| not_found("connection"))?;
                if !c.is_open() {
                    bail!("400:connection is closed");
                }
                c.send(body["text"].as_str().unwrap_or("")).map_err(|e| anyhow!("400:{e}"))?;
                ok(json!({"sent": true}))
            }
            ("POST", ["ws", id, "close"]) => {
                let sockets = self.sockets.lock().map_err(|_| lock_err())?;
                sockets.get(*id).ok_or_else(|| not_found("connection"))?.close();
                ok(json!({"closing": true}))
            }
            ("DELETE", ["ws", id]) => {
                if let Some(c) = self.sockets.lock().map_err(|_| lock_err())?.remove(*id) {
                    c.close();
                }
                ok(json!({"removed": true}))
            }
            _ => Err(not_found("endpoint")),
        }
    }

    fn handle(self: &Arc<Self>, req: Req) -> Resp {
        // Access log for troubleshooting: RSPOSTMAN_LOG=1 rspostman gui
        if std::env::var_os("RSPOSTMAN_LOG").is_none() {
            return self.dispatch(req);
        }
        let started = std::time::Instant::now();
        let line = format!("{} {}", req.method, req.path);
        eprintln!("--> {line}");
        let resp = self.dispatch(req);
        eprintln!("<-- {line} {} {}ms", resp.status, started.elapsed().as_millis());
        resp
    }

    fn dispatch(self: &Arc<Self>, req: Req) -> Resp {
        // DNS rebinding protection: only answer requests addressed to a loopback name.
        let host = req.header("host").unwrap_or("").to_lowercase();
        let hostname = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(&host);
        if !["127.0.0.1", "localhost", "[::1]"].contains(&hostname) {
            return Resp::text(403, "forbidden: this server only accepts requests for localhost");
        }
        if req.path.starts_with("/api/") {
            if req.method != "GET" && req.header("x-requested-with") != Some("rspostman") {
                return Resp::json(403, &json!({"error": "missing X-Requested-With: rspostman header"}));
            }
            return match self.api(&req) {
                Ok(r) => r.with("Cache-Control", "no-store"),
                Err(e) => {
                    let msg = format!("{e:#}");
                    let (status, text) = match msg.split_once(':') {
                        Some((code, rest)) if code.len() == 3 && code.chars().all(|c| c.is_ascii_digit()) => (code.parse().unwrap_or(500), rest.trim().to_string()),
                        _ if msg.contains("invalid JSON body") || msg.contains("missing field") || msg.contains("invalid type") => (400, msg),
                        _ => (500, msg),
                    };
                    Resp::json(status, &json!({ "error": text }))
                }
            };
        }
        if req.method != "GET" && req.method != "HEAD" {
            return Resp::text(405, "method not allowed");
        }
        let rel = if req.path == "/" { "index.html" } else { req.path.trim_start_matches('/') };
        if rel.contains("..") {
            return Resp::text(400, "bad path");
        }
        // Development aid: serve the UI from disk when RSPOSTMAN_UI_DIR is set.
        if let Some(dir) = std::env::var_os("RSPOSTMAN_UI_DIR") {
            return match std::fs::read(std::path::Path::new(&dir).join(rel)) {
                Ok(bytes) => Resp::new(200, mime(rel), bytes).with("Cache-Control", "no-store"),
                Err(_) => Resp::text(404, "not found"),
            };
        }
        match UI.get_file(rel) {
            Some(f) => Resp::new(200, mime(rel), f.contents().to_vec()).with("Cache-Control", "no-cache").with("X-Content-Type-Options", "nosniff"),
            None => Resp::text(404, "not found"),
        }
    }
}

fn mock_json(id: &str, m: &MockEntry) -> Value {
    json!({
        "id": id,
        "url": m.handle.url(),
        "port": m.handle.port,
        "collection_id": m.collection_id,
        "collection_name": m.collection_name,
        "routes": m.routes.iter().map(|r| json!({"method": r.method, "path": r.pattern(), "request": r.request_name, "example": r.example.name, "code": r.example.code})).collect::<Vec<_>>(),
        "log": m.log.lock().map(|l| l.clone()).unwrap_or_default(),
    })
}

/// Start the GUI backend on a loopback address (`port` 0 = any free port).
pub fn start(ws: Workspace, port: u16) -> Result<ServerHandle> {
    let app = App::new(ws);
    let handler_app = app.clone();
    let handle = serve("127.0.0.1", port, 16, move |req| handler_app.handle(req))?;
    app.port.store(handle.port, Ordering::SeqCst);
    Ok(handle)
}
