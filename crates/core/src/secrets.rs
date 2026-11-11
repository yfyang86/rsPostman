//! Keeping credentials out of places they do not belong.
//!
//! Collections and environments hold whatever the user typed – that is their job. Everything
//! *derived* from a request (history, run reports, verbose output, benchmark labels) goes
//! through the functions in this module first, so literal passwords, tokens and keys are not
//! copied around the disk or the terminal.
//!
//! A value that is only a `{{variable}}` reference is never redacted: it is not a secret, and
//! keeping it lets a history entry be replayed.

use crate::engine::Exchange;
use crate::http::{BodySpec, Prepared, Response};
use crate::model::{Auth, Body, Request, KV};
use serde_json::Value;

pub const REDACTED: &str = "<redacted>";

/// Auth helper parameters that carry a secret.
pub const SECRET_AUTH_PARAMS: &[&str] =
    &["password", "token", "value", "accessToken", "refreshToken", "clientSecret", "consumerSecret", "tokenSecret", "secretKey", "sessionToken", "secret"];

/// Header / query / form / JSON key names whose value is a credential.
pub fn is_sensitive_name(name: &str) -> bool {
    let n = name.trim().to_lowercase().replace('-', "_");
    if ["authorization", "proxy_authorization", "cookie", "set_cookie", "x_api_key", "api_key", "apikey", "sig", "signature", "x_amz_signature", "x_amz_credential", "passwd", "pwd", "pass"].contains(&n.as_str()) {
        return true;
    }
    // `max_tokens`, `total_tokens`, … are counts, not credentials
    if n.ends_with("tokens") || n.ends_with("token_count") || n.ends_with("token_type") {
        return false;
    }
    ["password", "secret", "token", "api_key", "apikey", "access_key", "private_key", "credential", "session_id", "sessionid"].iter().any(|p| n.contains(p))
}

/// Query parameters additionally treat a bare `key` as a credential (`?key=API_KEY`).
pub fn is_sensitive_param(name: &str) -> bool {
    is_sensitive_name(name) || name.trim().eq_ignore_ascii_case("key")
}

/// True when `value` is empty or consists only of `{{variable}}` placeholders.
pub fn is_reference(value: &str) -> bool {
    let mut rest = value.trim();
    if rest.is_empty() {
        return true;
    }
    while let Some(after) = rest.strip_prefix("{{") {
        match after.find("}}") {
            Some(end) => rest = after[end + 2..].trim_start(),
            None => return false,
        }
    }
    rest.is_empty()
}

fn mask(value: &str) -> String {
    if is_reference(value) {
        value.to_string()
    } else {
        REDACTED.to_string()
    }
}

/// `Bearer abc` -> `Bearer <redacted>`; everything else is replaced completely.
fn mask_header(name: &str, value: &str) -> String {
    if is_reference(value) {
        return value.to_string();
    }
    if name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("proxy-authorization") {
        if let Some((scheme, rest)) = value.trim().split_once(' ') {
            if !rest.trim().is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return if is_reference(rest) { value.to_string() } else { format!("{scheme} {REDACTED}") };
            }
        }
    }
    REDACTED.to_string()
}

/// Redact the password of `user:pass@host` and the values of sensitive query parameters.
pub fn redact_url(url: &str) -> String {
    let (main, fragment) = match url.split_once('#') {
        Some((m, f)) => (m, Some(f)),
        None => (url, None),
    };
    let (base, query) = match main.split_once('?') {
        Some((b, q)) => (b.to_string(), Some(q)),
        None => (main.to_string(), None),
    };
    // userinfo
    let base = match base.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = match rest.find('/') {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, ""),
            };
            match authority.rsplit_once('@') {
                Some((userinfo, host)) => {
                    let user = userinfo.split(':').next().unwrap_or("");
                    let shown = if userinfo.contains(':') { format!("{user}:{REDACTED}") } else { user.to_string() };
                    format!("{scheme}://{shown}@{host}{path}")
                }
                None => base.clone(),
            }
        }
        None => base,
    };
    let mut out = base;
    if let Some(q) = query {
        let parts: Vec<String> = q
            .split('&')
            .map(|p| match p.split_once('=') {
                Some((k, v)) if is_sensitive_param(&percent_encoding::percent_decode_str(k).decode_utf8_lossy()) => format!("{k}={}", mask(v)),
                _ => p.to_string(),
            })
            .collect();
        out.push('?');
        out.push_str(&parts.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

fn redact_json_value(v: &mut Value) -> bool {
    let mut changed = false;
    match v {
        Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                if is_sensitive_name(k) {
                    match val {
                        Value::String(s) if !is_reference(s) => {
                            *val = Value::String(REDACTED.into());
                            changed = true;
                        }
                        Value::Number(_) => {
                            *val = Value::String(REDACTED.into());
                            changed = true;
                        }
                        Value::Object(_) | Value::Array(_) => changed |= redact_json_value(val),
                        _ => {}
                    }
                } else {
                    changed |= redact_json_value(val);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                changed |= redact_json_value(item);
            }
        }
        _ => {}
    }
    changed
}

/// Redact the values of sensitive keys in a JSON document; other text is returned unchanged.
pub fn redact_json_text(text: &str) -> String {
    let trimmed = text.trim_start();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return text.to_string();
    }
    match serde_json::from_str::<Value>(text) {
        Ok(mut v) => {
            if redact_json_value(&mut v) {
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.to_string())
            } else {
                text.to_string()
            }
        }
        Err(_) => text.to_string(),
    }
}

