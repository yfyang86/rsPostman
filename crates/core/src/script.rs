//! Pre-request / test script execution in an embedded JavaScript engine (Boa).
//! The `pm.*` API itself lives in `sandbox.js`; this module feeds it and reads the results back.

use crate::http::{BodySpec, Prepared, Response, Settings};
use crate::model::{Body, Request, Url, KV};
use crate::vars::Scopes;
use boa_engine::property::Attribute;
use boa_engine::{js_string, Context, JsResult, JsString, JsValue, NativeFunction, Source};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;

const PRELUDE: &str = include_str!("sandbox.js");

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub skipped: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LogLine {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Info {
    #[serde(rename = "eventName")]
    pub event_name: String,
    pub iteration: usize,
    #[serde(rename = "iterationCount")]
    pub iteration_count: usize,
    #[serde(rename = "requestName")]
    pub request_name: String,
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(rename = "collectionName")]
    pub collection_name: String,
    pub environment: String,
}

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub tests: Vec<TestResult>,
    pub console: Vec<LogLine>,
    /// `Some(None)` = stop the run, `Some(Some(name))` = jump to that request.
    pub next: Option<Option<String>>,
    pub skip: bool,
    /// Uncaught error (syntax or runtime).
    pub error: Option<String>,
    pub visualizer: Option<Value>,
}

thread_local! {
    static SEND_SETTINGS: RefCell<Settings> = RefCell::new(Settings::default());
}

fn arg_string(args: &[JsValue], ctx: &mut Context) -> JsResult<String> {
    Ok(args.first().cloned().unwrap_or_default().to_string(ctx)?.to_std_string_escaped())
}

