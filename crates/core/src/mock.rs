//! Local mock server built from the saved example responses of a collection.

use crate::model::{Collection, Example};
use crate::server::{serve, Req, Resp, ServerHandle};
use crate::vars::Scopes;
use anyhow::{bail, Result};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct MockRoute {
    pub method: String,
    /// Path segments; `None` is a wildcard (`:id` or an unresolved `{{var}}`).
    pub segments: Vec<Option<String>>,
    pub query: Vec<(String, String)>,
    pub request_name: String,
    pub example: Example,
}

impl MockRoute {
    pub fn pattern(&self) -> String {
        format!("/{}", self.segments.iter().map(|s| s.clone().unwrap_or_else(|| "*".into())).collect::<Vec<_>>().join("/"))
    }
}

#[derive(Debug, Clone, Default)]
pub struct MockOptions {
    /// Artificial delay before every response.
    pub delay_ms: u64,
    /// Do not add permissive CORS headers.
    pub no_cors: bool,
}

fn path_of(raw_base: &str, sc: &Scopes) -> Vec<Option<String>> {
    let resolved = sc.resolve(raw_base);
    let path = if let Some((_, rest)) = resolved.split_once("://") {
        rest.split_once('/').map(|(_, p)| p.to_string()).unwrap_or_default()
    } else if resolved.starts_with('/') {
        resolved[1..].to_string()
    } else {
        // `{{baseUrl}}/users` or `host/users`
        resolved.split_once('/').map(|(_, p)| p.to_string()).unwrap_or_default()
    };
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| if s.starts_with(':') || s.contains("{{") { None } else { Some(percent_encoding::percent_decode_str(s).decode_utf8_lossy().to_string()) })
        .collect()
}

/// One route per saved example.
pub fn routes(collection: &Collection, sc: &Scopes) -> Vec<MockRoute> {
    let mut out = vec![];
    for (_, item) in collection.requests() {
        let Some(req) = item.request.as_ref() else { continue };
        for ex in &item.response {
            let source = ex.original_request.as_ref().unwrap_or(req);
            out.push(MockRoute {
                method: sc.resolve(&source.method).to_uppercase(),
                segments: path_of(&source.url.base(), sc),
                query: source.url.query.iter().filter(|q| !q.disabled).map(|q| (sc.resolve(&q.key), sc.resolve(&q.value))).collect(),
                request_name: item.name.clone(),
                example: ex.clone(),
            });
        }
    }
    out
}

fn score(route: &MockRoute, req: &Req) -> Option<i32> {
    if route.method != req.method {
        return None;
    }
    let segs = req.segments();
    if segs.len() != route.segments.len() {
        return None;
    }
    let mut score = 0;
    for (pattern, actual) in route.segments.iter().zip(&segs) {
        match pattern {
            Some(p) if p == actual => score += 10,
            Some(_) => return None,
            None => score += 1,
        }
    }
    for (k, v) in &route.query {
        if req.query.iter().any(|(rk, rv)| rk == k && rv == v) {
            score += 3;
        }
    }
    if (200..300).contains(&route.example.code) || route.example.code == 0 {
        score += 1;
    }
    Some(score)
}

/// Pick the best example for a request and render it.
pub fn respond(routes: &[MockRoute], req: &Req, sc: &Scopes, opts: &MockOptions) -> Resp {
    let cors = |mut r: Resp| {
        if !opts.no_cors {
            r = r.with("Access-Control-Allow-Origin", "*").with("Access-Control-Allow-Headers", "*").with("Access-Control-Allow-Methods", "*");
        }
        r
    };
    if req.method == "OPTIONS" && !opts.no_cors && !routes.iter().any(|r| r.method == "OPTIONS" && score(r, req).is_some()) {
        return cors(Resp::empty(204));
    }
    let want_code: Option<u16> = req.header("x-mock-response-code").and_then(|c| c.trim().parse().ok());
    let want_name = req.header("x-mock-response-name").map(|s| s.trim().to_string());
    let mut best: Option<(i32, &MockRoute)> = None;
    for r in routes {
        let Some(mut s) = score(r, req) else { continue };
        if let Some(c) = want_code {
            if r.example.code != c {
                continue;
            }
            s += 100;
        }
        if let Some(n) = &want_name {
            if &r.example.name != n {
                continue;
            }
            s += 100;
        }
        if best.map(|(b, _)| s > b).unwrap_or(true) {
            best = Some((s, r));
        }
    }
    let Some((_, route)) = best else {
        return cors(Resp::json(
            404,
            &json!({"error": {"name": "mockRequestNotFoundError", "message": format!("No saved example matches {} {}", req.method, req.path)}}),
        ));
    };
    if opts.delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(opts.delay_ms));
    }
    let ex = &route.example;
    let mut resp = Resp { status: if ex.code == 0 { 200 } else { ex.code }, headers: vec![], body: sc.resolve(&ex.body).into_bytes() };
    for h in ex.header.iter().filter(|h| !h.disabled) {
        let k = h.key.to_lowercase();
        if ["content-length", "content-encoding", "transfer-encoding", "connection", "date", "keep-alive"].contains(&k.as_str()) {
            continue;
        }
        resp.headers.push((h.key.clone(), sc.resolve(&h.value)));
    }
    if !resp.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) && !resp.body.is_empty() {
        let ct = match ex.preview_language.as_deref() {
            Some("json") => "application/json",
            Some("xml") => "application/xml",
            Some("html") => "text/html",
            _ if serde_json::from_slice::<serde_json::Value>(&resp.body).is_ok() => "application/json",
            _ => "text/plain",
        };
        resp.headers.push(("Content-Type".into(), ct.into()));
    }
    resp.headers.push(("x-mock-example".into(), ex.name.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).collect()));
    cors(resp)
}

