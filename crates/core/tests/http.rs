//! End-to-end HTTP tests against the built-in echo server.

use rspostman_core::http::{execute, prepare, Settings};
use rspostman_core::model::{Auth, Body, Collection, Item, Request, KV};
use rspostman_core::vars::Scopes;
use rspostman_core::{echo, Session, Workspace};
use serde_json::json;

fn settings() -> Settings {
    Settings { no_system_proxy: true, ..Default::default() }
}

fn send(req: &Request, auth: Option<&Auth>, sc: &Scopes) -> rspostman_core::http::Response {
    execute(&prepare(req, auth, sc).unwrap(), &settings(), None).unwrap()
}

fn scopes(base: &str) -> Scopes {
    let mut sc = Scopes::default();
    sc.environment.insert("base".into(), base.into());
    sc.environment.insert("who".into(), "wörld & co".into());
    sc.environment.insert("ascii".into(), "plain value".into());
    sc
}

#[test]
fn query_path_variables_and_headers() {
    let srv = echo::start("127.0.0.1", 0).unwrap();
    let sc = scopes(&srv.url());
    let mut req = Request::new("get", "{{base}}/anything/:kind/x?greet={{who}}&plain=1");
    req.url.variable[0].value = "a b".into();
    req.url.query.push(KV { key: "off".into(), value: "1".into(), disabled: true, ..Default::default() });
    req.header.push(KV::new("X-Who", "{{ascii}}"));
    req.header.push(KV::new("X-Utf8", "{{who}}"));
    req.header.push(KV { key: "X-Off".into(), value: "1".into(), disabled: true, ..Default::default() });
    let r = send(&req, None, &sc);
    assert_eq!(r.status, 200);
    let j = r.json().unwrap();
    assert_eq!(j["path"], "/anything/a b/x");
    assert_eq!(j["args"], json!({"greet": "wörld & co", "plain": "1"}));
    assert_eq!(j["headers"]["x-who"], "plain value");
    assert_eq!(j["headers"]["x-utf8"], "wörld & co");
    assert!(j["headers"].get("x-off").is_none());
    assert!(j["headers"]["user-agent"].as_str().unwrap().starts_with("rsPostman/"));
    assert!(r.is_text() && r.size > 0);
}

#[test]
fn bodies() {
    let srv = echo::start("127.0.0.1", 0).unwrap();
    let sc = scopes(&srv.url());

    let mut req = Request::new("POST", "{{base}}/post");
    req.body = Some(Body::raw("{\"hello\": \"{{who}}\"}", "json"));
    let j = send(&req, None, &sc).json().unwrap();
    assert_eq!(j["json"], json!({"hello": "wörld & co"}));
    assert_eq!(j["headers"]["content-type"], "application/json");

    req.body = Some(Body { mode: "urlencoded".into(), urlencoded: vec![KV::new("a", "1 + 1"), KV::new("b", "{{who}}")], ..Default::default() });
    let j = send(&req, None, &sc).json().unwrap();
    assert_eq!(j["form"], json!({"a": "1 + 1", "b": "wörld & co"}));

    let file = std::env::temp_dir().join(format!("rsp-{}.txt", rspostman_core::model::new_id()));
    std::fs::write(&file, "file-content").unwrap();
    let mut f = KV::new("upload", "");
    f.kind = Some("file".into());
    f.src = Some(json!(file.to_str().unwrap()));
    req.body = Some(Body { mode: "formdata".into(), formdata: vec![KV::new("field", "{{who}}"), f], ..Default::default() });
    let j = send(&req, None, &sc).json().unwrap();
    assert_eq!(j["form"], json!({"field": "wörld & co"}));
    assert_eq!(j["files"]["upload"]["text"], "file-content");
    assert_eq!(j["files"]["upload"]["content_type"], "text/plain");

    req.body = Some(Body { mode: "file".into(), file: Some(json!({"src": file.to_str().unwrap()})), ..Default::default() });
    assert_eq!(send(&req, None, &sc).json().unwrap()["data"], "file-content");

    req.body = Some(Body {
        mode: "graphql".into(),
        graphql: Some(rspostman_core::model::GraphQl { query: "query($id: ID!) { user(id: $id) { name } }".into(), variables: "{\"id\": 7}".into() }),
        ..Default::default()
    });
    let j = send(&req, None, &sc).json().unwrap();
    assert_eq!(j["json"]["variables"], json!({"id": 7}));
    assert!(j["json"]["query"].as_str().unwrap().contains("user(id"));
    std::fs::remove_file(file).ok();
}

