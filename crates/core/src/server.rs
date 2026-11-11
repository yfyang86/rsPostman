//! Small dependency-free HTTP/1.1 server (keep-alive, chunked requests, WebSocket upgrade)
//! used by the echo server, mock servers and the GUI backend. Intended for local use.

use anyhow::{anyhow, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerHandle {
    pub fn url(&self) -> String {
        let h = if self.host == "0.0.0.0" { "127.0.0.1" } else { &self.host };
        format!("http://{h}:{}", self.port)
    }
    /// Stop accepting connections. Open keep-alive connections end on their own.
    pub fn stop(&self) {
        if !self.stop.swap(true, Ordering::SeqCst) {
            // wake the accept loop
            let h = if self.host == "0.0.0.0" { "127.0.0.1" } else { &self.host };
            let _ = TcpStream::connect((h, self.port));
        }
    }
    /// Block until the server is stopped.
    pub fn join(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub type WsStream = tungstenite::WebSocket<TcpStream>;
type WsHandler = Arc<dyn Fn(Req, WsStream) + Send + Sync>;
type Handler = Arc<dyn Fn(Req) -> Resp + Send + Sync>;

const MAX_HEADER_BYTES: usize = 256 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024 * 1024;
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        418 => "I'm a teapot",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

/// Read one CRLF-terminated line (without the terminator). `None` on a clean EOF.
fn read_line(r: &mut BufReader<TcpStream>, budget: &mut usize) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(*budget as u64 + 1).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if n > *budget || buf.last() != Some(&b'\n') {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "header too large or truncated"));
    }
    *budget -= n;
    while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn write_response(stream: &mut TcpStream, resp: &Resp, head_only: bool, keep_alive: bool) -> std::io::Result<()> {
    let mut out = Vec::with_capacity(resp.body.len() + 256);
    out.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status)).as_bytes());
    for (k, v) in &resp.headers {
        let lower = k.to_lowercase();
        if ["content-length", "transfer-encoding", "connection"].contains(&lower.as_str()) || k.contains(['\r', '\n', ':']) {
            continue;
        }
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(v.replace(['\r', '\n'], " ").as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    let no_body = resp.status < 200 || resp.status == 204 || resp.status == 304;
    if !no_body {
        out.extend_from_slice(format!("Content-Length: {}\r\n", resp.body.len()).as_bytes());
    }
    out.extend_from_slice(if keep_alive { b"Connection: keep-alive\r\n\r\n" } else { b"Connection: close\r\n\r\n" });
    if !no_body && !head_only {
        out.extend_from_slice(&resp.body);
    }
    stream.write_all(&out)?;
    stream.flush()
}

/// Serve one connection until it is closed, idle for too long, or upgraded.
fn connection(mut stream: TcpStream, handler: Handler, ws: Option<WsHandler>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(IDLE_TIMEOUT));
    let Ok(clone) = stream.try_clone() else { return };
    let mut reader = BufReader::new(clone);
    loop {
        let mut budget = MAX_HEADER_BYTES;
        // request line (tolerate one leading blank line)
        let line = match read_line(&mut reader, &mut budget) {
            Ok(Some(l)) if l.is_empty() => match read_line(&mut reader, &mut budget) {
                Ok(Some(l)) => l,
                _ => return,
            },
            Ok(Some(l)) => l,
            _ => return,
        };
        let mut parts = line.split_whitespace();
        let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next()) else {
            let _ = write_response(&mut stream, &Resp::text(400, "malformed request line"), false, false);
            return;
        };
        let mut headers: Vec<(String, String)> = vec![];
        loop {
            match read_line(&mut reader, &mut budget) {
                Ok(Some(l)) if l.is_empty() => break,
                Ok(Some(l)) => {
                    if let Some((k, v)) = l.split_once(':') {
                        headers.push((k.trim().to_string(), v.trim().to_string()));
                    }
                }
                _ => {
                    let _ = write_response(&mut stream, &Resp::text(431, "request headers too large"), false, false);
                    return;
                }
            }
        }
        let header = |name: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str());
        let connection_hdr = header("connection").unwrap_or("").to_lowercase();
        let keep_alive = if version == "HTTP/1.0" { connection_hdr.contains("keep-alive") } else { !connection_hdr.contains("close") };

        // origin-form or absolute-form target
        let target = match target.split_once("://") {
            Some((_, rest)) => rest.find('/').map(|i| rest[i..].to_string()).unwrap_or_else(|| "/".into()),
            None => target.to_string(),
        };
        let (path, raw_query) = match target.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (target.clone(), String::new()),
        };
        let mut req = Req {
            method: method.to_uppercase(),
            path: percent_encoding::percent_decode_str(&path).decode_utf8_lossy().to_string(),
            query: url::form_urlencoded::parse(raw_query.as_bytes()).map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            raw_query,
            headers: headers.clone(),
            body: vec![],
        };

        // WebSocket upgrade: hand the socket over to the handler on its own thread.
        let wants_upgrade = header("upgrade").map(|u| u.eq_ignore_ascii_case("websocket")).unwrap_or(false);
        if let (true, Some(wsh), Some(key)) = (wants_upgrade, ws.clone(), header("sec-websocket-key")) {
            let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
            let reply = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n");
            if stream.write_all(reply.as_bytes()).is_err() {
                return;
            }
            let _ = stream.set_read_timeout(None);
            let socket = tungstenite::WebSocket::from_raw_socket(stream, tungstenite::protocol::Role::Server, None);
            wsh(req, socket);
            return;
        }

        // body
        if header("expect").map(|e| e.eq_ignore_ascii_case("100-continue")).unwrap_or(false) {
            let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        let chunked = header("transfer-encoding").map(|t| t.to_lowercase().contains("chunked")).unwrap_or(false);
        let body_result: std::io::Result<()> = (|| {
            if chunked {
                loop {
                    let mut b = 1024;
                    let size_line = read_line(&mut reader, &mut b)?.unwrap_or_default();
                    let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16)
                        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad chunk size"))?;
                    if req.body.len() + size > MAX_BODY_BYTES {
                        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "body too large"));
                    }
                    if size == 0 {
                        // trailers
                        while let Some(l) = read_line(&mut reader, &mut b)? {
                            if l.is_empty() {
                                break;
                            }
                        }
                        return Ok(());
                    }
                    let start = req.body.len();
                    req.body.resize(start + size, 0);
                    reader.read_exact(&mut req.body[start..])?;
                    let mut crlf = [0u8; 2];
                    reader.read_exact(&mut crlf)?;
                }
            } else if let Some(len) = header("content-length") {
                let len: usize = len.trim().parse().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad content-length"))?;
                if len > MAX_BODY_BYTES {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "body too large"));
                }
                req.body.resize(len, 0);
                reader.read_exact(&mut req.body)?;
            }
            Ok(())
        })();
        if body_result.is_err() {
            let _ = write_response(&mut stream, &Resp::text(400, "invalid request body"), false, false);
            return;
        }

        let head_only = req.method == "HEAD";
        let h = handler.clone();
        let resp = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || h(req))).unwrap_or_else(|_| Resp::text(500, "internal error"));
        if write_response(&mut stream, &resp, head_only, keep_alive).is_err() || !keep_alive {
            return;
        }
    }
}

