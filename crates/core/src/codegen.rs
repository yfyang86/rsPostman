//! Code snippet generation from a prepared (fully resolved) request.

use crate::http::{BodySpec, Prepared};
use anyhow::{bail, Result};

/// `(id, display name)` of the supported targets.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("curl", "cURL"),
    ("http", "HTTP (raw)"),
    ("httpie", "HTTPie"),
    ("wget", "Wget"),
    ("python", "Python - requests"),
    ("javascript", "JavaScript - fetch"),
    ("node", "Node.js - axios"),
    ("go", "Go - net/http"),
    ("rust", "Rust - reqwest"),
    ("java", "Java - OkHttp"),
    ("csharp", "C# - HttpClient"),
    ("php", "PHP - cURL"),
    ("ruby", "Ruby - Net::HTTP"),
    ("powershell", "PowerShell - Invoke-RestMethod"),
];

fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Double-quoted string literal valid in JS, Python, Go, Java, C#, Rust, Ruby and PHP.
fn dq(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '$' => o.push_str("\\$"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Like `dq` but without escaping `$` (languages without interpolation).
fn q(s: &str) -> String {
    dq(s).replace("\\$", "$")
}

fn body_text(p: &Prepared) -> Option<String> {
    match &p.body {
        BodySpec::Raw { .. } | BodySpec::Urlencoded { .. } => Some(p.body.preview()),
        _ => None,
    }
}

/// Headers without a multipart content type (libraries generate their own boundary).
fn headers(p: &Prepared) -> Vec<(String, String)> {
    let multipart = matches!(p.body, BodySpec::Form { .. });
    p.headers.iter().filter(|(k, _)| !(multipart && k.eq_ignore_ascii_case("content-type"))).cloned().collect()
}

pub fn generate(lang: &str, p: &Prepared) -> Result<String> {
    let hs = headers(p);
    let body = body_text(p);
    let m = p.method.as_str();
    Ok(match lang {
        "curl" => {
            let mut l = vec![format!("curl --location --request {m} {}", sh(&p.url))];
            for (k, v) in &hs {
                l.push(format!("--header {}", sh(&format!("{k}: {v}"))));
            }
            match &p.body {
                BodySpec::None => {}
                BodySpec::Raw { text } => l.push(format!("--data-raw {}", sh(text))),
                BodySpec::Urlencoded { fields } => {
                    for (k, v) in fields {
                        l.push(format!("--data-urlencode {}", sh(&format!("{k}={v}"))));
                    }
                }
                BodySpec::Form { parts, .. } => {
                    for f in parts {
                        l.push(format!("--form {}", sh(&if f.file { format!("{}=@\"{}\"", f.name, f.value) } else { format!("{}=\"{}\"", f.name, f.value) })));
                    }
                }
                BodySpec::File { path } => l.push(format!("--data-binary {}", sh(&format!("@{path}")))),
            }
            l.join(" \\\n  ")
        }
        "http" => {
            let u = url::Url::parse(&p.url)?;
            let target = match u.query() {
                Some(qs) => format!("{}?{qs}", u.path()),
                None => u.path().to_string(),
            };
            let host = match u.port() {
                Some(port) => format!("{}:{port}", u.host_str().unwrap_or("")),
                None => u.host_str().unwrap_or("").to_string(),
            };
            let mut s = format!("{m} {target} HTTP/1.1\nHost: {host}\n");
            for (k, v) in &p.headers {
                s.push_str(&format!("{k}: {v}\n"));
            }
            let b = match &p.body {
                BodySpec::Form { .. } => String::from_utf8_lossy(&p.body.bytes().unwrap_or_default().unwrap_or_default()).replace("\r\n", "\n"),
                BodySpec::File { path } => format!("<contents of {path}>"),
                _ => body.clone().unwrap_or_default(),
            };
            if !b.is_empty() {
                s.push_str(&format!("Content-Length: {}\n\n{b}", b.len()));
            }
            s
        }
        "httpie" => {
            let mut l = vec![];
            let mut prefix = String::new();
            match &p.body {
                BodySpec::Raw { text } => prefix = format!("printf '%s' {} | ", sh(text)),
                BodySpec::File { path } => l.push(format!("@{}", sh(path))),
                _ => {}
            }
            let flag = match &p.body {
                BodySpec::Urlencoded { .. } => "--form ",
                BodySpec::Form { .. } => "--multipart ",
                _ => "",
            };
            let mut out = vec![format!("{prefix}http {flag}--follow {m} {}", sh(&p.url))];
            for (k, v) in &hs {
                out.push(sh(&format!("{k}:{v}")));
            }
            match &p.body {
                BodySpec::Urlencoded { fields } => out.extend(fields.iter().map(|(k, v)| sh(&format!("{k}={v}")))),
                BodySpec::Form { parts, .. } => out.extend(parts.iter().map(|f| sh(&if f.file { format!("{}@{}", f.name, f.value) } else { format!("{}={}", f.name, f.value) }))),
                _ => {}
            }
            out.extend(l);
            out.join(" \\\n  ")
        }
        "wget" => {
            let mut l = vec![format!("wget --no-check-certificate --quiet --method {m}")];
            for (k, v) in &p.headers {
                l.push(format!("--header {}", sh(&format!("{k}: {v}"))));
            }
            match &p.body {
                BodySpec::None => {}
                BodySpec::File { path } => l.push(format!("--body-file={}", sh(path))),
                BodySpec::Form { .. } => l.push(format!("--body-data {}", sh(&String::from_utf8_lossy(&p.body.bytes()?.unwrap_or_default())))),
                _ => l.push(format!("--body-data {}", sh(&body.clone().unwrap_or_default()))),
            }
            l.push(format!("-O - {}", sh(&p.url)));
            l.join(" \\\n  ")
        }
        "python" => {
            let mut s = format!("import requests\n\nurl = {}\n\n", q(&p.url));
            s.push_str("headers = {\n");
            for (k, v) in &hs {
                s.push_str(&format!("    {}: {},\n", q(k), q(v)));
            }
            s.push_str("}\n");
            let mut args = String::from("headers=headers");
            match &p.body {
                BodySpec::None => {}
                BodySpec::Raw { text } => {
                    s.push_str(&format!("payload = {}\n", q(text)));
                    args.push_str(", data=payload.encode(\"utf-8\")");
                }
                BodySpec::Urlencoded { fields } => {
                    s.push_str("payload = {\n");
                    for (k, v) in fields {
                        s.push_str(&format!("    {}: {},\n", q(k), q(v)));
                    }
                    s.push_str("}\n");
                    args.push_str(", data=payload");
                }
                BodySpec::Form { parts, .. } => {
                    s.push_str("files = [\n");
                    for f in parts {
                        if f.file {
                            s.push_str(&format!("    ({}, open({}, \"rb\")),\n", q(&f.name), q(&f.value)));
                        } else {
                            s.push_str(&format!("    ({}, (None, {})),\n", q(&f.name), q(&f.value)));
                        }
                    }
                    s.push_str("]\n");
                    args.push_str(", files=files");
                }
                BodySpec::File { path } => {
                    s.push_str(&format!("payload = open({}, \"rb\")\n", q(path)));
                    args.push_str(", data=payload");
                }
            }
            s.push_str(&format!("\nresponse = requests.request({}, url, {args})\n\nprint(response.status_code)\nprint(response.text)\n", q(m)));
            s
        }
        "javascript" | "node" => {
            let axios = lang == "node";
            let mut s = String::new();
            if axios {
                s.push_str("const axios = require(\"axios\");\n");
                if matches!(p.body, BodySpec::Form { .. }) {
                    s.push_str("const FormData = require(\"form-data\");\nconst fs = require(\"fs\");\n");
                }
                s.push('\n');
            }
            s.push_str("const headers = {\n");
            for (k, v) in &hs {
                s.push_str(&format!("  {}: {},\n", dq(k), dq(v)));
            }
            s.push_str("};\n\n");
            let mut has_body = true;
            match &p.body {
                BodySpec::None => has_body = false,
                BodySpec::Raw { text } => s.push_str(&format!("const body = {};\n\n", dq(text))),
                BodySpec::Urlencoded { fields } => {
                    s.push_str("const body = new URLSearchParams();\n");
                    for (k, v) in fields {
                        s.push_str(&format!("body.append({}, {});\n", dq(k), dq(v)));
                    }
                    s.push('\n');
                }
                BodySpec::Form { parts, .. } => {
                    s.push_str("const body = new FormData();\n");
                    for f in parts {
                        if f.file && axios {
                            s.push_str(&format!("body.append({}, fs.createReadStream({}));\n", dq(&f.name), dq(&f.value)));
                        } else if f.file {
                            s.push_str(&format!("body.append({}, fileInput.files[0], {});\n", dq(&f.name), dq(&f.value)));
                        } else {
                            s.push_str(&format!("body.append({}, {});\n", dq(&f.name), dq(&f.value)));
                        }
                    }
                    s.push('\n');
                }
                BodySpec::File { path } => {
                    if axios {
                        s.push_str(&format!("const body = require(\"fs\").readFileSync({});\n\n", dq(path)));
                    } else {
                        s.push_str(&format!("const body = fileInput.files[0]; // {path}\n\n"));
                    }
                }
            }
            if axios {
                s.push_str(&format!(
                    "axios({{\n  method: {},\n  url: {},\n  headers,{}\n}})\n  .then((response) => console.log(JSON.stringify(response.data)))\n  .catch((error) => console.error(error));\n",
                    dq(&m.to_lowercase()),
                    dq(&p.url),
                    if has_body { "\n  data: body," } else { "" }
                ));
            } else {
                s.push_str(&format!(
                    "fetch({}, {{\n  method: {},\n  headers,{}\n  redirect: \"follow\",\n}})\n  .then((response) => response.text())\n  .then((result) => console.log(result))\n  .catch((error) => console.error(error));\n",
                    dq(&p.url),
                    dq(m),
                    if has_body { "\n  body," } else { "" }
                ));
            }
            s
        }
        "go" => {
            let payload = match &p.body {
                BodySpec::None => None,
                BodySpec::File { path } => Some(format!("<contents of {path}>")),
                BodySpec::Form { .. } => Some(String::from_utf8_lossy(&p.body.bytes()?.unwrap_or_default()).to_string()),
                _ => body.clone(),
            };
            let mut s = String::from("package main\n\nimport (\n\t\"fmt\"\n\t\"io\"\n\t\"net/http\"\n\t\"strings\"\n)\n\nfunc main() {\n");
            match &payload {
                Some(t) => s.push_str(&format!("\tpayload := strings.NewReader({})\n", q(t))),
                None => s.push_str("\tpayload := strings.NewReader(\"\")\n"),
            }
            s.push_str(&format!("\treq, err := http.NewRequest({}, {}, payload)\n\tif err != nil {{\n\t\tpanic(err)\n\t}}\n", q(m), q(&p.url)));
            for (k, v) in &p.headers {
                s.push_str(&format!("\treq.Header.Add({}, {})\n", q(k), q(v)));
            }
            s.push_str("\n\tres, err := http.DefaultClient.Do(req)\n\tif err != nil {\n\t\tpanic(err)\n\t}\n\tdefer res.Body.Close()\n\n\tbody, _ := io.ReadAll(res.Body)\n\tfmt.Println(res.Status)\n\tfmt.Println(string(body))\n}\n");
            s
        }
        "rust" => {
            let mut s = String::from("// [dependencies] reqwest = { version = \"0.12\", features = [\"blocking\", \"multipart\"] }\nfn main() -> Result<(), Box<dyn std::error::Error>> {\n    let client = reqwest::blocking::Client::new();\n");
            s.push_str(&format!("    let response = client\n        .request(reqwest::Method::from_bytes(b{})?, {})\n", q(m), q(&p.url)));
            for (k, v) in &hs {
                s.push_str(&format!("        .header({}, {})\n", q(k), q(v)));
            }
            match &p.body {
                BodySpec::None => {}
                BodySpec::Raw { text } => s.push_str(&format!("        .body({})\n", q(text))),
                BodySpec::Urlencoded { fields } => {
                    let items: Vec<String> = fields.iter().map(|(k, v)| format!("({}, {})", q(k), q(v))).collect();
                    s.push_str(&format!("        .form(&[{}])\n", items.join(", ")));
                }
                BodySpec::Form { parts, .. } => {
                    s.push_str("        .multipart(\n            reqwest::blocking::multipart::Form::new()\n");
                    for f in parts {
                        if f.file {
                            s.push_str(&format!("                .file({}, {})?\n", q(&f.name), q(&f.value)));
                        } else {
                            s.push_str(&format!("                .text({}, {})\n", q(&f.name), q(&f.value)));
                        }
                    }
                    s.push_str("        )\n");
                }
                BodySpec::File { path } => s.push_str(&format!("        .body(std::fs::read({})?)\n", q(path))),
            }
            s.push_str("        .send()?;\n\n    println!(\"{}\", response.status());\n    println!(\"{}\", response.text()?);\n    Ok(())\n}\n");
            s
        }
        "java" => {
            let ct = p.header("content-type").unwrap_or("text/plain").to_string();
            let mut s = String::from("OkHttpClient client = new OkHttpClient().newBuilder().build();\n");
            let body_expr = match &p.body {
                BodySpec::None => {
                    if ["GET", "HEAD"].contains(&m) { "null".to_string() } else { "RequestBody.create(new byte[0], null)".to_string() }
                }
                BodySpec::Form { parts, .. } => {
                    s.push_str("RequestBody body = new MultipartBody.Builder().setType(MultipartBody.FORM)\n");
                    for f in parts {
                        if f.file {
                            s.push_str(&format!("    .addFormDataPart({}, {}, RequestBody.create(new java.io.File({}), MediaType.parse(\"application/octet-stream\")))\n", q(&f.name), q(&f.value), q(&f.value)));
                        } else {
                            s.push_str(&format!("    .addFormDataPart({}, {})\n", q(&f.name), q(&f.value)));
                        }
                    }
                    s.push_str("    .build();\n");
                    "body".to_string()
                }
                BodySpec::File { path } => {
                    s.push_str(&format!("RequestBody body = RequestBody.create(new java.io.File({}), MediaType.parse({}));\n", q(path), q(&ct)));
                    "body".to_string()
                }
                _ => {
                    s.push_str(&format!("MediaType mediaType = MediaType.parse({});\nRequestBody body = RequestBody.create({}, mediaType);\n", q(&ct), q(&body.clone().unwrap_or_default())));
                    "body".to_string()
                }
            };
            s.push_str(&format!("Request request = new Request.Builder()\n    .url({})\n    .method({}, {body_expr})\n", q(&p.url), q(m)));
            for (k, v) in &hs {
                s.push_str(&format!("    .addHeader({}, {})\n", q(k), q(v)));
            }
            s.push_str("    .build();\nResponse response = client.newCall(request).execute();\nSystem.out.println(response.body().string());\n");
            s
        }
        "csharp" => {
            let mut s = format!("var client = new HttpClient();\nvar request = new HttpRequestMessage(new HttpMethod({}), {});\n", q(m), q(&p.url));
            for (k, v) in hs.iter().filter(|(k, _)| !k.to_lowercase().starts_with("content-")) {
                s.push_str(&format!("request.Headers.TryAddWithoutValidation({}, {});\n", q(k), q(v)));
            }
            let ct = p.header("content-type").unwrap_or("text/plain").split(';').next().unwrap_or("").trim().to_string();
            match &p.body {
                BodySpec::None => {}
                BodySpec::Urlencoded { fields } => {
                    s.push_str("request.Content = new FormUrlEncodedContent(new Dictionary<string, string>\n{\n");
                    for (k, v) in fields {
                        s.push_str(&format!("    {{ {}, {} }},\n", q(k), q(v)));
                    }
                    s.push_str("});\n");
                }
                BodySpec::Form { parts, .. } => {
                    s.push_str("var content = new MultipartFormDataContent();\n");
                    for f in parts {
                        if f.file {
                            s.push_str(&format!("content.Add(new StreamContent(File.OpenRead({})), {}, Path.GetFileName({}));\n", q(&f.value), q(&f.name), q(&f.value)));
                        } else {
                            s.push_str(&format!("content.Add(new StringContent({}), {});\n", q(&f.value), q(&f.name)));
                        }
                    }
                    s.push_str("request.Content = content;\n");
                }
                BodySpec::File { path } => s.push_str(&format!("request.Content = new StreamContent(File.OpenRead({}));\n", q(path))),
                BodySpec::Raw { text } => s.push_str(&format!("request.Content = new StringContent({}, System.Text.Encoding.UTF8, {});\n", q(text), q(&ct))),
            }
            s.push_str("var response = await client.SendAsync(request);\nConsole.WriteLine((int)response.StatusCode);\nConsole.WriteLine(await response.Content.ReadAsStringAsync());\n");
            s
        }
        "php" => {
            let mut s = format!("<?php\n\n$curl = curl_init();\n\ncurl_setopt_array($curl, [\n    CURLOPT_URL => {},\n    CURLOPT_RETURNTRANSFER => true,\n    CURLOPT_FOLLOWLOCATION => true,\n    CURLOPT_CUSTOMREQUEST => {},\n", dq(&p.url), dq(m));
            match &p.body {
                BodySpec::None => {}
                BodySpec::Form { parts, .. } => {
                    s.push_str("    CURLOPT_POSTFIELDS => [\n");
                    for f in parts {
                        if f.file {
                            s.push_str(&format!("        {} => new CURLFile({}),\n", dq(&f.name), dq(&f.value)));
                        } else {
                            s.push_str(&format!("        {} => {},\n", dq(&f.name), dq(&f.value)));
                        }
                    }
                    s.push_str("    ],\n");
                }
                BodySpec::File { path } => s.push_str(&format!("    CURLOPT_POSTFIELDS => file_get_contents({}),\n", dq(path))),
                _ => s.push_str(&format!("    CURLOPT_POSTFIELDS => {},\n", dq(&body.clone().unwrap_or_default()))),
            }
            s.push_str("    CURLOPT_HTTPHEADER => [\n");
            for (k, v) in &hs {
                s.push_str(&format!("        {},\n", dq(&format!("{k}: {v}"))));
            }
            s.push_str("    ],\n]);\n\n$response = curl_exec($curl);\ncurl_close($curl);\n\necho $response;\n");
            s
        }
        "ruby" => {
            let class = {
                let l = m.to_lowercase();
                let mut c = l.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            };
            let mut s = format!("require \"uri\"\nrequire \"net/http\"\n\nurl = URI({})\n\nhttp = Net::HTTP.new(url.host, url.port)\nhttp.use_ssl = url.scheme == \"https\"\n\nrequest = Net::HTTP::{class}.new(url)\n", q(&p.url));
            for (k, v) in &p.headers {
                s.push_str(&format!("request[{}] = {}\n", q(k), q(v)));
            }
            match &p.body {
                BodySpec::None => {}
                BodySpec::File { path } => s.push_str(&format!("request.body = File.binread({})\n", q(path))),
                BodySpec::Form { .. } => s.push_str(&format!("request.body = {}\n", q(&String::from_utf8_lossy(&p.body.bytes()?.unwrap_or_default())))),
                _ => s.push_str(&format!("request.body = {}\n", q(&body.clone().unwrap_or_default()))),
            }
            s.push_str("\nresponse = http.request(request)\nputs response.code\nputs response.read_body\n");
            s
        }
        "powershell" => {
            let ps = |s: &str| format!("'{}'", s.replace('\'', "''"));
            let mut s = String::from("$headers = @{\n");
            for (k, v) in hs.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("content-type")) {
                s.push_str(&format!("    {} = {}\n", ps(k), ps(v)));
            }
            s.push_str("}\n\n");
            let mut args = format!("-Uri {} -Method {m} -Headers $headers", ps(&p.url));
            if let Some(ct) = hs.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
                args.push_str(&format!(" -ContentType {}", ps(&ct.1)));
            }
            match &p.body {
                BodySpec::None => {}
                BodySpec::File { path } => args.push_str(&format!(" -InFile {}", ps(path))),
                BodySpec::Form { parts, .. } => {
                    s.push_str("$form = @{\n");
                    for f in parts {
                        if f.file {
                            s.push_str(&format!("    {} = Get-Item -Path {}\n", ps(&f.name), ps(&f.value)));
                        } else {
                            s.push_str(&format!("    {} = {}\n", ps(&f.name), ps(&f.value)));
                        }
                    }
                    s.push_str("}\n\n");
                    args.push_str(" -Form $form");
                }
                _ => {
                    s.push_str(&format!("$body = {}\n\n", ps(&body.clone().unwrap_or_default())));
                    args.push_str(" -Body $body");
                }
            }
            s.push_str(&format!("$response = Invoke-RestMethod {args}\n$response | ConvertTo-Json\n"));
            s
        }
        other => bail!("unknown language `{other}`; available: {}", LANGUAGES.iter().map(|l| l.0).collect::<Vec<_>>().join(", ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FormPart;

    fn sample(body: BodySpec) -> Prepared {
        Prepared {
            method: "POST".into(),
            url: "https://api.test/v1/items?x=1".into(),
            headers: vec![("Content-Type".into(), "application/json".into()), ("X-Quote".into(), "it's \"q\"".into())],
            body,
            digest: None,
        }
    }

    #[test]
    fn every_language_generates_for_every_body_kind() {
        let bodies = vec![
            BodySpec::None,
            BodySpec::Raw { text: "{\"a\": \"b's $x\"}".into() },
            BodySpec::Urlencoded { fields: vec![("a".into(), "1 2".into())] },
            BodySpec::Form { boundary: "B".into(), parts: vec![FormPart { name: "n".into(), value: "v".into(), file: false, content_type: None }] },
            BodySpec::File { path: "/tmp/x.bin".into() },
        ];
        for (lang, _) in LANGUAGES {
            for b in &bodies {
                let out = generate(lang, &sample(b.clone())).unwrap();
                assert!(out.contains("api.test"), "{lang}: {out}");
            }
        }
        assert!(generate("cobol", &sample(BodySpec::None)).is_err());
    }

    #[test]
    fn curl_is_shell_safe_and_reimportable() {
        let p = sample(BodySpec::Raw { text: "{\"a\": \"b's $x\"}".into() });
        let cmd = generate("curl", &p).unwrap();
        let back = crate::curl::parse(&cmd).unwrap();
        assert_eq!(back.method, "POST");
        assert_eq!(back.url.raw, p.url);
        assert_eq!(back.body.unwrap().raw.unwrap(), "{\"a\": \"b's $x\"}");
        assert_eq!(back.header[1].value, "it's \"q\"");
    }

    #[test]
    fn escaping() {
        assert_eq!(dq("a\"b\\c\n$d"), "\"a\\\"b\\\\c\\n\\$d\"");
        assert_eq!(q("$d"), "\"$d\"");
        assert_eq!(sh("it's"), "'it'\\''s'");
    }
}
