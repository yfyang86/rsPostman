//! Minimal threaded HTTP server used by the echo server, mock servers and the GUI backend.

use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct Req {
    pub method: String,
    /// Path without query string (percent-decoded).
    pub path: String,
    pub raw_query: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| anyhow!("invalid JSON body: {e}"))
    }
    /// Path segments, e.g. `/a/b` -> `["a", "b"]`.
    pub fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
        Resp { status, headers: vec![("Content-Type".into(), content_type.into())], body: body.into() }
    }
    pub fn json<T: serde::Serialize>(status: u16, v: &T) -> Self {
        Resp::new(status, "application/json", serde_json::to_vec_pretty(v).unwrap_or_default())
    }
    pub fn text(status: u16, s: &str) -> Self {
        Resp::new(status, "text/plain; charset=utf-8", s.as_bytes().to_vec())
    }
    pub fn empty(status: u16) -> Self {
        Resp { status, headers: vec![], body: vec![] }
    }
    pub fn with(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
}

pub struct ServerHandle {
    pub port: u16,
    pub host: String,
    server: Arc<tiny_http::Server>,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl ServerHandle {
    pub fn url(&self) -> String {
        let h = if self.host == "0.0.0.0" { "127.0.0.1" } else { &self.host };
        format!("http://{h}:{}", self.port)
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        for _ in 0..self.threads.len().max(1) {
            self.server.unblock();
        }
    }
    /// Block until the server is stopped.
    pub fn join(mut self) {
        for t in std::mem::take(&mut self.threads) {
            let _ = t.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Start a server on `host:port` (`port` 0 picks a free port).
pub fn serve<F>(host: &str, port: u16, workers: usize, handler: F) -> Result<ServerHandle>
where
    F: Fn(Req) -> Resp + Send + Sync + 'static,
{
    let server = tiny_http::Server::http((host, port)).map_err(|e| anyhow!("cannot listen on {host}:{port}: {e}"))?;
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(port);
    let server = Arc::new(server);
    let stop = Arc::new(AtomicBool::new(false));
    let handler = Arc::new(handler);
    let mut threads = vec![];
    for _ in 0..workers.max(1) {
        let (server, stop, handler) = (server.clone(), stop.clone(), handler.clone());
        threads.push(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let mut rq = match server.recv() {
                    Ok(r) => r,
                    Err(_) => break,
                };
                let full = rq.url().to_string();
                let (path, raw_query) = match full.split_once('?') {
                    Some((p, q)) => (p.to_string(), q.to_string()),
                    None => (full.clone(), String::new()),
                };
                let mut body = vec![];
                let _ = rq.as_reader().read_to_end(&mut body);
                let req = Req {
                    method: rq.method().as_str().to_uppercase(),
                    path: percent_encoding::percent_decode_str(&path).decode_utf8_lossy().to_string(),
                    query: url::form_urlencoded::parse(raw_query.as_bytes()).map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                    raw_query,
                    headers: rq.headers().iter().map(|h| (h.field.as_str().as_str().to_string(), h.value.as_str().to_string())).collect(),
                    body,
                };
                let resp = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(req)))
                    .unwrap_or_else(|_| Resp::text(500, "internal error"));
                let mut out = tiny_http::Response::from_data(resp.body).with_status_code(resp.status);
                for (k, v) in &resp.headers {
                    if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                        out.add_header(h);
                    }
                }
                let _ = rq.respond(out);
            }
        }));
    }
    Ok(ServerHandle { port, host: host.to_string(), server, stop, threads })
}

#[derive(Debug, Clone, Default)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Parse a `multipart/form-data` body.
pub fn parse_multipart(content_type: &str, body: &[u8]) -> Vec<Part> {
    let Some(boundary) = content_type.split(';').filter_map(|p| p.trim().strip_prefix("boundary=")).next() else {
        return vec![];
    };
    let delim = format!("--{}", boundary.trim_matches('"')).into_bytes();
    let mut parts = vec![];
    let mut pos = match find(body, &delim, 0) {
        Some(p) => p + delim.len(),
        None => return parts,
    };
    loop {
        if body[pos..].starts_with(b"--") {
            break;
        }
        let start = pos + 2.min(body.len() - pos); // skip CRLF
        let Some(next) = find(body, &delim, start) else { break };
        let chunk = &body[start..next.saturating_sub(2).max(start)];
        if let Some(sep) = find(chunk, b"\r\n\r\n", 0) {
            let head = String::from_utf8_lossy(&chunk[..sep]).to_string();
            let mut part = Part { data: chunk[sep + 4..].to_vec(), ..Default::default() };
            for line in head.lines() {
                let lower = line.to_lowercase();
                if lower.starts_with("content-disposition:") {
                    for attr in line.split(';') {
                        let attr = attr.trim();
                        if let Some(v) = attr.strip_prefix("name=") {
                            part.name = v.trim_matches('"').to_string();
                        } else if let Some(v) = attr.strip_prefix("filename=") {
                            part.filename = Some(v.trim_matches('"').to_string());
                        }
                    }
                } else if lower.starts_with("content-type:") {
                    part.content_type = Some(line[13..].trim().to_string());
                }
            }
            parts.push(part);
        }
        pos = next + delim.len();
        if pos >= body.len() {
            break;
        }
    }
    parts
}