fn js(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

fn response_json(r: &Response) -> Value {
    json!({
        "code": r.status,
        "status": r.status_text,
        "headers": r.headers.iter().map(|(k, v)| json!({"key": k, "value": v})).collect::<Vec<_>>(),
        "body": r.text(),
        "responseTime": r.time_ms,
        "responseSize": r.size,
        "cookies": r.cookies.iter().map(|c| json!({"key": c.name, "value": c.value, "name": c.name, "domain": c.domain, "path": c.path})).collect::<Vec<_>>(),
    })
}

/// `pm.sendRequest` backend.
fn native_send(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let spec: Value = serde_json::from_str(&arg_string(args, ctx)?).unwrap_or(Value::Null);
    let result = (|| -> anyhow::Result<Response> {
        let mut p = Prepared {
            method: spec["method"].as_str().unwrap_or("GET").to_string(),
            url: spec["url"].as_str().unwrap_or("").to_string(),
            ..Default::default()
        };
        if !p.url.contains("://") {
            p.url = format!("http://{}", p.url);
        }
        for h in spec["headers"].as_array().cloned().unwrap_or_default() {
            p.headers.push((h[0].as_str().unwrap_or("").to_string(), h[1].as_str().unwrap_or("").to_string()));
        }
        let fields = |v: &Value| -> Vec<(String, String)> {
            v.as_array().map(|a| a.iter().map(|f| (f[0].as_str().unwrap_or("").to_string(), f[1].as_str().unwrap_or("").to_string())).collect()).unwrap_or_default()
        };
        let body = &spec["body"];
        let mut language = body["language"].as_str().unwrap_or("text").to_string();
        match body["mode"].as_str() {
            Some("raw") => {
                let text = body["raw"].as_str().unwrap_or("").to_string();
                if language == "text" && serde_json::from_str::<Value>(&text).map(|v| v.is_object() || v.is_array()).unwrap_or(false) {
                    language = "json".into();
                }
                if !text.is_empty() {
                    p.body = BodySpec::Raw { text };
                }
            }
            Some("urlencoded") => p.body = BodySpec::Urlencoded { fields: fields(&body["fields"]) },
            Some("formdata") => {
                p.body = BodySpec::Form {
                    boundary: format!("----rsPostman{}", crate::model::new_id().replace('-', "")),
                    parts: fields(&body["fields"]).into_iter().map(|(name, value)| crate::http::FormPart { name, value, file: false, content_type: None }).collect(),
                }
            }
            _ => {}
        }
        if p.header("content-type").is_none() {
            if let Some(ct) = p.body.content_type(&language) {
                p.headers.push(("Content-Type".into(), ct));
            }
        }
        let settings = SEND_SETTINGS.with(|s| s.borrow().clone());
        crate::http::execute(&p, &settings, None)
    })();
    let out = match result {
        Ok(r) => json!({ "response": response_json(&r) }),
        Err(e) => json!({ "error": e.to_string() }),
    };
    Ok(js(&out.to_string()))
}

fn native_dynamic(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let name = arg_string(args, ctx)?;
    Ok(js(&crate::vars::dynamic(&name).unwrap_or_default()))
}

fn rows(list: &[KV]) -> Value {
    Value::Array(list.iter().map(|k| json!({"key": k.key, "value": k.value, "disabled": k.disabled})).collect())
}

fn request_json(req: &Request) -> Value {
    json!({
        "method": req.method,
        "url": req.url.raw,
        "headers": rows(&req.header),
        "body": req.body.as_ref().map(|b| json!({
            "mode": b.mode,
            "raw": b.raw.clone().unwrap_or_default(),
            "urlencoded": rows(&b.urlencoded),
            "formdata": rows(&b.formdata),
            "graphql": b.graphql,
            "options": b.options,
        })),
        "auth": req.auth.as_ref().map(|a| json!({"type": a.kind})),
    })
}

fn kv_rows(v: &Value) -> Vec<KV> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|r| KV {
                    key: r["key"].as_str().unwrap_or("").to_string(),
                    value: crate::model::value_to_string(&r["value"]),
                    disabled: r["disabled"].as_bool().unwrap_or(false),
                    ..Default::default()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Write request changes made by a pre-request script back into the model.
fn apply_request(req: &mut Request, v: &Value) {
    if let Some(m) = v["method"].as_str() {
        req.method = m.to_string();
    }
    if let Some(u) = v["url"].as_str() {
        if u != req.url.raw {
            let old = std::mem::take(&mut req.url.variable);
            req.url = Url::parse(u);
            for slot in req.url.variable.iter_mut() {
                if let Some(o) = old.iter().find(|o| o.key == slot.key) {
                    *slot = o.clone();
                }
            }
        }
    }
    let headers = kv_rows(&v["headers"]);
    if headers.iter().map(|h| (&h.key, &h.value, h.disabled)).ne(req.header.iter().map(|h| (&h.key, &h.value, h.disabled))) {
        req.header = headers;
    }
    let b = &v["body"];
    if b.is_object() {
        let body = req.body.get_or_insert_with(Body::default);
        if let Some(mode) = b["mode"].as_str() {
            body.mode = mode.to_string();
        }
        if body.mode == "raw" {
            body.raw = Some(b["raw"].as_str().unwrap_or("").to_string());
        }
        for (field, target) in [("urlencoded", &mut body.urlencoded), ("formdata", &mut body.formdata)] {
            let new = kv_rows(&b[field]);
            if new.iter().map(|h| (&h.key, &h.value, h.disabled)).ne(target.iter().map(|h| (&h.key, &h.value, h.disabled))) {
                // keep file rows (not visible to scripts) untouched when nothing changed
                *target = new;
            }
        }
    }
}

pub struct ScriptContext<'a> {
    pub scopes: &'a mut Scopes,
    /// The (unresolved) request; pre-request scripts may modify it.
    pub request: &'a mut Request,
    pub response: Option<&'a Response>,
    pub info: Info,
    pub settings: &'a Settings,
    /// Cookies that apply to the request URL.
    pub cookies: Vec<(String, String)>,
}

fn eval_string(ctx: &mut Context, code: &str) -> Result<String, String> {
    match ctx.eval(Source::from_bytes(code)) {
        Ok(v) => v.to_string(ctx).map(|s| s.to_std_string_escaped()).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Run one script. Never panics on script errors; they are reported in `Outcome::error`.
pub fn run(code: &str, sc: ScriptContext) -> Outcome {
    let mut outcome = Outcome::default();
    if code.trim().is_empty() {
        return outcome;
    }
    SEND_SETTINGS.with(|s| *s.borrow_mut() = sc.settings.clone());
    let input = json!({
        "scopes": sc.scopes,
        "request": request_json(sc.request),
        "response": sc.response.map(response_json),
        "info": sc.info,
        "cookies": sc.cookies.iter().map(|(k, v)| json!({"key": k, "value": v})).collect::<Vec<_>>(),
    })
    .to_string();

    let mut ctx = Context::default();
    ctx.runtime_limits_mut().set_loop_iteration_limit(5_000_000);
    ctx.runtime_limits_mut().set_recursion_limit(2_000);
    let setup = (|| -> JsResult<()> {
        ctx.register_global_property(js_string!("__input"), js(&input), Attribute::all())?;
        ctx.register_global_callable(js_string!("__send"), 1, NativeFunction::from_fn_ptr(native_send))?;
        ctx.register_global_callable(js_string!("__dynamic"), 1, NativeFunction::from_fn_ptr(native_dynamic))?;
        ctx.eval(Source::from_bytes(PRELUDE))?;
        Ok(())
    })();
    if let Err(e) = setup {
        outcome.error = Some(format!("sandbox initialisation failed: {e}"));
        return outcome;
    }

    // Async wrapper: allows top-level `await` and `return`.
    let wrapped = format!("(async function () {{\n{code}\n}})().then(undefined, function (e) {{ globalThis.__uncaught = {{ e: e }}; }});");
    match ctx.eval(Source::from_bytes(&wrapped)) {
        Err(e) => outcome.error = Some(e.to_string()),
        Ok(_) => {
            ctx.run_jobs();
            for _ in 0..10_000 {
                match eval_string(&mut ctx, "String(__runTimers())") {
                    Ok(s) if s == "true" => ctx.run_jobs(),
                    _ => break,
                }
            }
            let uncaught = eval_string(
                &mut ctx,
                "(function(){ const u = globalThis.__uncaught; if (!u || __skipSignal(u.e)) return ''; const e = u.e; \
                 if (e && e.name === 'AssertionError') return 'AssertionError: ' + e.message; \
                 if (e instanceof Error) return e.name + ': ' + e.message; \
                 try { return typeof e === 'string' ? e : JSON.stringify(e) } catch (x) { return String(e) } })()",
            );
            match uncaught {
                Ok(s) if !s.is_empty() => outcome.error = Some(s),
                Err(e) => outcome.error = Some(e),
                _ => {}
            }
        }
    }

    match eval_string(&mut ctx, "__finish()").map(|s| serde_json::from_str::<Value>(&s)) {
        Ok(Ok(v)) => {
            if let Ok(scopes) = serde_json::from_value::<Scopes>(v["scopes"].clone()) {
                *sc.scopes = scopes;
            }
            outcome.tests = serde_json::from_value(v["tests"].clone()).unwrap_or_default();
            outcome.console = serde_json::from_value(v["console"].clone()).unwrap_or_default();
            if v["next"]["set"].as_bool().unwrap_or(false) {
                outcome.next = Some(v["next"]["name"].as_str().map(|s| s.to_string()));
            }
            outcome.skip = v["skip"].as_bool().unwrap_or(false);
            if !v["visualizer"].is_null() {
                outcome.visualizer = Some(v["visualizer"].clone());
            }
            if sc.info.event_name == "prerequest" {
                apply_request(sc.request, &v["request"]);
            }
        }
        Ok(Err(e)) => outcome.error.get_or_insert(format!("sandbox returned invalid state: {e}")).truncate(2000),
        Err(e) => {
            outcome.error.get_or_insert(e);
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec(code: &str, phase: &str, response: Option<&Response>) -> (Outcome, Scopes, Request) {
        let mut scopes = Scopes::default();
        scopes.environment.insert("host".into(), "example.test".into());
        scopes.globals.insert("g".into(), "G".into());
        scopes.data.insert("row".into(), "7".into());
        let mut req = Request::new("POST", "https://{{host}}/v1/users?page=2");
        req.header.push(KV::new("Accept", "application/json"));
        req.body = Some(Body::raw("{\"a\":1}", "json"));
        let o = run(
            code,
            ScriptContext {
                scopes: &mut scopes,
                request: &mut req,
                response,
                info: Info { event_name: phase.into(), iteration: 2, iteration_count: 5, request_name: "Create user".into(), request_id: "id1".into(), ..Default::default() },
                settings: &Settings::default(),
                cookies: vec![("sid".into(), "abc".into())],
            },
        );
        (o, scopes, req)
    }

    fn sample_response() -> Response {
        Response {
            status: 201,
            status_text: "Created".into(),
            headers: vec![("Content-Type".into(), "application/json".into()), ("X-Rate".into(), "9".into())],
            body: br#"{"id": 42, "name": "Ann", "tags": ["a", "b"], "profile": {"age": 30, "city": null}}"#.to_vec(),
            time_ms: 123,
            size: 80,
            ..Default::default()
        }
    }

    fn names(o: &Outcome, passed: bool) -> Vec<&str> {
        o.tests.iter().filter(|t| t.passed == passed).map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn variables_and_request_mutation() {
        let (o, sc, req) = exec(
            r#"
            pm.environment.set("token", "t-" + pm.variables.get("row"));
            pm.globals.set("count", 3);
            pm.collectionVariables.set("obj", {a: [1, 2]});
            pm.variables.set("tmp", "local");
            pm.globals.unset("g");
            console.log("host is", pm.environment.get("host"), {x: 1}, [1, "two"], null, undefined);
            console.warn(pm.variables.replaceIn("{{host}}/{{tmp}}/{{missing}}"), pm.variables.replaceIn("{{$guid}}").length);
            pm.request.headers.add({key: "X-Trace", value: "abc"});
            pm.request.headers.upsert({key: "accept", value: "text/plain"});
            pm.request.url.addQueryParams("debug=1");
            pm.request.method = "put";
            pm.request.body.raw = JSON.stringify({a: 2});
            pm.test("info", () => {
                pm.expect(pm.info.eventName).to.equal("prerequest");
                pm.expect(pm.info.iteration).to.equal(2);
                pm.expect(pm.info.requestName).to.equal("Create user");
                pm.expect(pm.iterationData.get("row")).to.equal("7");
                pm.expect(pm.request.url.getHost()).to.equal("{{host}}");
                pm.expect(pm.request.url.query.get("page")).to.equal("2");
                pm.expect(pm.cookies.get("sid")).to.equal("abc");
                pm.expect(pm.response).to.be.undefined;
            });
            "#,
            "prerequest",
            None,
        );
        assert_eq!(o.error, None);
        assert_eq!(names(&o, false), Vec::<&str>::new(), "{:?}", o.tests);
        assert_eq!(sc.environment["token"], "t-7");
        assert_eq!(sc.globals.get("count").unwrap(), "3");
        assert!(!sc.globals.contains_key("g"));
        assert_eq!(sc.collection["obj"], r#"{"a":[1,2]}"#);
        assert_eq!(sc.local["tmp"], "local");
        assert_eq!(o.console[0].message, r#"host is example.test {"x":1} [1,"two"] null undefined"#);
        assert_eq!(o.console[1].message, "example.test/local/{{missing}} 36");
        assert_eq!(o.console[1].level, "warn");
        assert_eq!(req.method, "PUT");
        assert_eq!(req.url.raw, "https://{{host}}/v1/users?page=2&debug=1");
        assert_eq!(req.url.query.len(), 2);
        assert_eq!(req.header.iter().map(|h| format!("{}={}", h.key, h.value)).collect::<Vec<_>>(), ["Accept=text/plain", "X-Trace=abc"]);
        assert_eq!(req.body.unwrap().raw.unwrap(), r#"{"a":2}"#);
    }

    #[test]
    fn response_assertions_and_expect() {
        let r = sample_response();
        let (o, _, _) = exec(
            r#"
            const j = pm.response.json();
            pm.test("status", () => {
                pm.response.to.have.status(201);
                pm.response.to.have.status("Created");
                pm.response.to.be.success;
                pm.response.to.not.be.error;
                pm.response.to.be.json;
                pm.response.to.have.header("content-type");
                pm.response.to.have.header("X-Rate", "9");
                pm.response.to.not.have.header("X-Nope");
                pm.response.to.have.jsonBody("profile.age", 30);
                pm.response.to.have.body(/Ann/);
                pm.expect(pm.response.code).to.be.oneOf([200, 201]);
                pm.expect(pm.response.responseTime).to.be.below(200);
                pm.expect(pm.response.headers.get("x-rate")).to.eql("9");
                pm.expect(pm.response.text()).to.include("Ann");
            });
            pm.test("expect matrix", () => {
                pm.expect(j.id).to.equal(42).and.to.be.a("number");
                pm.expect(j).to.have.property("name", "Ann");
                pm.expect(j).to.have.property("profile").that.is.an("object").with.property("age").above(18);
                pm.expect(j).to.have.nested.property("profile.age", 30);
                pm.expect(j.tags).to.be.an("array").that.includes("a").and.has.lengthOf(2);
                pm.expect(j.tags).to.have.length.above(1);
                pm.expect(j.tags).to.have.members(["b", "a"]);
                pm.expect(j.tags).to.deep.equal(["a", "b"]);
                pm.expect(j).to.include({id: 42});
                pm.expect(j).to.deep.include({tags: ["a", "b"]});
                pm.expect(j).to.have.all.keys("id", "name", "tags", "profile");
                pm.expect(j).to.include.keys("id");
                pm.expect(j).to.have.any.keys("zzz", "id");
                pm.expect(j.profile.city).to.be.null;
                pm.expect(j.nope).to.be.undefined;
                pm.expect(j.nope).to.not.exist;
                pm.expect(j.name).to.match(/^A/).and.have.string("nn");
                pm.expect("").to.be.empty;
                pm.expect([1]).to.not.be.empty;
                pm.expect(1.05).to.be.closeTo(1, 0.1);
                pm.expect(5).to.be.within(1, 10).and.at.least(5).and.at.most(5);
                pm.expect(true).to.be.true; pm.expect(0).to.not.be.ok; pm.expect(NaN).to.be.NaN;
                pm.expect(() => { throw new TypeError("boom") }).to.throw(TypeError);
                pm.expect(() => { throw new Error("boom") }).to.throw("boom");
                pm.expect(() => 1).to.not.throw();
                pm.expect(new Date(0)).to.be.instanceOf(Date);
                pm.expect(j).to.be.jsonSchema({type: "object", required: ["id"], properties: {id: {type: "integer"}, tags: {type: "array", items: {type: "string"}}}});
                pm.expect(j).to.satisfy(x => x.id > 0);
            });
            pm.test("fails with message", () => { pm.expect(j.id, "id check").to.equal(43); });
            pm.test("negation fails", () => { pm.expect(j.tags).to.not.include("a"); });
            pm.test("status mismatch", () => { pm.response.to.have.status(200); });
            pm.test("schema mismatch", () => { pm.response.to.have.jsonSchema({required: ["zip"], properties: {id: {type: "string"}}}); });
            pm.test("throws plain", () => { null.x; });
            pm.test.skip("skipped one", () => {});
            pm.test("async ok", async () => { const v = await Promise.resolve(2); pm.expect(v).to.equal(2); });
            pm.test("async fail", async () => { await Promise.resolve(); pm.expect(1).to.equal(2); });
            pm.test("callback", (done) => { setTimeout(() => done(), 10); });
            tests["legacy pass"] = responseCode.code === 201;
            tests["legacy fail"] = responseBody.indexOf("zzz") !== -1;
            pm.visualizer.set("<b>{{name}}</b>", {name: j.name});
            "#,
            "test",
            Some(&r),
        );
        assert_eq!(o.error, None);
        assert_eq!(names(&o, true), ["status", "expect matrix", "skipped one", "async ok", "callback", "legacy pass"], "{:#?}", o.tests);
        assert_eq!(names(&o, false), ["fails with message", "negation fails", "status mismatch", "schema mismatch", "throws plain", "async fail", "legacy fail"]);
        let err = |n: &str| o.tests.iter().find(|t| t.name == n).unwrap().error.clone().unwrap();
        assert_eq!(err("fails with message"), "id check: expected 42 to equal 43");
        assert_eq!(err("negation fails"), r#"expected ["a","b"] to not include "a""#);
        assert_eq!(err("status mismatch"), "expected response to have status code 200 but got 201");
        assert!(err("schema mismatch").contains("missing required property \"zip\"") && err("schema mismatch").contains("data.id should be string"));
        assert!(err("throws plain").starts_with("TypeError"));
        assert_eq!(err("async fail"), "expected 1 to equal 2");
        assert!(o.tests.iter().find(|t| t.name == "skipped one").unwrap().skipped);
        assert_eq!(o.visualizer.unwrap()["data"]["name"], "Ann");
    }

    #[test]
    fn flow_control_errors_and_limits() {
        let (o, _, _) = exec("pm.execution.setNextRequest('Other'); postman.setNextRequest(null);", "test", None);
        assert_eq!(o.next, Some(None));
        let (o, _, _) = exec("pm.setNextRequest('Login')", "test", None);
        assert_eq!(o.next, Some(Some("Login".into())));
        let (o, _, _) = exec("pm.test('a', () => {}); pm.execution.skipRequest(); pm.test('b', () => {});", "prerequest", None);
        assert!(o.skip && o.error.is_none());
        assert_eq!(o.tests.len(), 1);

        let (o, sc, _) = exec("pm.environment.set('before', '1'); undefinedFunction();", "test", None);
        assert!(o.error.as_ref().unwrap().contains("ReferenceError"), "{:?}", o.error);
        assert_eq!(sc.environment["before"], "1", "state before the error is kept");
        assert!(exec("let x = ;", "test", None).0.error.unwrap().contains("SyntaxError"));
        assert!(exec("while (true) {}", "test", None).0.error.unwrap().contains("limit"));
        assert!(exec("throw new Error('custom')", "test", None).0.error.unwrap().contains("Error: custom"));
        assert!(exec("require('lodash')", "test", None).0.error.unwrap().contains("Cannot find module"));
        let (o, _, _) = exec("const {v4} = require('uuid'); console.log(v4().length, btoa('hi'), atob('aGk=')); return; console.log('never')", "test", None);
        assert_eq!((o.error, o.console[0].message.as_str(), o.console.len()), (None, "36 aGk= hi", 1));
    }

    #[test]
    fn send_request_from_script() {
        let srv = crate::echo::start("127.0.0.1", 0).unwrap();
        let code = format!(
            r#"
            pm.sendRequest("{base}/get?from=script", (err, res) => {{
                pm.test("callback form", () => {{
                    pm.expect(err).to.be.null;
                    pm.expect(res.code).to.equal(200);
                    pm.expect(res.json().args.from).to.equal("script");
                }});
                pm.environment.set("chained", res.json().args.from);
            }});
            const res = await pm.sendRequest({{url: "{base}/post", method: "POST", header: {{"X-A": "1"}}, body: {{mode: "raw", raw: JSON.stringify({{n: 5}})}}}});
            pm.test("promise form", () => {{
                pm.expect(res.json().json.n).to.equal(5);
                pm.expect(res.json().headers["x-a"]).to.equal("1");
                pm.expect(res.json().headers["content-type"]).to.equal("application/json");
            }});
            const form = await pm.sendRequest({{url: "{base}/post", method: "POST", body: {{mode: "urlencoded", urlencoded: [{{key: "k", value: "{{{{host}}}}"}}]}}}});
            pm.test("urlencoded", () => pm.expect(form.json().form.k).to.equal("example.test"));
            pm.sendRequest("http://127.0.0.1:1/", (err, res) => {{
                pm.test("network error is reported", () => {{ pm.expect(err).to.be.an("error"); pm.expect(res).to.be.undefined; }});
            }});
            "#,
            base = srv.url()
        );
        let mut scopes = Scopes::default();
        scopes.environment.insert("host".into(), "example.test".into());
        let mut req = Request::new("GET", "http://x");
        let o = run(
            &code,
            ScriptContext { scopes: &mut scopes, request: &mut req, response: None, info: Info { event_name: "prerequest".into(), ..Default::default() }, settings: &Settings { no_system_proxy: true, ..Default::default() }, cookies: vec![] },
        );
        assert_eq!(o.error, None);
        assert_eq!(o.tests.len(), 4);
        assert!(o.tests.iter().all(|t| t.passed), "{:#?}", o.tests);
        assert_eq!(scopes.environment["chained"], "script");
    }
}