fn redact_rows(rows: &mut [KV]) {
    for r in rows.iter_mut().filter(|r| !r.is_file()) {
        if is_sensitive_name(&r.key) {
            r.value = mask(&r.value);
        }
    }
}

/// What was sent on the wire, without credentials.
pub fn redact_prepared(p: &Prepared) -> Prepared {
    let mut out = p.clone();
    out.url = redact_url(&p.url);
    for (k, v) in out.headers.iter_mut() {
        if is_sensitive_name(k) {
            *v = mask_header(k, v);
        }
    }
    out.body = match &p.body {
        BodySpec::Raw { text } => BodySpec::Raw { text: redact_json_text(text) },
        BodySpec::Urlencoded { fields } => {
            BodySpec::Urlencoded { fields: fields.iter().map(|(k, v)| (k.clone(), if is_sensitive_name(k) { mask(v) } else { v.clone() })).collect() }
        }
        BodySpec::Form { boundary, parts } => BodySpec::Form {
            boundary: boundary.clone(),
            parts: parts
                .iter()
                .map(|part| {
                    let mut part = part.clone();
                    if !part.file && is_sensitive_name(&part.name) {
                        part.value = mask(&part.value);
                    }
                    part
                })
                .collect(),
        },
        other => other.clone(),
    };
    if let Some((user, _)) = &p.digest {
        out.digest = Some((user.clone(), REDACTED.into()));
    }
    out
}

pub fn redact_auth(auth: &Auth) -> Auth {
    let mut out = auth.clone();
    for (k, v) in out.params.iter_mut() {
        if SECRET_AUTH_PARAMS.contains(&k.as_str()) {
            *v = mask(v);
        }
    }
    out
}

fn redact_body(b: &Body) -> Body {
    let mut out = b.clone();
    if let Some(raw) = &b.raw {
        out.raw = Some(redact_json_text(raw));
    }
    redact_rows(&mut out.urlencoded);
    redact_rows(&mut out.formdata);
    out
}

/// The editable request, without literal credentials (variable references are kept).
pub fn redact_request(r: &Request) -> Request {
    let mut out = r.clone();
    out.url.raw = redact_url(&r.url.raw);
    for q in out.url.query.iter_mut() {
        if is_sensitive_param(&q.key) {
            q.value = mask(&q.value);
        }
    }
    for h in out.header.iter_mut() {
        if is_sensitive_name(&h.key) {
            h.value = mask_header(&h.key, &h.value);
        }
    }
    out.auth = r.auth.as_ref().map(redact_auth);
    out.body = r.body.as_ref().map(redact_body);
    out
}

/// Redact credentials echoed back by the server (`Set-Cookie`, token fields of JSON bodies).
pub fn redact_response(r: &Response) -> Response {
    let mut out = r.clone();
    out.url = redact_url(&r.url);
    out.redirects = r.redirects.iter().map(|u| redact_url(u)).collect();
    for (k, v) in out.headers.iter_mut() {
        if is_sensitive_name(k) {
            *v = REDACTED.into();
        } else if k.eq_ignore_ascii_case("location") {
            *v = redact_url(v);
        }
    }
    for c in out.cookies.iter_mut() {
        c.value = REDACTED.into();
    }
    if r.is_text() {
        if let Ok(text) = std::str::from_utf8(&r.body) {
            let redacted = redact_json_text(text);
            if redacted != text {
                out.body = redacted.into_bytes();
            }
        }
    }
    out
}

pub fn redact_exchange(ex: &Exchange) -> Exchange {
    let mut out = ex.clone();
    out.request = ex.request.as_ref().map(redact_prepared);
    out.response = ex.response.as_ref().map(redact_response);
    out
}

/// Secrets shorter than this are not scrubbed by value (they would shred unrelated text).
pub const MIN_SCRUB_LEN: usize = 4;

