//! Import of `curl` command lines.

use crate::model::{Auth, Body, Request, Url, KV};
use anyhow::{bail, Result};

/// Split a command line like a POSIX shell (quotes, backslashes, `$'..'`, line continuations).
pub fn tokenize(input: &str) -> Result<Vec<String>> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut has = false;
    let mut it = input.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('\n') => {}
                Some('\r') => {
                    if it.peek() == Some(&'\n') {
                        it.next();
                    }
                }
                Some(n) => {
                    cur.push(n);
                    has = true;
                }
                None => {}
            },
            '^' if matches!(it.peek(), Some('\n') | Some('\r')) => {
                // Windows cmd line continuation
                while matches!(it.peek(), Some('\n') | Some('\r')) {
                    it.next();
                }
            }
            '\'' => {
                has = true;
                loop {
                    match it.next() {
                        Some('\'') => break,
                        Some(x) => cur.push(x),
                        None => bail!("unterminated single quote"),
                    }
                }
            }
            '"' => {
                has = true;
                loop {
                    match it.next() {
                        Some('"') => break,
                        Some('\\') => match it.next() {
                            Some(x @ ('"' | '\\' | '$' | '`')) => cur.push(x),
                            Some('\n') => {}
                            Some(x) => {
                                cur.push('\\');
                                cur.push(x);
                            }
                            None => bail!("unterminated double quote"),
                        },
                        Some(x) => cur.push(x),
                        None => bail!("unterminated double quote"),
                    }
                }
            }
            '$' if it.peek() == Some(&'\'') => {
                it.next();
                has = true;
                loop {
                    match it.next() {
                        Some('\'') => break,
                        Some('\\') => match it.next() {
                            Some('n') => cur.push('\n'),
                            Some('r') => cur.push('\r'),
                            Some('t') => cur.push('\t'),
                            Some(x) => cur.push(x),
                            None => bail!("unterminated $'..' quote"),
                        },
                        Some(x) => cur.push(x),
                        None => bail!("unterminated $'..' quote"),
                    }
                }
            }
            c if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    Ok(out)
}

/// Options that take a value we do not care about.
const IGNORED_WITH_VALUE: &[&str] = &[
    "-o", "--output", "-w", "--write-out", "-m", "--max-time", "--connect-timeout", "--retry", "--retry-delay", "-x", "--proxy",
    "--cacert", "--cert", "--key", "-E", "-c", "--cookie-jar", "--max-redirs", "-T", "--upload-file", "--resolve", "-K", "--config",
    "--limit-rate", "--ciphers", "--proxy-user", "-U", "--interface", "--dns-servers", "--unix-socket", "--tlsv1.2", "--http-version",
    "-D", "--dump-header", "--trace", "--trace-ascii", "--stderr", "-z", "--time-cond", "--range", "-r", "--cert-type", "--key-type", "--pass",
];

