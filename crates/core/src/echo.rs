//! Built-in httpbin-like echo server. Used by the test-suite and handy for trying things offline.

use crate::server::{parse_multipart, serve_ws, Req, Resp, ServerHandle, WsStream};
use anyhow::Result;
use md5::{Digest, Md5};
use serde_json::{json, Map, Value};

fn obj(pairs: &[(String, String)]) -> Value {
    let mut m = Map::new();
    for (k, v) in pairs {
        match m.get_mut(k) {
            Some(Value::Array(a)) => a.push(json!(v)),
            Some(old) => *old = json!([old.clone(), v]),
            None => {
                m.insert(k.clone(), json!(v));
            }
        }
    }
    Value::Object(m)
}

fn cookies(req: &Req) -> Vec<(String, String)> {
    req.header("cookie")
        .unwrap_or("")
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn echo(req: &Req) -> Value {
    let ct = req.header("content-type").unwrap_or("").to_string();
    let mut form = json!({});
    let mut files = Map::new();
    if ct.starts_with("application/x-www-form-urlencoded") {
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(&req.body).map(|(k, v)| (k.to_string(), v.to_string())).collect();
        form = obj(&pairs);
    } else if ct.starts_with("multipart/form-data") {
        let mut pairs = vec![];
        for p in parse_multipart(&ct, &req.body) {
            match &p.filename {
                Some(f) => {
                    files.insert(
                        p.name.clone(),
                        json!({"filename": f, "size": p.data.len(), "content_type": p.content_type, "text": String::from_utf8_lossy(&p.data)}),
                    );
                }
                None => pairs.push((p.name.clone(), String::from_utf8_lossy(&p.data).to_string())),
            }
        }
        form = obj(&pairs);
    }
    let host = req.header("host").unwrap_or("localhost");
    let q = if req.raw_query.is_empty() { String::new() } else { format!("?{}", req.raw_query) };
    json!({
        "method": req.method,
        "url": format!("http://{host}{}{q}", req.path),
        "path": req.path,
        "args": obj(&req.query),
        "headers": obj(&req.headers.iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect::<Vec<_>>()),
        "cookies": obj(&cookies(req)),
        "data": if ct.starts_with("multipart/") { String::new() } else { req.text() },
        "json": serde_json::from_slice::<Value>(&req.body).ok(),
        "form": form,
        "files": files,
    })
}

fn md5(s: &str) -> String {
    hex::encode(Md5::digest(s.as_bytes()))
}

pub fn handle(req: Req) -> Resp {
    let seg = req.segments();
    let m = req.method.as_str();
    match (m, seg.as_slice()) {
        (_, []) => Resp::json(200, &json!({"name": "rsPostman echo server", "endpoints": ["/get", "/post", "/put", "/patch", "/delete", "/anything", "/status/:code", "/headers", "/basic-auth/:user/:pass", "/bearer", "/digest-auth/:user/:pass", "/cookies", "/cookies/set?k=v", "/cookies/delete?k", "/redirect/:n", "/redirect-to?url=", "/delay/:ms", "/html", "/xml", "/json", "/bytes/:n", "/uuid", "/llm (OpenAI-compatible chat stream)", "/llm/fail (empty stream)", "/response-headers?k=v", "ws://.../ws (WebSocket echo)"]})),
        ("GET", ["get"]) | ("POST", ["post"]) | ("PUT", ["put"]) | ("PATCH", ["patch"]) | ("DELETE", ["delete"]) | (_, ["anything", ..]) => {
            Resp::json(200, &echo(&req))
        }
        (_, ["get" | "post" | "put" | "patch" | "delete"]) => Resp::json(405, &json!({"error": "method not allowed"})),
        (_, ["status", code]) => {
            let code: u16 = code.parse().unwrap_or(400);
            if code == 204 || code == 304 {
                Resp::empty(code)
            } else {
                Resp::json(code, &json!({"status": code}))
            }
        }
        (_, ["headers"]) => Resp::json(200, &json!({"headers": echo(&req)["headers"]})),
        (_, ["basic-auth", user, pass]) => {
            let expect = format!("Basic {}", crate::http::basic_token(user, pass));
            if req.header("authorization") == Some(expect.as_str()) {
                Resp::json(200, &json!({"authenticated": true, "user": user}))
            } else {
                Resp::json(401, &json!({"authenticated": false})).with("WWW-Authenticate", "Basic realm=\"echo\"")
            }
        }
        (_, ["bearer"]) => match req.header("authorization").and_then(|a| a.strip_prefix("Bearer ")) {
            Some(t) if !t.is_empty() => Resp::json(200, &json!({"authenticated": true, "token": t})),
            _ => Resp::json(401, &json!({"authenticated": false})).with("WWW-Authenticate", "Bearer"),
        },
        (_, ["digest-auth", user, pass]) => {
            let nonce = "dcd98b7102dd2f0e8b11d0f600bfb0c093";
            let challenge = || {
                Resp::json(401, &json!({"authenticated": false}))
                    .with("WWW-Authenticate", &format!("Digest realm=\"echo\", qop=\"auth\", nonce=\"{nonce}\", opaque=\"abc\", algorithm=MD5"))
            };
            let Some(a) = req.header("authorization").filter(|a| a.starts_with("Digest ")) else { return challenge() };
            let p = crate::auth::challenge_params(a);
            let g = |k: &str| p.get(k).cloned().unwrap_or_default();
            let ha1 = md5(&format!("{user}:echo:{pass}"));
            let ha2 = md5(&format!("{m}:{}", g("uri")));
            let expect = md5(&format!("{ha1}:{nonce}:{}:{}:auth:{ha2}", g("nc"), g("cnonce")));
            if g("username") == *user && g("response") == expect {
                Resp::json(200, &json!({"authenticated": true, "user": user}))
            } else {
                challenge()
            }
        }
        (_, ["cookies"]) => Resp::json(200, &json!({"cookies": obj(&cookies(&req))})),
        (_, ["cookies", "set"]) => {
            let mut r = Resp::empty(302).with("Location", "/cookies");
            for (k, v) in &req.query {
                r = r.with("Set-Cookie", &format!("{k}={v}; Path=/"));
            }
            r
        }
        (_, ["cookies", "delete"]) => {
            let mut r = Resp::empty(302).with("Location", "/cookies");
            for (k, _) in &req.query {
                r = r.with("Set-Cookie", &format!("{k}=; Path=/; Max-Age=0"));
            }
            r
        }
        (_, ["redirect", n]) => {
            let n: u32 = n.parse().unwrap_or(1);
            let loc = if n <= 1 { "/get".to_string() } else { format!("/redirect/{}", n - 1) };
            Resp::empty(302).with("Location", &loc)
        }
        (_, ["redirect-to"]) => {
            let code = req.param("status_code").and_then(|c| c.parse().ok()).unwrap_or(302);
            Resp::empty(code).with("Location", req.param("url").unwrap_or("/get"))
        }
        (_, ["delay", ms]) => {
            let ms: u64 = ms.parse().unwrap_or(0).min(30_000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Resp::json(200, &json!({"delayed_ms": ms}))
        }
        // OpenAI-compatible chat completion stream (fixed ticker tape) for profiling practice.
        // `/llm/fail` answers with an empty stream (no events, no [DONE]).
        ("POST", ["llm"] | ["llm", "fail"]) => {
            let req_json: Value = serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
            let model = req_json["model"].as_str().unwrap_or("echo-llm");
            let tokens = if seg.len() == 2 {
                vec![]
            } else {
                vec!["Hello", ",", " this", " is", " a", " streamed", " test", " reply", " from", " the", " echo", " LLM", "."]
            };
            let chunks: Vec<String> = tokens
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let last = i == tokens.len() - 1;
                    let finish: Value = if last { json!("stop") } else { Value::Null };
                    let usage = if last {
                        json!({"prompt_tokens": 9, "completion_tokens": tokens.len(), "total_tokens": 9 + tokens.len()})
                    } else {
                        Value::Null
                    };
                    format!(
                        "data: {}\n\n",
                        json!({"id": "chatcmpl-echo", "object": "chat.completion.chunk", "created": 0, "model": model,
                               "choices": [{"index": 0, "delta": {"content": t}, "finish_reason": finish}], "usage": usage})
                    )
                })
                .collect();
            let mut body = chunks.join("");
            if !tokens.is_empty() {
                body.push_str("data: [DONE]\n\n");
            }
            Resp::new(200, "text/event-stream; charset=utf-8", body)
        }
        (_, ["html"]) => Resp::new(200, "text/html; charset=utf-8", "<!DOCTYPE html><html><head><title>Echo</title></head><body><h1>Hello from rsPostman</h1><p>HTML sample</p></body></html>"),
        (_, ["xml"]) => Resp::new(200, "application/xml", "<?xml version=\"1.0\" encoding=\"UTF-8\"?><slideshow title=\"Sample\"><slide type=\"all\"><title>Hello</title></slide></slideshow>"),
        (_, ["json"]) => Resp::json(200, &json!({"slideshow": {"title": "Sample", "slides": [{"title": "Hello", "type": "all"}, {"title": "World", "items": [1, 2, 3]}]}})),
        (_, ["bytes", n]) => {
            let n: usize = n.parse().unwrap_or(16).min(1 << 20);
            Resp::new(200, "application/octet-stream", (0..n).map(|i| (i * 31 % 256) as u8).collect::<Vec<u8>>())
        }
        (_, ["uuid"]) => Resp::json(200, &json!({"uuid": crate::model::new_id()})),
        (_, ["response-headers"]) => {
            let mut r = Resp::json(200, &obj(&req.query));
            for (k, v) in &req.query {
                r = r.with(k, v);
            }
            r
        }
        _ => Resp::json(404, &json!({"error": "not found", "path": req.path})),
    }
}

/// Start the echo server (`port` 0 = any free port).
pub fn start(host: &str, port: u16) -> Result<ServerHandle> {
    serve_ws(host, port, 8, handle, Some(std::sync::Arc::new(ws_echo)))
}

/// WebSocket endpoint (any path): greets, then echoes every text and binary message.
fn ws_echo(req: Req, mut socket: WsStream) {
    use tungstenite::Message;
    let _ = socket.send(Message::Text(json!({"event": "connected", "path": req.path}).to_string()));
    loop {
        match socket.read() {
            Ok(m @ (Message::Text(_) | Message::Binary(_))) => {
                if socket.send(m).is_err() {
                    break;
                }
            }
            // keep reading after a close frame so the closing handshake completes
            Err(_) => break,
            Ok(_) => {}
        }
    }
}