#[test]
fn auth_schemes() {
    let srv = echo::start("127.0.0.1", 0).unwrap();
    let sc = scopes(&srv.url());
    let basic = Auth::new("basic", &[("username", "bob"), ("password", "s3cret")]);
    assert_eq!(send(&Request::new("GET", "{{base}}/basic-auth/bob/s3cret"), Some(&basic), &sc).status, 200);
    assert_eq!(send(&Request::new("GET", "{{base}}/basic-auth/bob/other"), Some(&basic), &sc).status, 401);

    let bearer = Auth::new("bearer", &[("token", "tok-{{ascii}}")]);
    let r = send(&Request::new("GET", "{{base}}/bearer"), Some(&bearer), &sc);
    assert_eq!(r.json().unwrap()["token"], "tok-plain value");

    let digest = Auth::new("digest", &[("username", "dan"), ("password", "pw")]);
    assert_eq!(send(&Request::new("GET", "{{base}}/digest-auth/dan/pw?x=1"), Some(&digest), &sc).status, 200);
    assert_eq!(send(&Request::new("GET", "{{base}}/digest-auth/dan/nope"), Some(&digest), &sc).status, 401);

    let key = Auth::new("apikey", &[("key", "X-Api-Key"), ("value", "k1"), ("in", "header")]);
    assert_eq!(send(&Request::new("GET", "{{base}}/headers"), Some(&key), &sc).json().unwrap()["headers"]["x-api-key"], "k1");
    let key = Auth::new("apikey", &[("key", "api_key"), ("value", "k2"), ("in", "query")]);
    assert_eq!(send(&Request::new("GET", "{{base}}/get?a=1"), Some(&key), &sc).json().unwrap()["args"], json!({"a": "1", "api_key": "k2"}));

    let aws = Auth::new("awsv4", &[("accessKey", "AK"), ("secretKey", "SK"), ("region", "eu-west-1"), ("service", "s3")]);
    let h = send(&Request::new("GET", "{{base}}/headers"), Some(&aws), &sc).json().unwrap();
    assert!(h["headers"]["authorization"].as_str().unwrap().starts_with("AWS4-HMAC-SHA256 Credential=AK/"));
    assert!(h["headers"]["x-amz-date"].is_string());

    let o1 = Auth::new("oauth1", &[("consumerKey", "ck"), ("consumerSecret", "cs")]);
    let h = send(&Request::new("GET", "{{base}}/headers"), Some(&o1), &sc).json().unwrap();
    assert!(h["headers"]["authorization"].as_str().unwrap().contains("oauth_signature="));
}

#[test]
fn redirects_cookies_and_errors() {
    let srv = echo::start("127.0.0.1", 0).unwrap();
    let sc = scopes(&srv.url());
    let r = send(&Request::new("GET", "{{base}}/redirect/3"), None, &sc);
    assert_eq!((r.status, r.redirects.len()), (200, 3));
    assert!(r.url.ends_with("/get"));

    let p = prepare(&Request::new("GET", "{{base}}/redirect/3"), None, &sc).unwrap();
    let st = Settings { follow_redirects: false, ..settings() };
    assert_eq!(execute(&p, &st, None).unwrap().status, 302);

    // POST -> 302 becomes GET
    let mut post = Request::new("POST", "{{base}}/redirect-to?url=/get");
    post.body = Some(Body::raw("x", "text"));
    assert_eq!(send(&post, None, &sc).json().unwrap()["method"], "GET");

    let mut jar = rspostman_core::cookies::CookieJar::default();
    let p = prepare(&Request::new("GET", "{{base}}/cookies/set?session=abc&theme=dark"), None, &sc).unwrap();
    let r = execute(&p, &settings(), Some(&mut jar)).unwrap();
    assert_eq!(r.json().unwrap()["cookies"], json!({"session": "abc", "theme": "dark"}));
    assert_eq!(r.cookies.len(), 2);
    assert_eq!(jar.cookies.len(), 2);
    let p = prepare(&Request::new("GET", "{{base}}/cookies/delete?theme"), None, &sc).unwrap();
    assert_eq!(execute(&p, &settings(), Some(&mut jar)).unwrap().json().unwrap()["cookies"], json!({"session": "abc"}));

    let p = prepare(&Request::new("GET", "{{base}}/delay/1500"), None, &sc).unwrap();
    let st = Settings { timeout_ms: 200, ..settings() };
    assert!(execute(&p, &st, None).is_err());

    assert!(prepare(&Request::new("GET", ""), None, &sc).is_err());
    let p = prepare(&Request::new("GET", "http://127.0.0.1:1/nothing"), None, &sc).unwrap();
    assert!(execute(&p, &settings(), None).is_err());
    assert_eq!(send(&Request::new("GET", "{{base}}/status/204"), None, &sc).status, 204);
    assert_eq!(send(&Request::new("HEAD", "{{base}}/anything"), None, &sc).body.len(), 0);
    assert_eq!(send(&Request::new("GET", "{{base}}/bytes/1000"), None, &sc).size, 1000);
}

#[test]
fn session_inherits_auth_and_records_history() {
    let srv = echo::start("127.0.0.1", 0).unwrap();
    let ws = Workspace::open(Some(std::env::temp_dir().join(format!("rsp-ws-{}", rspostman_core::model::new_id())))).unwrap();
    ws.save_settings(&settings()).unwrap();
    let mut c = Collection::new("API");
    c.auth = Some(Auth::new("bearer", &[("token", "{{token}}")]));
    c.variable.push(KV::new("base", srv.url()));
    c.variable.push(KV::new("token", "from-collection"));
    let item = Item::request("who", Request::new("GET", "{{base}}/bearer"));
    let id = item.id.clone();
    c.add_item(None, item).unwrap();
    ws.save_collection(&c).unwrap();

    let mut env = rspostman_core::Environment::new("dev");
    env.set("token", "from-env");
    ws.save_environment(&env).unwrap();

    let mut s = Session::load(&ws, Some(&c), None).unwrap();
    let ex = s.send_item(&c, &id);
    assert_eq!(ex.response.as_ref().unwrap().json().unwrap()["token"], "from-collection");
    let mut s = Session::load(&ws, Some(&c), Some("dev")).unwrap();
    let ex = s.send_item(&c, &id);
    assert_eq!(ex.response.as_ref().unwrap().json().unwrap()["token"], "from-env");
    rspostman_core::engine::record(&ws, &s.settings, &ex, c.find(&id).unwrap().request.as_ref(), Some(c.id())).unwrap();
    s.persist(&ws, None).unwrap();
    let h = ws.history().unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].response.as_ref().unwrap().status, 200);
    assert_eq!(h[0].source.as_ref().unwrap().url.raw, "{{base}}/bearer");
    std::fs::remove_dir_all(&ws.root).ok();
}