pub fn parse(command: &str) -> Result<Request> {
    let tokens = tokenize(command.trim())?;
    let mut it = tokens.into_iter().peekable();
    match it.peek().map(|s| s.as_str()) {
        Some("curl") | Some("curl.exe") => {
            it.next();
        }
        _ => bail!("not a curl command (must start with `curl`)"),
    }
    let mut method: Option<String> = None;
    let mut url: Option<String> = None;
    let mut headers: Vec<KV> = vec![];
    let mut data: Vec<String> = vec![];
    let mut urlencode: Vec<String> = vec![];
    let mut form: Vec<KV> = vec![];
    let mut json = false;
    let mut get = false;
    let mut head = false;
    let mut user: Option<String> = None;
    let mut binary_file: Option<String> = None;

    while let Some(tok) = it.next() {
        // --opt=value
        let (opt, inline) = match tok.split_once('=') {
            Some((o, v)) if o.starts_with("--") => (o.to_string(), Some(v.to_string())),
            _ => (tok.clone(), None),
        };
        // -XPOST / -HName: v style
        let (opt, inline) = if inline.is_none() && opt.len() > 2 && opt.starts_with('-') && !opt.starts_with("--") {
            let flag = &opt[..2];
            if ["-X", "-H", "-d", "-F", "-u", "-A", "-e", "-b", "-o", "-m", "-x"].contains(&flag) {
                (flag.to_string(), Some(opt[2..].to_string()))
            } else {
                (opt, None)
            }
        } else {
            (opt, inline)
        };
        let mut val = |name: &str| -> Result<String> {
            match inline.clone().or_else(|| it.next()) {
                Some(v) => Ok(v),
                None => bail!("curl option {name} needs a value"),
            }
        };
        match opt.as_str() {
            "-X" | "--request" => method = Some(val("-X")?.to_uppercase()),
            "-H" | "--header" => {
                let h = val("-H")?;
                match h.split_once(':') {
                    Some((k, v)) => headers.push(KV::new(k.trim(), v.trim())),
                    None => headers.push(KV::new(h.trim_end_matches(';').trim(), "")),
                }
            }
            "-d" | "--data" | "--data-ascii" | "--data-raw" => data.push(val("-d")?),
            "--data-binary" => {
                let v = val("--data-binary")?;
                match v.strip_prefix('@') {
                    Some(path) if data.is_empty() => binary_file = Some(path.to_string()),
                    _ => data.push(v),
                }
            }
            "--data-urlencode" => urlencode.push(val("--data-urlencode")?),
            "--json" => {
                json = true;
                data.push(val("--json")?);
            }
            "-F" | "--form" | "--form-string" => {
                let f = val("-F")?;
                let (k, v) = f.split_once('=').unwrap_or((&f, ""));
                let v = v.split(";type=").next().unwrap_or(v);
                if let Some(path) = v.strip_prefix('@').filter(|_| opt != "--form-string") {
                    let mut kv = KV::new(k, "");
                    kv.kind = Some("file".into());
                    kv.src = Some(serde_json::json!(path.trim_matches('"')));
                    form.push(kv);
                } else {
                    let mut kv = KV::new(k, v.trim_matches('"'));
                    kv.kind = Some("text".into());
                    form.push(kv);
                }
            }
            "-u" | "--user" => user = Some(val("-u")?),
            "-A" | "--user-agent" => headers.push(KV::new("User-Agent", val("-A")?)),
            "-e" | "--referer" => headers.push(KV::new("Referer", val("-e")?)),
            "-b" | "--cookie" => headers.push(KV::new("Cookie", val("-b")?)),
            "--url" => url = Some(val("--url")?),
            "-G" | "--get" => get = true,
            "-I" | "--head" => head = true,
            "--oauth2-bearer" => headers.push(KV::new("Authorization", format!("Bearer {}", val("--oauth2-bearer")?))),
            o if IGNORED_WITH_VALUE.contains(&o) => {
                val(o)?;
            }
            o if o.starts_with('-') && o.len() > 1 => {} // boolean flags: -L -k -s -v --compressed ...
            _ => {
                if url.is_none() {
                    url = Some(tok)
                }
            }
        }
    }

    let Some(url) = url else { bail!("curl command has no URL") };
    let mut req = Request { url: Url::parse(&url), header: headers, ..Default::default() };

    for u in &urlencode {
        // "name=content" or "content": content gets URL-encoded by curl; we store it decoded
        data.push(match u.split_once('=') {
            Some((k, v)) if !k.is_empty() => format!("{k}={}", crate::http::form_encode(v)),
            Some((_, v)) => crate::http::form_encode(v),
            None => crate::http::form_encode(u),
        });
    }
    let has_body = !data.is_empty() || !form.is_empty() || binary_file.is_some();

    if get && !data.is_empty() {
        let mut q = req.url.query.clone();
        q.extend(crate::model::parse_query(&data.join("&")));
        req.url.query = q;
        req.url.sync_raw_from_query();
    } else if !form.is_empty() {
        req.body = Some(Body { mode: "formdata".into(), formdata: form, ..Default::default() });
    } else if let Some(path) = binary_file {
        req.body = Some(Body { mode: "file".into(), file: Some(serde_json::json!({ "src": path })), ..Default::default() });
    } else if !data.is_empty() {
        let text = data.join("&");
        let ct = req.header.iter().find(|h| h.key.eq_ignore_ascii_case("content-type")).map(|h| h.value.to_lowercase());
        let looks_json = serde_json::from_str::<serde_json::Value>(&text).map(|v| v.is_object() || v.is_array()).unwrap_or(false);
        if json || ct.as_deref().map(|c| c.contains("json")).unwrap_or(false) || (ct.is_none() && looks_json) {
            if json && ct.is_none() {
                req.header.push(KV::new("Content-Type", "application/json"));
                req.header.push(KV::new("Accept", "application/json"));
            }
            req.body = Some(Body::raw(&text, "json"));
        } else if ct.is_none() || ct.as_deref().map(|c| c.contains("x-www-form-urlencoded")).unwrap_or(false) {
            let fields = crate::model::parse_query(&text)
                .into_iter()
                .map(|k| {
                    let dec = |s: &str| percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().to_string();
                    KV::new(dec(&k.key), dec(&k.value))
                })
                .collect();
            req.body = Some(Body { mode: "urlencoded".into(), urlencoded: fields, ..Default::default() });
        } else {
            let lang = match ct.as_deref() {
                Some(c) if c.contains("xml") => "xml",
                Some(c) if c.contains("html") => "html",
                _ => "text",
            };
            req.body = Some(Body::raw(&text, lang));
        }
    }

    req.method = match method {
        Some(m) => m,
        None if head => "HEAD".into(),
        None if has_body && !get => "POST".into(),
        None => "GET".into(),
    };
    if let Some(u) = user {
        let (name, pass) = u.split_once(':').unwrap_or((&u, ""));
        req.auth = Some(Auth::new("basic", &[("username", name), ("password", pass)]));
    }
    // An `Authorization: Bearer x` header becomes a bearer auth helper.
    if req.auth.is_none() {
        if let Some(pos) = req.header.iter().position(|h| h.key.eq_ignore_ascii_case("authorization") && h.value.to_lowercase().starts_with("bearer ")) {
            let h = req.header.remove(pos);
            req.auth = Some(Auth::new("bearer", &[("token", h.value[7..].trim())]));
        }
    }
    Ok(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer() {
        let t = tokenize("curl 'a b' \"c \\\"d\\\" $x\" e\\ f \\\n  $'g\\nh' ''").unwrap();
        assert_eq!(t, ["curl", "a b", "c \"d\" $x", "e f", "g\nh", ""]);
        assert!(tokenize("curl 'oops").is_err());
    }

    #[test]
    fn json_post() {
        let r = parse(
            "curl -X POST https://api.test/users?x=1 \\\n -H 'Content-Type: application/json' -H 'Authorization: Bearer abc' \\\n --data-raw '{\"name\":\"n\"}' --compressed -L",
        )
        .unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url.raw, "https://api.test/users?x=1");
        assert_eq!(r.url.query[0].key, "x");
        let b = r.body.unwrap();
        assert_eq!((b.mode.as_str(), b.language()), ("raw", "json"));
        assert_eq!(r.auth.unwrap().get("token"), "abc");
        assert_eq!(r.header.len(), 1);
    }

    #[test]
    fn forms_and_basic_auth() {
        let r = parse("curl -u bob:secret -d a=1 -d 'b=x%20y' http://h/form").unwrap();
        assert_eq!(r.method, "POST");
        let b = r.body.unwrap();
        assert_eq!(b.mode, "urlencoded");
        assert_eq!(b.urlencoded[1].value, "x y");
        assert_eq!(r.auth.unwrap().get("password"), "secret");

        let r = parse("curl -F 'file=@/tmp/a.png;type=image/png' -F name=joe http://h/up").unwrap();
        let b = r.body.unwrap();
        assert_eq!(b.mode, "formdata");
        assert!(b.formdata[0].is_file());
        assert_eq!(b.formdata[0].file_paths(), ["/tmp/a.png"]);
        assert_eq!(b.formdata[1].value, "joe");
    }

    #[test]
    fn get_with_data_and_head() {
        let r = parse("curl -G http://h/s -d q=rust --data-urlencode 'p=a b'").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.url.raw, "http://h/s?q=rust&p=a+b");
        assert_eq!(parse("curl -I http://h").unwrap().method, "HEAD");
        assert_eq!(parse("curl -XDELETE --url http://h/1").unwrap().method, "DELETE");
        assert!(parse("wget http://h").is_err());
        assert!(parse("curl -s").is_err());
    }
}