/// Start a mock server for `collection`. `log` receives one line per request.
pub fn start(
    collection: &Collection,
    sc: Scopes,
    host: &str,
    port: u16,
    opts: MockOptions,
    log: Option<Box<dyn Fn(String) + Send + Sync>>,
) -> Result<(ServerHandle, Vec<MockRoute>)> {
    let table = routes(collection, &sc);
    if table.is_empty() {
        bail!("collection `{}` has no saved examples to mock; save a response as an example first", collection.info.name);
    }
    let shared = Arc::new(table.clone());
    let handle = serve(host, port, 8, move |req| {
        let resp = respond(&shared, &req, &sc, &opts);
        if let Some(l) = &log {
            l(format!("{} {} -> {}", req.method, req.path, resp.status));
        }
        resp
    })?;
    Ok((handle, table))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Item, Request, KV};

    fn collection() -> Collection {
        let mut c = Collection::new("Mocked");
        let mut users = Item::request("Get user", Request::new("GET", "{{baseUrl}}/users/:id?expand=true"));
        users.response.push(Example { name: "found".into(), code: 200, status: "OK".into(), body: r#"{"id": 1, "rnd": "{{$randomInt}}", "env": "{{stage}}"}"#.into(), header: vec![KV::new("Content-Type", "application/json"), KV::new("Content-Length", "999")], ..Default::default() });
        users.response.push(Example { name: "missing".into(), code: 404, status: "Not Found".into(), body: r#"{"error": "no such user"}"#.into(), ..Default::default() });
        let mut me = Example { name: "me".into(), code: 200, body: "it's me".into(), ..Default::default() };
        me.original_request = Some(Request::new("GET", "https://api.example.com/users/me"));
        users.response.push(me);
        let mut create = Item::request("Create user", Request::new("POST", "{{baseUrl}}/users"));
        create.response.push(Example { name: "created".into(), code: 201, body: "<ok/>".into(), preview_language: Some("xml".into()), ..Default::default() });
        c.add_item(None, users).unwrap();
        c.add_item(None, create).unwrap();
        c
    }

    fn req(method: &str, path: &str, query: &[(&str, &str)], headers: &[(&str, &str)]) -> Req {
        Req {
            method: method.into(),
            path: path.into(),
            query: query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn matching() {
        let mut sc = Scopes::default();
        sc.environment.insert("stage".into(), "dev".into());
        let table = routes(&collection(), &sc);
        assert_eq!(table.iter().map(|r| r.pattern()).collect::<Vec<_>>(), ["/users/*", "/users/*", "/users/me", "/users"]);
        let o = MockOptions::default();
        let r = respond(&table, &req("GET", "/users/7", &[], &[]), &sc, &o);
        assert_eq!(r.status, 200);
        let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(body["env"], "dev");
        assert!(body["rnd"].as_str().unwrap().parse::<u32>().is_ok());
        assert!(!r.headers.iter().any(|(k, _)| k == "Content-Length"));
        assert!(r.headers.iter().any(|(k, v)| k == "Access-Control-Allow-Origin" && v == "*"));
        // literal segment beats wildcard
        assert_eq!(respond(&table, &req("GET", "/users/me", &[], &[]), &sc, &o).body, b"it's me");
        // example selection by header
        assert_eq!(respond(&table, &req("GET", "/users/7", &[], &[("x-mock-response-code", "404")]), &sc, &o).status, 404);
        assert_eq!(respond(&table, &req("GET", "/users/7", &[], &[("X-Mock-Response-Name", "missing")]), &sc, &o).status, 404);
        let created = respond(&table, &req("POST", "/users", &[], &[]), &sc, &o);
        assert_eq!(created.status, 201);
        assert!(created.headers.iter().any(|(k, v)| k == "Content-Type" && v == "application/xml"));
        assert_eq!(respond(&table, &req("DELETE", "/users/7", &[], &[]), &sc, &o).status, 404);
        assert_eq!(respond(&table, &req("GET", "/users/7/extra", &[], &[]), &sc, &o).status, 404);
        assert_eq!(respond(&table, &req("OPTIONS", "/users", &[], &[]), &sc, &o).status, 204);
    }

    #[test]
    fn serves_over_http() {
        let (srv, table) = start(&collection(), Scopes::default(), "127.0.0.1", 0, MockOptions::default(), None).unwrap();
        assert_eq!(table.len(), 4);
        let p = crate::http::Prepared { method: "POST".into(), url: format!("{}/users", srv.url()), ..Default::default() };
        let r = crate::http::execute(&p, &crate::http::Settings { no_system_proxy: true, ..Default::default() }, None).unwrap();
        assert_eq!((r.status, r.text().as_str(), r.header("x-mock-example")), (201, "<ok/>", Some("created")));
        assert!(start(&Collection::new("empty"), Scopes::default(), "127.0.0.1", 0, MockOptions::default(), None).is_err());
    }
}
