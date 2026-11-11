//! Turning a model `Request` into a concrete HTTP exchange.

use crate::cookies::{Cookie, CookieJar};
use crate::model::{Auth, Request, Url};
use crate::vars::Scopes;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const USER_AGENT: &str = concat!("rsPostman/", env!("CARGO_PKG_VERSION"));

const QUERY_SET: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'#').add(b'<').add(b'>').add(b'&').add(b'`');
const KEY_SET: &AsciiSet = &QUERY_SET.add(b'=');
const FORM_SET: &AsciiSet = &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'*');

pub fn form_encode(s: &str) -> String {
    utf8_percent_encode(s, FORM_SET).to_string().replace("%20", "+")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Request timeout in milliseconds; 0 disables the timeout.
    pub timeout_ms: u64,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub verify_ssl: bool,
    /// Proxy URL (`http://`, `https://`, `socks5://`); empty = system proxy settings.
    pub proxy: String,
    /// Ignore `HTTP(S)_PROXY` environment variables.
    pub no_system_proxy: bool,
    /// Extra CA certificate bundle (PEM).
    pub ca_cert: String,
    /// Client certificate + private key (PEM, both in one file).
    pub client_cert: String,
    pub send_default_headers: bool,
    pub user_agent: String,
    /// Persist cookies between requests.
    pub cookies: bool,
    /// Maximum number of history entries kept.
    pub history_limit: usize,
    /// Maximum response body bytes stored per history entry.
    pub history_body_limit: usize,
    /// Keep literal passwords, tokens and keys in the history. Off by default: the history
    /// then stores `<redacted>` instead (variable references are always kept).
    pub history_store_secrets: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            timeout_ms: 0,
            follow_redirects: true,
            max_redirects: 10,
            verify_ssl: true,
            proxy: String::new(),
            no_system_proxy: false,
            ca_cert: String::new(),
            client_cert: String::new(),
            send_default_headers: true,
            user_agent: USER_AGENT.into(),
            cookies: true,
            history_limit: 500,
            history_body_limit: 512 * 1024,
            history_store_secrets: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormPart {
    pub name: String,
    /// Text value, or file path when `file` is set.
    pub value: String,
    #[serde(default)]
    pub file: bool,
    #[serde(default)]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BodySpec {
    #[default]
    None,
    Raw {
        text: String,
    },
    Urlencoded {
        fields: Vec<(String, String)>,
    },
    Form {
        boundary: String,
        parts: Vec<FormPart>,
    },
    File {
        path: String,
    },
}

fn guess_mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "json" => "application/json",
        "txt" | "log" | "md" => "text/plain",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "js" => "application/javascript",
        "css" => "text/css",
        _ => "application/octet-stream",
    }
}

impl BodySpec {
    pub fn bytes(&self) -> Result<Option<Vec<u8>>> {
        Ok(match self {
            BodySpec::None => None,
            BodySpec::Raw { text } => Some(text.clone().into_bytes()),
            BodySpec::Urlencoded { fields } => Some(
                fields
                    .iter()
                    .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
                    .collect::<Vec<_>>()
                    .join("&")
                    .into_bytes(),
            ),
            BodySpec::File { path } => Some(std::fs::read(path).with_context(|| format!("cannot read body file {path}"))?),
            BodySpec::Form { boundary, parts } => {
                let mut out = Vec::new();
                for p in parts {
                    out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                    let name = p.name.replace('"', "%22");
                    if p.file {
                        let data = std::fs::read(&p.value).with_context(|| format!("cannot read form file {}", p.value))?;
                        let fname = std::path::Path::new(&p.value).file_name().and_then(|f| f.to_str()).unwrap_or("file");
                        let ct = p.content_type.clone().unwrap_or_else(|| guess_mime(&p.value).to_string());
                        out.extend_from_slice(
                            format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{fname}\"\r\nContent-Type: {ct}\r\n\r\n")
                                .as_bytes(),
                        );
                        out.extend_from_slice(&data);
                    } else {
                        out.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes());
                        if let Some(ct) = &p.content_type {
                            out.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
                        }
                        out.extend_from_slice(b"\r\n");
                        out.extend_from_slice(p.value.as_bytes());
                    }
                    out.extend_from_slice(b"\r\n");
                }
                out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
                Some(out)
            }
        })
    }

    /// Content type implied by the body when the user did not set one.
    pub fn content_type(&self, language: &str) -> Option<String> {
        match self {
            BodySpec::None => None,
            BodySpec::Raw { .. } => Some(
                match language {
                    "json" | "graphql" => "application/json",
                    "xml" => "application/xml",
                    "html" => "text/html",
                    "javascript" => "application/javascript",
                    _ => "text/plain",
                }
                .to_string(),
            ),
            BodySpec::Urlencoded { .. } => Some("application/x-www-form-urlencoded".into()),
            BodySpec::Form { boundary, .. } => Some(format!("multipart/form-data; boundary={boundary}")),
            BodySpec::File { path } => Some(guess_mime(path).to_string()),
        }
    }

    /// Short human readable rendering (history, console, verbose output).
    pub fn preview(&self) -> String {
        match self {
            BodySpec::None => String::new(),
            BodySpec::Raw { text } => text.clone(),
            BodySpec::Urlencoded { .. } => String::from_utf8(self.bytes().ok().flatten().unwrap_or_default()).unwrap_or_default(),
            BodySpec::Form { parts, .. } => parts
                .iter()
                .map(|p| if p.file { format!("{}=@{}", p.name, p.value) } else { format!("{}={}", p.name, p.value) })
                .collect::<Vec<_>>()
                .join("\n"),
            BodySpec::File { path } => format!("@{path}"),
        }
    }
}