/// Replace every occurrence of the given secret values in `text`.
pub fn scrub_text(text: &str, values: &[String]) -> String {
    let mut out = text.to_string();
    for v in values.iter().filter(|v| v.chars().count() >= MIN_SCRUB_LEN) {
        if out.contains(v.as_str()) {
            out = out.replace(v.as_str(), REDACTED);
        }
    }
    out
}

fn scrub_json(v: &mut Value, values: &[String]) {
    match v {
        Value::String(s) => *s = scrub_text(s, values),
        Value::Array(a) => a.iter_mut().for_each(|x| scrub_json(x, values)),
        Value::Object(o) => o.values_mut().for_each(|x| scrub_json(x, values)),
        _ => {}
    }
}

/// Remove secret values (vault contents) from a wire snapshot, wherever they appear.
pub fn scrub_prepared(p: &Prepared, values: &[String]) -> Prepared {
    let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
    scrub_json(&mut v, values);
    serde_json::from_value(v).unwrap_or_else(|_| redact_prepared(p))
}

/// Remove secret values from a response (URL, headers and text body).
pub fn scrub_response(r: &Response, values: &[String]) -> Response {
    let mut out = r.clone();
    out.url = scrub_text(&r.url, values);
    out.redirects = r.redirects.iter().map(|u| scrub_text(u, values)).collect();
    for (_, v) in out.headers.iter_mut() {
        *v = scrub_text(v, values);
    }
    for c in out.cookies.iter_mut() {
        c.value = scrub_text(&c.value, values);
    }
    if let Ok(text) = std::str::from_utf8(&r.body) {
        let scrubbed = scrub_text(text, values);
        if scrubbed != text {
            out.body = scrubbed.into_bytes();
        }
    }
    out
}