/// Start a server on `host:port` (`port` 0 picks a free port). One thread per connection.
pub fn serve<F>(host: &str, port: u16, workers: usize, handler: F) -> Result<ServerHandle>
where
    F: Fn(Req) -> Resp + Send + Sync + 'static,
{
    serve_ws(host, port, workers, handler, None)
}

/// Like `serve`, additionally accepting WebSocket upgrades.
/// `_workers` is kept for API compatibility; connections are served by one thread each.
pub fn serve_ws<F>(host: &str, port: u16, _workers: usize, handler: F, ws: Option<WsHandler>) -> Result<ServerHandle>
where
    F: Fn(Req) -> Resp + Send + Sync + 'static,
{
    let listener = TcpListener::bind((host, port)).map_err(|e| anyhow!("cannot listen on {host}:{port}: {e}"))?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let stop = Arc::new(AtomicBool::new(false));
    let handler: Handler = Arc::new(handler);
    let flag = stop.clone();
    let thread = std::thread::spawn(move || {
        for conn in listener.incoming() {
            if flag.load(Ordering::SeqCst) {
                break;
            }
            match conn {
                Ok(stream) => {
                    let (h, w) = (handler.clone(), ws.clone());
                    // a failed spawn (resource exhaustion) drops the connection instead of the server
                    let _ = std::thread::Builder::new().name("rsp-http".into()).spawn(move || connection(stream, h, w));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    Ok(ServerHandle { port, host: host.to_string(), stop, thread: Some(thread) })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(port: u16, request: &[u8]) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(request).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn keep_alive_chunked_head_and_errors() {
        let srv = serve("127.0.0.1", 0, 1, |req| {
            if req.path == "/panic" {
                panic!("boom");
            }
            Resp::text(200, &format!("{} {} q={:?} body={}", req.method, req.path, req.param("a"), req.text())).with("X-Echo", "1").with("Content-Length", "999")
        })
        .unwrap();
        // two pipelined requests on one connection, the second one closes it
        let out = raw(srv.port, b"POST /one?a=%C3%A9 HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\nabcGET /two HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert_eq!(out.matches("HTTP/1.1 200 OK").count(), 2, "{out}");
        assert!(out.contains("POST /one q=Some(\"é\") body=abc"), "{out}");
        assert!(out.contains("GET /two q=None body="));
        assert!(out.contains("Connection: keep-alive") && out.contains("Connection: close"));
        assert!(!out.contains("999"), "handler-supplied Content-Length must be replaced");
        // chunked request body with a trailer and 100-continue
        let out = raw(srv.port, b"PUT /c HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\nX-T: 1\r\n\r\n");
        assert!(out.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK"), "{out}");
        assert!(out.ends_with("body=Wikipedia"), "{out}");
        // HEAD has headers but no body; HTTP/1.0 closes by default
        let out = raw(srv.port, b"HEAD /h HTTP/1.0\r\n\r\n");
        assert!(out.contains("Content-Length: 20") && out.ends_with("\r\n\r\n"), "{out}");
        // a panicking handler answers 500 and the server keeps running
        assert!(raw(srv.port, b"GET /panic HTTP/1.1\r\nConnection: close\r\n\r\n").starts_with("HTTP/1.1 500"));
        assert!(raw(srv.port, b"garbage\r\n\r\n").starts_with("HTTP/1.1 400"));
        assert!(raw(srv.port, b"POST / HTTP/1.1\r\nContent-Length: nope\r\n\r\n").starts_with("HTTP/1.1 400"));
        assert!(raw(srv.port, b"GET /ok HTTP/1.1\r\nConnection: close\r\n\r\n").starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn burst_of_connections_is_served_and_stop_works() {
        let srv = serve("127.0.0.1", 0, 1, |req| Resp::text(200, &req.path)).unwrap();
        let port = srv.port;
        // open many connections first, then use them all (the pattern that starved tiny_http)
        let mut socks: Vec<TcpStream> = (0..40).map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap()).collect();
        for (i, s) in socks.iter_mut().enumerate() {
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            s.write_all(format!("GET /n{i} HTTP/1.1\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
        }
        for (i, s) in socks.iter_mut().enumerate() {
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            assert!(out.ends_with(&format!("/n{i}")), "{out}");
        }
        srv.stop();
        srv.join();
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }

    #[test]
    fn multipart_parsing() {
        let body = b"--XX\r\nContent-Disposition: form-data; name=\"a\"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name=\"f\"; filename=\"x.txt\"\r\nContent-Type: text/plain\r\n\r\nline1\r\nline2\r\n--XX--\r\n";
        let parts = parse_multipart("multipart/form-data; boundary=XX", body);
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].name.as_str(), parts[0].data.as_slice()), ("a", &b"1"[..]));
        assert_eq!((parts[1].filename.as_deref(), parts[1].content_type.as_deref(), parts[1].data.as_slice()), (Some("x.txt"), Some("text/plain"), &b"line1\r\nline2"[..]));
        assert!(parse_multipart("text/plain", body).is_empty());
    }
}