/// A fully resolved request, ready to be sent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Prepared {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: BodySpec,
    /// Digest credentials (`user`, `password`) answered on a 401 challenge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<(String, String)>,
}

impl Prepared {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        self.headers.push((name.to_string(), value.to_string()));
    }
    pub fn add_query(&mut self, key: &str, value: &str) {
        let sep = if self.url.contains('?') { '&' } else { '?' };
        self.url.push(sep);
        self.url.push_str(&format!("{}={}", utf8_percent_encode(key, KEY_SET), utf8_percent_encode(value, QUERY_SET)));
    }
}

mod b64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD.decode(s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub http_version: String,
    pub headers: Vec<(String, String)>,
    /// Body bytes (base64 in JSON).
    #[serde(with = "b64")]
    pub body: Vec<u8>,
    pub time_ms: u64,
    /// Time until the response headers of the final hop arrived (time to first byte), in ms.
    #[serde(default)]
    pub ttfb_ms: u64,
    /// Body size in bytes (after decompression).
    pub size: usize,
    /// Final URL after redirects.
    pub url: String,
    #[serde(default)]
    pub redirects: Vec<String>,
    /// Cookies set by this exchange.
    #[serde(default)]
    pub cookies: Vec<Cookie>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
    pub fn content_type(&self) -> String {
        self.header("content-type").unwrap_or("").split(';').next().unwrap_or("").trim().to_lowercase()
    }
    pub fn is_text(&self) -> bool {
        let ct = self.content_type();
        ct.is_empty() && std::str::from_utf8(&self.body).is_ok()
            || ct.starts_with("text/")
            || ["json", "xml", "javascript", "html", "x-www-form-urlencoded", "yaml", "graphql"].iter().any(|k| ct.contains(k))
    }
    pub fn headers_size(&self) -> usize {
        self.headers.iter().map(|(k, v)| k.len() + v.len() + 4).sum()
    }
}

/// Resolve the final URL: variables, `:path` variables, enabled query rows.
pub fn build_url(url: &Url, sc: &Scopes) -> String {
    let base = url.base();
    let (scheme, rest) = match base.split_once("://") {
        Some((s, r)) => (format!("{s}://"), r.to_string()),
        None => (String::new(), base.clone()),
    };
    let rest = rest
        .split('/')
        .enumerate()
        .map(|(i, seg)| {
            if i > 0 && seg.starts_with(':') && seg.len() > 1 {
                if let Some(v) = url.variable.iter().find(|k| k.key == seg[1..]) {
                    return utf8_percent_encode(&sc.resolve(&v.value), KEY_SET).to_string().replace('/', "%2F");
                }
            }
            seg.to_string()
        })
        .collect::<Vec<_>>()
        .join("/");
    let mut s = sc.resolve(&format!("{scheme}{rest}")).trim().to_string();
    if !s.is_empty() && !s.contains("://") {
        s = format!("http://{s}");
    }
    let q: Vec<String> = url
        .query
        .iter()
        .filter(|k| !k.disabled && !(k.key.is_empty() && k.value.is_empty()))
        .map(|k| {
            let key = utf8_percent_encode(&sc.resolve(&k.key), KEY_SET).to_string();
            let val = utf8_percent_encode(&sc.resolve(&k.value), QUERY_SET).to_string();
            format!("{key}={val}")
        })
        .collect();
    if !q.is_empty() {
        s.push(if s.contains('?') { '&' } else { '?' });
        s.push_str(&q.join("&"));
    }
    s
}

fn random_boundary() -> String {
    use rand::Rng;
    let mut r = rand::thread_rng();
    format!("--------------------------{}", (0..24).map(|_| r.gen_range(0..10).to_string()).collect::<String>())
}