/// True when a request read back from the history had credentials removed.
pub fn was_redacted(r: &Request) -> bool {
    serde_json::to_string(r).map(|s| s.contains(REDACTED)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FormPart;

    #[test]
    fn sensitive_names() {
        for n in ["Authorization", "Cookie", "X-Api-Key", "api_key", "access_token", "client_secret", "Password", "X-Auth-Token", "refreshToken", "sessionId", "X-Amz-Security-Token"] {
            assert!(is_sensitive_name(n), "{n}");
        }
        for n in ["Accept", "Content-Type", "max_tokens", "total_tokens", "token_type", "page", "user", "keyboard", "monkey", "X-Request-Id"] {
            assert!(!is_sensitive_name(n), "{n}");
        }
    }

    #[test]
    fn references_are_not_secrets() {
        assert!(is_reference(""));
        assert!(is_reference("{{token}}"));
        assert!(is_reference(" {{a}}{{b}} "));
        assert!(!is_reference("Bearer {{token}}"));
        assert!(!is_reference("abc"));
        assert!(!is_reference("{{open"));
        assert_eq!(mask_header("Authorization", "Bearer {{token}}"), "Bearer {{token}}");
        assert_eq!(mask_header("Authorization", "Bearer abc.def"), "Bearer <redacted>");
        assert_eq!(mask_header("Authorization", "Basic dTpw"), "Basic <redacted>");
        assert_eq!(mask_header("Authorization", "raw-key"), "<redacted>");
        assert_eq!(mask_header("X-Api-Key", "{{key}}"), "{{key}}");
    }

    #[test]
    fn urls() {
        assert_eq!(redact_url("https://bob:pw@h.test/p?a=1&api_key=XYZ&token={{t}}#frag"), "https://bob:<redacted>@h.test/p?a=1&api_key=<redacted>&token={{t}}#frag");
        assert_eq!(redact_url("https://bob@h.test/p"), "https://bob@h.test/p");
        assert_eq!(redact_url("{{base}}/x?access_token=abc&max_tokens=5"), "{{base}}/x?access_token=<redacted>&max_tokens=5");
        assert_eq!(redact_url("http://h/plain"), "http://h/plain");
        assert_eq!(redact_url("https://maps.test/api?key=AIza&q=x"), "https://maps.test/api?key=<redacted>&q=x");
        assert!(!is_sensitive_name("key"), "a JSON field called `key` is too generic to redact");
    }

    #[test]
    fn prepared_and_request() {
        let p = Prepared {
            method: "POST".into(),
            url: "http://h/login?apikey=K".into(),
            headers: vec![("Authorization".into(), "Bearer T0P".into()), ("Accept".into(), "*/*".into()), ("Cookie".into(), "sid=1".into())],
            body: BodySpec::Raw { text: r#"{"user":"bob","password":"hunter2","nested":{"client_secret":"s","n":1},"max_tokens":9}"#.into() },
            digest: Some(("bob".into(), "pw".into())),
        };
        let r = redact_prepared(&p);
        let dump = serde_json::to_string(&r).unwrap();
        for leaked in ["T0P", "hunter2", "sid=1", "\"pw\"", "apikey=K", "\"s\""] {
            assert!(!dump.contains(leaked), "{leaked} leaked in {dump}");
        }
        assert!(dump.contains("bob") && dump.contains("Bearer <redacted>") && dump.contains("max_tokens"));
        assert_eq!(r.header("accept"), Some("*/*"));

        let form = Prepared {
            body: BodySpec::Form { boundary: "b".into(), parts: vec![FormPart { name: "password".into(), value: "x1".into(), file: false, content_type: None }, FormPart { name: "token".into(), value: "/tmp/f".into(), file: true, content_type: None }] },
            ..p.clone()
        };
        let BodySpec::Form { parts, .. } = redact_prepared(&form).body else { panic!() };
        assert_eq!((parts[0].value.as_str(), parts[1].value.as_str()), (REDACTED, "/tmp/f"));
        let enc = Prepared { body: BodySpec::Urlencoded { fields: vec![("client_secret".into(), "cs".into()), ("grant_type".into(), "password".into())] }, ..p };
        assert_eq!(redact_prepared(&enc).body.preview(), "client_secret=%3Credacted%3E&grant_type=password");

        let mut req = Request::new("GET", "http://h/x?token=abc");
        req.header.push(KV::new("X-Api-Key", "{{key}}"));
        req.header.push(KV::new("Authorization", "Basic abc"));
        req.auth = Some(Auth::new("basic", &[("username", "bob"), ("password", "pw")]));
        assert!(!was_redacted(&req));
        let red = redact_request(&req);
        assert!(was_redacted(&red));
        assert_eq!(red.url.raw, "http://h/x?token=<redacted>");
        assert_eq!(red.url.query[0].value, REDACTED);
        assert_eq!(red.header[0].value, "{{key}}");
        assert_eq!(red.header[1].value, "Basic <redacted>");
        let a = red.auth.unwrap();
        assert_eq!((a.get("username"), a.get("password")), ("bob", REDACTED));
        // references survive, so such an entry stays replayable
        let mut by_ref = Request::new("GET", "{{base}}/x");
        by_ref.auth = Some(Auth::new("bearer", &[("token", "{{token}}")]));
        assert!(!was_redacted(&redact_request(&by_ref)));
    }

    #[test]
    fn scrubbing_by_value() {
        let values = vec!["S3CRET-VALUE".to_string(), "abc".to_string()];
        assert_eq!(scrub_text("x S3CRET-VALUE y abc", &values), "x <redacted> y abc", "short values are left alone");
        let p = Prepared {
            method: "GET".into(),
            url: "http://h/path/S3CRET-VALUE/x".into(),
            headers: vec![("X-Odd-Name".into(), "pre-S3CRET-VALUE".into())],
            body: BodySpec::Raw { text: "free text S3CRET-VALUE".into() },
            digest: Some(("u".into(), "S3CRET-VALUE".into())),
        };
        let out = scrub_prepared(&p, &values);
        assert!(!serde_json::to_string(&out).unwrap().contains("S3CRET-VALUE"));
        assert_eq!(out.url, "http://h/path/<redacted>/x");
        let r = Response { url: "http://h/S3CRET-VALUE".into(), headers: vec![("X-Echo".into(), "S3CRET-VALUE".into())], body: b"{\"echo\": \"S3CRET-VALUE\"}".to_vec(), ..Default::default() };
        let out = scrub_response(&r, &values);
        assert!(!serde_json::to_string(&out).unwrap().contains("S3CRET") && !out.text().contains("S3CRET"));
    }

    #[test]
    fn responses() {
        let r = Response {
            status: 200,
            url: "http://h/cb?access_token=U1".into(),
            redirects: vec!["http://h/cb?access_token=U1".into()],
            headers: vec![("Content-Type".into(), "application/json".into()), ("Set-Cookie".into(), "sid=abc; Path=/".into()), ("Location".into(), "/cb?access_token=U1".into())],
            body: br#"{"access_token":"AT","expires_in":3600,"user":{"name":"n","refresh_token":"RT"}}"#.to_vec(),
            ..Default::default()
        };
        let out = redact_response(&r);
        let text = out.text();
        assert!(!text.contains("AT\"") && !text.contains("RT") && text.contains("3600") && text.contains("\"n\""));
        assert_eq!(out.header("set-cookie"), Some(REDACTED));
        assert!(!serde_json::to_string(&out).unwrap().contains("U1"));
        let plain = Response { body: b"token=abc not json".to_vec(), ..Default::default() };
        assert_eq!(redact_response(&plain).body, plain.body);
    }
}