/// Build the wire-level request from the model, applying variables and auth.
pub fn prepare(req: &Request, auth: Option<&Auth>, sc: &Scopes) -> Result<Prepared> {
    let mut p = Prepared {
        method: sc.resolve(&req.method).trim().to_uppercase(),
        url: build_url(&req.url, sc),
        ..Default::default()
    };
    if p.method.is_empty() {
        p.method = "GET".into();
    }
    if p.url.is_empty() {
        return Err(anyhow!("request URL is empty"));
    }
    for h in req.header.iter().filter(|h| !h.disabled && !h.key.trim().is_empty()) {
        p.headers.push((sc.resolve(h.key.trim()), sc.resolve(&h.value)));
    }
    let mut language = "text";
    if let Some(b) = req.body.as_ref().filter(|b| !b.disabled) {
        language = b.language();
        p.body = match b.mode.as_str() {
            "raw" => match &b.raw {
                Some(t) if !t.is_empty() => BodySpec::Raw { text: sc.resolve(t) },
                _ => BodySpec::None,
            },
            "urlencoded" => BodySpec::Urlencoded {
                fields: b
                    .urlencoded
                    .iter()
                    .filter(|k| !k.disabled && !k.key.is_empty())
                    .map(|k| (sc.resolve(&k.key), sc.resolve(&k.value)))
                    .collect(),
            },
            "formdata" => {
                let mut parts = vec![];
                for k in b.formdata.iter().filter(|k| !k.disabled && !k.key.is_empty()) {
                    let content_type: Option<String> = None;
                    if k.is_file() {
                        for path in k.file_paths() {
                            parts.push(FormPart { name: sc.resolve(&k.key), value: sc.resolve(&path), file: true, content_type: content_type.clone() });
                        }
                    } else {
                        parts.push(FormPart { name: sc.resolve(&k.key), value: sc.resolve(&k.value), file: false, content_type });
                    }
                }
                BodySpec::Form { boundary: random_boundary(), parts }
            }
            "file" | "binary" => match b.file_path() {
                Some(path) => BodySpec::File { path: sc.resolve(&path) },
                None => BodySpec::None,
            },
            "graphql" => {
                language = "graphql";
                let g = b.graphql.clone().unwrap_or_default();
                let vars = sc.resolve(&g.variables);
                let vars: serde_json::Value =
                    if vars.trim().is_empty() { serde_json::json!({}) } else { serde_json::from_str(&vars).context("GraphQL variables are not valid JSON")? };
                BodySpec::Raw { text: serde_json::json!({"query": sc.resolve(&g.query), "variables": vars}).to_string() }
            }
            _ => BodySpec::None,
        };
    }
    if p.header("content-type").is_none() {
        if let Some(ct) = p.body.content_type(language) {
            p.headers.push(("Content-Type".into(), ct));
        }
    }
    if let Some(a) = auth {
        crate::auth::apply(a, &mut p, sc)?;
    }
    // A vault reference that could not be resolved must not be sent to a server as text.
    let missing = crate::vault::references(&serde_json::to_string(&p).unwrap_or_default());
    if !missing.is_empty() {
        return Err(anyhow!(
            "vault secret `{}` is not available: the vault is locked or has no secret with that name",
            missing.join("`, `")
        ));
    }
    Ok(p)
}

/// Build an HTTP client for `st`. Reuse it (see [`execute_with`]) to keep connections alive
/// across requests.
pub fn build_client(st: &Settings) -> Result<reqwest::blocking::Client> {
    build_client_with(st, true)
}

/// Like [`build_client`]; with `keep_alive` off every request opens a new connection.
pub fn build_client_with(st: &Settings, keep_alive: bool) -> Result<reqwest::blocking::Client> {
    let mut b = reqwest::blocking::Client::builder()
        .pool_max_idle_per_host(if keep_alive { usize::MAX } else { 0 })
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(!st.verify_ssl)
        .timeout(if st.timeout_ms > 0 { Some(Duration::from_millis(st.timeout_ms)) } else { None })
        .connect_timeout(Duration::from_secs(30));
    if !st.proxy.trim().is_empty() {
        b = b.proxy(reqwest::Proxy::all(st.proxy.trim()).context("invalid proxy URL")?);
    } else if st.no_system_proxy {
        b = b.no_proxy();
    }
    if !st.ca_cert.trim().is_empty() {
        let pem = std::fs::read(st.ca_cert.trim()).with_context(|| format!("cannot read CA certificate {}", st.ca_cert))?;
        for cert in reqwest::Certificate::from_pem_bundle(&pem)? {
            b = b.add_root_certificate(cert);
        }
    }
    if !st.client_cert.trim().is_empty() {
        let pem = std::fs::read(st.client_cert.trim()).with_context(|| format!("cannot read client certificate {}", st.client_cert))?;
        b = b.identity(reqwest::Identity::from_pem(&pem)?);
    }
    Ok(b.build()?)
}

pub(crate) fn explain(e: reqwest::Error) -> anyhow::Error {
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        let t = s.to_string();
        if !msg.contains(&t) {
            msg.push_str(": ");
            msg.push_str(&t);
        }
        src = s.source();
    }
    anyhow!(msg)
}

/// Send the request. Redirects, cookies and digest challenges are handled here.
pub fn execute(p: &Prepared, st: &Settings, jar: Option<&mut CookieJar>) -> Result<Response> {
    execute_with(&build_client(st)?, p, st, jar)
}

/// Like [`execute`] with a caller-provided client, so repeated requests share its connection pool.
pub fn execute_with(client: &reqwest::blocking::Client, p: &Prepared, st: &Settings, mut jar: Option<&mut CookieJar>) -> Result<Response> {
    let mut method = p.method.clone();
    let mut url = url::Url::parse(&p.url).map_err(|e| anyhow!("invalid URL `{}`: {e}", p.url))?;
    let mut headers = p.headers.clone();
    let mut body = p.body.bytes()?;
    let mut redirects = vec![];
    let mut cookies_set = vec![];
    let mut digest_done = false;
    let has = |h: &[(String, String)], n: &str| h.iter().any(|(k, _)| k.eq_ignore_ascii_case(n));
    let started = Instant::now();
    loop {
        let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| anyhow!("invalid HTTP method `{method}`"))?;
        let mut rb = client.request(m, url.clone());
        for (k, v) in &headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes()).map_err(|_| anyhow!("invalid header name `{k}`"))?;
            let value = reqwest::header::HeaderValue::from_str(v).map_err(|_| anyhow!("invalid value for header `{k}`"))?;
            rb = rb.header(name, value);
        }
        if st.send_default_headers {
            if !has(&headers, "user-agent") {
                rb = rb.header("User-Agent", if st.user_agent.is_empty() { USER_AGENT } else { &st.user_agent });
            }
            if !has(&headers, "accept") {
                rb = rb.header("Accept", "*/*");
            }
        }
        if !has(&headers, "cookie") {
            if let Some(c) = jar.as_ref().and_then(|j| j.header_for(&url)) {
                rb = rb.header("Cookie", c);
            }
        }
        if let Some(b) = &body {
            rb = rb.body(b.clone());
        }
        let resp = rb.send().map_err(explain)?;
        let headers_at = started.elapsed().as_millis() as u64;
        let status = resp.status();
        let resp_headers: Vec<(String, String)> =
            resp.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
        for (k, v) in &resp_headers {
            if k.eq_ignore_ascii_case("set-cookie") {
                if let Some(c) = Cookie::parse(v, &url) {
                    if let Some(j) = jar.as_mut() {
                        j.store(c.clone());
                    }
                    cookies_set.push(c);
                }
            }
        }
        let find = |n: &str| resp_headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());

        if status.as_u16() == 401 && !digest_done {
            if let (Some((user, pass)), Some(ch)) = (&p.digest, find("www-authenticate")) {
                if ch.to_lowercase().starts_with("digest") {
                    digest_done = true;
                    let uri = match url.query() {
                        Some(q) => format!("{}?{}", url.path(), q),
                        None => url.path().to_string(),
                    };
                    let value = crate::auth::digest_header(&ch, user, pass, &method, &uri, body.as_deref().unwrap_or(b""));
                    headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
                    headers.push(("Authorization".into(), value));
                    continue;
                }
            }
        }

        if status.is_redirection() && st.follow_redirects && (redirects.len() as u32) < st.max_redirects {
            if let Some(loc) = find("location") {
                let next = url.join(&loc).map_err(|e| anyhow!("invalid redirect location `{loc}`: {e}"))?;
                redirects.push(next.to_string());
                let code = status.as_u16();
                if code == 303 || ((code == 301 || code == 302) && method != "GET" && method != "HEAD") {
                    method = "GET".into();
                    body = None;
                    headers.retain(|(k, _)| !k.to_lowercase().starts_with("content-"));
                }
                if next.host_str() != url.host_str() {
                    headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
                }
                url = next;
                continue;
            }
        }

        let version = format!("{:?}", resp.version());
        let final_url = resp.url().to_string();
        let bytes = resp.bytes().map_err(explain)?.to_vec();
        return Ok(Response {
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or("").to_string(),
            http_version: version,
            headers: resp_headers,
            size: bytes.len(),
            body: bytes,
            time_ms: started.elapsed().as_millis() as u64,
            ttfb_ms: headers_at,
            url: final_url,
            redirects,
            cookies: cookies_set,
        });
    }
}

pub fn basic_token(user: &str, pass: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
}

pub fn human_size(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.2} KB", n as f64 / 1024.0)
    } else {
        format!("{:.2} MB", n as f64 / 1048576.0)
    }
}

pub fn human_time(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        format!("{:.2} s", ms as f64 / 1000.0)
    }
}
