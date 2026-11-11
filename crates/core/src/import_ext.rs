//! Import of OpenAPI 3 / Swagger 2 definitions (JSON or YAML) and HAR archives.

use crate::import::Imported;
use crate::model::{Auth, Body, Collection, Example, Item, Request, Url, KV};
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub fn parse_json(v: &Value) -> Result<Option<Imported>> {
    if v.get("openapi").is_some() || v.get("swagger").is_some() {
        return Ok(Some(Imported::Collection(openapi(v)?)));
    }
    if v.get("log").and_then(|l| l.get("entries")).is_some() {
        return Ok(Some(Imported::Collection(har(v)?)));
    }
    Ok(None)
}

pub fn parse_text(t: &str) -> Result<Option<Imported>> {
    if !(t.contains("openapi") || t.contains("swagger")) {
        return Ok(None);
    }
    match serde_yaml::from_str::<Value>(t) {
        Ok(v) if v.is_object() => parse_json(&v),
        _ => Ok(None),
    }
}

// ------------------------------------------------------------------ OpenAPI

fn deref<'a>(root: &'a Value, v: &'a Value) -> &'a Value {
    let mut cur = v;
    for _ in 0..16 {
        match cur.get("$ref").and_then(|r| r.as_str()) {
            Some(r) if r.starts_with("#/") => {
                let pointer = r[1..].to_string();
                match root.pointer(&pointer) {
                    Some(t) => cur = t,
                    None => return cur,
                }
            }
            _ => return cur,
        }
    }
    cur
}

/// Build an example value from a JSON schema.
fn sample(root: &Value, schema: &Value, depth: usize) -> Value {
    let s = deref(root, schema);
    if depth > 6 {
        return Value::Null;
    }
    for k in ["example", "default"] {
        if let Some(v) = s.get(k) {
            return v.clone();
        }
    }
    if let Some(v) = s.get("examples").and_then(|e| e.as_array()).and_then(|a| a.first()) {
        return v.clone();
    }
    if let Some(v) = s.get("enum").and_then(|e| e.as_array()).and_then(|a| a.first()) {
        return v.clone();
    }
    if let Some(all) = s.get("allOf").and_then(|a| a.as_array()) {
        let mut merged = serde_json::Map::new();
        for part in all {
            if let Value::Object(o) = sample(root, part, depth + 1) {
                merged.extend(o);
            }
        }
        return Value::Object(merged);
    }
    for k in ["oneOf", "anyOf"] {
        if let Some(first) = s.get(k).and_then(|a| a.as_array()).and_then(|a| a.first()) {
            return sample(root, first, depth + 1);
        }
    }
    let ty = match s.get("type") {
        Some(Value::String(t)) => t.as_str(),
        Some(Value::Array(a)) => a.iter().filter_map(|t| t.as_str()).find(|t| *t != "null").unwrap_or("string"),
        _ if s.get("properties").is_some() => "object",
        _ if s.get("items").is_some() => "array",
        _ => "string",
    };
    match ty {
        "object" => {
            let mut o = serde_json::Map::new();
            if let Some(props) = s.get("properties").and_then(|p| p.as_object()) {
                for (k, v) in props {
                    if deref(root, v).get("readOnly").and_then(|r| r.as_bool()) != Some(true) || depth > 0 {
                        o.insert(k.clone(), sample(root, v, depth + 1));
                    }
                }
            }
            Value::Object(o)
        }
        "array" => json!([sample(root, s.get("items").unwrap_or(&Value::Null), depth + 1)]),
        "integer" => json!(s.get("minimum").and_then(|m| m.as_i64()).unwrap_or(0)),
        "number" => json!(s.get("minimum").and_then(|m| m.as_f64()).unwrap_or(0.0)),
        "boolean" => json!(true),
        "null" => Value::Null,
        _ => json!(match s.get("format").and_then(|f| f.as_str()) {
            Some("date-time") => "2024-01-01T00:00:00Z",
            Some("date") => "2024-01-01",
            Some("uuid") => "3fa85f64-5717-4562-b3fc-2c963f66afa6",
            Some("email") => "user@example.com",
            Some("uri") | Some("url") => "https://example.com",
            Some("byte") => "c3RyaW5n",
            Some("password") => "********",
            _ => "string",
        }),
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn param_value(root: &Value, p: &Value) -> String {
    if let Some(e) = p.get("example") {
        return crate::model::value_to_string(e);
    }
    if let Some(e) = p.get("examples").and_then(|e| e.as_object()).and_then(|o| o.values().next()) {
        return crate::model::value_to_string(deref(root, e).get("value").unwrap_or(&Value::Null));
    }
    // v3 keeps the schema nested, v2 inlines type/default on the parameter
    let schema = p.get("schema").map(|s| deref(root, s)).unwrap_or(p);
    for k in ["example", "default"] {
        if let Some(v) = schema.get(k) {
            return crate::model::value_to_string(v);
        }
    }
    if let Some(v) = schema.get("enum").and_then(|e| e.as_array()).and_then(|a| a.first()) {
        return crate::model::value_to_string(v);
    }
    String::new()
}

/// Example payload for a media type object (`example`, `examples`, or sampled from `schema`).
fn media_example(root: &Value, media: &Value) -> Option<Value> {
    if let Some(e) = media.get("example") {
        return Some(e.clone());
    }
    if let Some(e) = media.get("examples").and_then(|e| e.as_object()).and_then(|o| o.values().next()) {
        if let Some(v) = deref(root, e).get("value") {
            return Some(v.clone());
        }
    }
    media.get("schema").map(|s| sample(root, s, 0))
}

fn form_rows(root: &Value, schema: &Value) -> Vec<KV> {
    let s = deref(root, schema);
    s.get("properties")
        .and_then(|p| p.as_object())
        .map(|props| {
            props
                .iter()
                .map(|(k, v)| {
                    let d = deref(root, v);
                    let mut row = KV::new(k.clone(), "");
                    if d.get("format").and_then(|f| f.as_str()) == Some("binary") {
                        row.kind = Some("file".into());
                    } else {
                        row.kind = Some("text".into());
                        row.value = crate::model::value_to_string(&sample(root, v, 1));
                    }
                    row
                })
                .collect()
        })
        .unwrap_or_default()
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "",
    }
}

fn security_auth(root: &Value) -> Option<Auth> {
    let schemes = root.pointer("/components/securitySchemes").or_else(|| root.get("securityDefinitions"))?.as_object()?;
    let wanted = root.get("security").and_then(|s| s.as_array()).and_then(|a| a.first()).and_then(|o| o.as_object()).and_then(|o| o.keys().next().cloned());
    let (_, scheme) = match &wanted {
        Some(name) => schemes.iter().find(|(k, _)| *k == name)?,
        None => schemes.iter().next()?,
    };
    let scheme = deref(root, scheme);
    let ty = scheme.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match ty {
        "http" => match scheme.get("scheme").and_then(|s| s.as_str()).unwrap_or("").to_lowercase().as_str() {
            "basic" => Some(Auth::new("basic", &[("username", "{{username}}"), ("password", "{{password}}")])),
            "bearer" => Some(Auth::new("bearer", &[("token", "{{bearerToken}}")])),
            "digest" => Some(Auth::new("digest", &[("username", "{{username}}"), ("password", "{{password}}")])),
            _ => None,
        },
        "basic" => Some(Auth::new("basic", &[("username", "{{username}}"), ("password", "{{password}}")])),
        "apiKey" => Some(Auth::new(
            "apikey",
            &[("key", scheme.get("name").and_then(|n| n.as_str()).unwrap_or("X-API-Key")), ("value", "{{apiKey}}"), ("in", if scheme.get("in").and_then(|i| i.as_str()) == Some("query") { "query" } else { "header" })],
        )),
        "oauth2" | "openIdConnect" => Some(Auth::new("oauth2", &[("accessToken", "{{accessToken}}"), ("addTokenTo", "header")])),
        _ => None,
    }
}

pub fn openapi(root: &Value) -> Result<Collection> {
    let v3 = root.get("openapi").is_some();
    let Some(paths) = root.get("paths").and_then(|p| p.as_object()) else { bail!("OpenAPI document has no `paths`") };
    let mut c = Collection::new(root.pointer("/info/title").and_then(|t| t.as_str()).unwrap_or("Imported API"));
    c.info.description = root.pointer("/info/description").and_then(|d| d.as_str()).map(|s| s.to_string());

    let base = if v3 {
        let server = root.pointer("/servers/0").cloned().unwrap_or(Value::Null);
        let mut url = server.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
        if let Some(vars) = server.get("variables").and_then(|v| v.as_object()) {
            for (k, v) in vars {
                url = url.replace(&format!("{{{k}}}"), v.get("default").and_then(|d| d.as_str()).unwrap_or(""));
            }
        }
        url
    } else {
        let host = root.get("host").and_then(|h| h.as_str()).unwrap_or("");
        let scheme = root.pointer("/schemes/0").and_then(|s| s.as_str()).unwrap_or("https");
        let base_path = root.get("basePath").and_then(|b| b.as_str()).unwrap_or("");
        if host.is_empty() { base_path.to_string() } else { format!("{scheme}://{host}{base_path}") }
    };
    c.variable.push(KV::new("baseUrl", base.trim_end_matches('/')));
    c.auth = security_auth(root);
    if let Some(a) = &c.auth {
        for v in a.params.values() {
            if let Some(name) = v.strip_prefix("{{").and_then(|s| s.strip_suffix("}}")) {
                c.variable.push(KV::new(name, ""));
            }
        }
    }

    for (path, item) in paths {
        let item = deref(root, item);
        let shared: Vec<Value> = item.get("parameters").and_then(|p| p.as_array()).cloned().unwrap_or_default();
        for method in ["get", "post", "put", "patch", "delete", "head", "options", "trace"] {
            let Some(op) = item.get(method) else { continue };
            let name = op
                .get("summary")
                .and_then(|s| s.as_str())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| op.get("operationId").and_then(|s| s.as_str()))
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{} {path}", method.to_uppercase()));
            // `{id}` -> `:id`
            let postman_path = path.split('/').map(|seg| match seg.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                Some(n) => format!(":{n}"),
                None => seg.to_string(),
            }).collect::<Vec<_>>().join("/");
            let mut req = Request { method: method.to_uppercase(), url: Url::parse(&format!("{{{{baseUrl}}}}{postman_path}")), ..Default::default() };
            req.description = op.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());

            let mut params: Vec<Value> = shared.clone();
            params.extend(op.get("parameters").and_then(|p| p.as_array()).cloned().unwrap_or_default());
            let mut form: Vec<KV> = vec![];
            let mut body_schema: Option<Value> = None;
            for p in &params {
                let p = deref(root, p);
                let pname = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let required = p.get("required").and_then(|r| r.as_bool()).unwrap_or(false);
                let description = p.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                let value = param_value(root, p);
                match p.get("in").and_then(|i| i.as_str()).unwrap_or("") {
                    "query" => req.url.query.push(KV { key: pname, value, disabled: !required, description, ..Default::default() }),
                    "header" => req.header.push(KV { key: pname, value, disabled: !required, description, ..Default::default() }),
                    "path" => {
                        if let Some(slot) = req.url.variable.iter_mut().find(|k| k.key == pname) {
                            slot.value = value;
                            slot.description = description;
                        }
                    }
                    "formData" => {
                        let mut row = KV { key: pname, value, description, ..Default::default() };
                        row.kind = Some(if p.get("type").and_then(|t| t.as_str()) == Some("file") { "file" } else { "text" }.into());
                        form.push(row);
                    }
                    "body" => body_schema = p.get("schema").cloned(),
                    _ => {}
                }
            }
            req.url.sync_raw_from_query();

            if let Some(content) = op.get("requestBody").map(|b| deref(root, b)).and_then(|b| b.get("content")).and_then(|c| c.as_object()) {
                let pick = |needle: &str| content.iter().find(|(k, _)| k.contains(needle));
                if let Some((_, media)) = pick("json") {
                    req.body = Some(Body::raw(&text_of(&media_example(root, media).unwrap_or(json!({}))), "json"));
                } else if let Some((_, media)) = pick("x-www-form-urlencoded") {
                    req.body = Some(Body { mode: "urlencoded".into(), urlencoded: form_rows(root, media.get("schema").unwrap_or(&Value::Null)), ..Default::default() });
                } else if let Some((_, media)) = pick("multipart") {
                    req.body = Some(Body { mode: "formdata".into(), formdata: form_rows(root, media.get("schema").unwrap_or(&Value::Null)), ..Default::default() });
                } else if let Some((ct, media)) = content.iter().next() {
                    let lang = if ct.contains("xml") { "xml" } else { "text" };
                    req.body = Some(Body::raw(&text_of(&media_example(root, media).unwrap_or(Value::Null)), lang));
                    req.header.push(KV::new("Content-Type", ct.clone()));
                }
            } else if let Some(schema) = body_schema {
                req.body = Some(Body::raw(&text_of(&sample(root, &schema, 0)), "json"));
            } else if !form.is_empty() {
                let multipart = form.iter().any(|f| f.is_file()) || op.get("consumes").and_then(|c| c.as_array()).map(|a| a.iter().any(|x| x.as_str().map(|s| s.contains("multipart")).unwrap_or(false))).unwrap_or(false);
                req.body = Some(if multipart {
                    Body { mode: "formdata".into(), formdata: form, ..Default::default() }
                } else {
                    Body { mode: "urlencoded".into(), urlencoded: form, ..Default::default() }
                });
            }
            if op.get("security").and_then(|s| s.as_array()).map(|a| a.is_empty()).unwrap_or(false) {
                req.auth = Some(Auth::new("noauth", &[]));
            }

            let mut it = Item::request(&name, req.clone());
            if let Some(responses) = op.get("responses").and_then(|r| r.as_object()) {
                for (code, resp) in responses {
                    let resp = deref(root, resp);
                    let code_num: u16 = code.parse().unwrap_or(if code == "default" { 500 } else { 200 });
                    let (ct, body) = if let Some(content) = resp.get("content").and_then(|c| c.as_object()) {
                        match content.iter().find(|(k, _)| k.contains("json")).or_else(|| content.iter().next()) {
                            Some((ct, media)) => (Some(ct.clone()), media_example(root, media)),
                            None => (None, None),
                        }
                    } else if let Some(examples) = resp.get("examples").and_then(|e| e.as_object()) {
                        examples.iter().next().map(|(ct, v)| (Some(ct.clone()), Some(v.clone()))).unwrap_or((None, None))
                    } else if let Some(schema) = resp.get("schema") {
                        (Some("application/json".to_string()), Some(sample(root, schema, 0)))
                    } else {
                        (None, None)
                    };
                    let description = resp.get("description").and_then(|d| d.as_str()).unwrap_or("").trim().to_string();
                    it.response.push(Example {
                        id: crate::model::new_id(),
                        name: if description.is_empty() { code.clone() } else { description },
                        original_request: Some(req.clone()),
                        status: status_text(code_num).into(),
                        code: code_num,
                        header: ct.iter().map(|c| KV::new("Content-Type", c.clone())).collect(),
                        body: body.as_ref().map(text_of).unwrap_or_default(),
                        preview_language: ct.as_ref().map(|c| if c.contains("json") { "json" } else if c.contains("xml") { "xml" } else if c.contains("html") { "html" } else { "text" }.to_string()),
                    });
                }
            }

            let tag = op.get("tags").and_then(|t| t.as_array()).and_then(|a| a.first()).and_then(|t| t.as_str());
            match tag {
                Some(tag) => {
                    let folder_id = match c.item.iter().find(|i| i.is_folder() && i.name == tag) {
                        Some(f) => f.id.clone(),
                        None => {
                            let mut f = Item::folder(tag);
                            f.description = root.get("tags").and_then(|t| t.as_array()).and_then(|a| a.iter().find(|x| x.get("name").and_then(|n| n.as_str()) == Some(tag))).and_then(|t| t.get("description")).and_then(|d| d.as_str()).map(|s| s.to_string());
                            let id = f.id.clone();
                            c.item.push(f);
                            id
                        }
                    };
                    c.add_item(Some(&folder_id), it)?;
                }
                None => c.item.push(it),
            }
        }
    }
    Ok(c)
}

// ------------------------------------------------------------------ HAR

pub fn har(root: &Value) -> Result<Collection> {
    let Some(entries) = root.pointer("/log/entries").and_then(|e| e.as_array()) else { bail!("HAR file has no log.entries") };
    let mut c = Collection::new("HAR import");
    for e in entries {
        let r = &e["request"];
        let Some(url) = r["url"].as_str() else { continue };
        let mut req = Request::new(r["method"].as_str().unwrap_or("GET"), url);
        for h in r["headers"].as_array().cloned().unwrap_or_default() {
            let name = h["name"].as_str().unwrap_or("");
            let lower = name.to_lowercase();
            if name.starts_with(':') || ["content-length", "host", "connection", "cookie", "accept-encoding"].contains(&lower.as_str()) {
                continue;
            }
            req.header.push(KV::new(name, h["value"].as_str().unwrap_or("")));
        }
        let post = &r["postData"];
        if post.is_object() {
            let mime = post["mimeType"].as_str().unwrap_or("").to_lowercase();
            let params: Vec<KV> = post["params"].as_array().map(|a| a.iter().map(|p| KV::new(p["name"].as_str().unwrap_or(""), p["value"].as_str().unwrap_or(""))).collect()).unwrap_or_default();
            if mime.contains("x-www-form-urlencoded") && !params.is_empty() {
                req.body = Some(Body { mode: "urlencoded".into(), urlencoded: params, ..Default::default() });
            } else if mime.contains("multipart") && !params.is_empty() {
                req.header.retain(|h| !h.key.eq_ignore_ascii_case("content-type"));
                req.body = Some(Body { mode: "formdata".into(), formdata: params, ..Default::default() });
            } else if let Some(text) = post["text"].as_str() {
                let lang = if mime.contains("json") { "json" } else if mime.contains("xml") { "xml" } else { "text" };
                req.body = Some(Body::raw(text, lang));
            }
        }
        let path = url::Url::parse(url).map(|u| u.path().to_string()).unwrap_or_else(|_| url.to_string());
        let mut item = Item::request(&format!("{} {path}", req.method), req.clone());
        let resp = &e["response"];
        if let Some(status) = resp["status"].as_u64().filter(|s| *s > 0) {
            let body = resp.pointer("/content/text").and_then(|t| t.as_str()).unwrap_or("");
            if resp.pointer("/content/encoding").and_then(|x| x.as_str()) != Some("base64") {
                item.response.push(Example {
                    id: crate::model::new_id(),
                    name: format!("{status} {}", resp["statusText"].as_str().unwrap_or("")).trim().to_string(),
                    original_request: Some(req),
                    status: resp["statusText"].as_str().unwrap_or("").to_string(),
                    code: status as u16,
                    header: resp["headers"].as_array().map(|a| a.iter().filter(|h| !h["name"].as_str().unwrap_or("").starts_with(':')).map(|h| KV::new(h["name"].as_str().unwrap_or(""), h["value"].as_str().unwrap_or(""))).collect()).unwrap_or_default(),
                    body: body.to_string(),
                    preview_language: None,
                });
            }
        }
        c.item.push(item);
    }
    if c.item.is_empty() {
        bail!("HAR file contains no requests");
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PETSTORE: &str = r##"
openapi: 3.0.3
info:
  title: Pet Store
  description: Sample API
servers:
  - url: https://{region}.petstore.test/v1/
    variables:
      region: { default: eu }
security:
  - token: []
tags:
  - name: pets
    description: Everything about pets
paths:
  /pets:
    get:
      tags: [pets]
      summary: List pets
      parameters:
        - { name: limit, in: query, required: true, schema: { type: integer, default: 20 } }
        - { name: tag, in: query, schema: { type: string } }
        - { name: X-Request-Id, in: header, required: true, example: abc }
      responses:
        "200":
          description: A list of pets
          content:
            application/json:
              schema: { type: array, items: { $ref: "#/components/schemas/Pet" } }
    post:
      tags: [pets]
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema: { $ref: "#/components/schemas/Pet" }
      responses:
        "201": { description: Created, content: { application/json: { example: { id: 7, name: Rex } } } }
        "400": { description: Invalid input }
  /pets/{petId}:
    parameters:
      - { name: petId, in: path, required: true, schema: { type: integer, example: 42 } }
    get:
      security: []
      responses:
        default: { description: unexpected error }
    delete:
      summary: Delete pet
      responses:
        "204": { description: Deleted }
  /upload:
    post:
      requestBody:
        content:
          multipart/form-data:
            schema:
              type: object
              properties:
                file: { type: string, format: binary }
                note: { type: string, example: hi }
      responses: {}
components:
  securitySchemes:
    token: { type: http, scheme: bearer }
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        id: { type: integer, readOnly: true }
        name: { type: string, example: Rex }
        status: { type: string, enum: [available, sold] }
        born: { type: string, format: date }
        owner: { $ref: "#/components/schemas/Owner" }
    Owner:
      type: object
      properties:
        email: { type: string, format: email }
        pets: { type: array, items: { $ref: "#/components/schemas/Pet" } }
"##;

    #[test]
    fn openapi3_yaml() {
        let Imported::Collection(c) = crate::import::parse(PETSTORE).unwrap() else { panic!("not a collection") };
        assert_eq!(c.info.name, "Pet Store");
        assert_eq!(c.variables()["baseUrl"], "https://eu.petstore.test/v1");
        assert_eq!(c.auth.as_ref().unwrap().kind, "bearer");
        assert!(c.variables().contains_key("bearerToken"));
        let paths: Vec<String> = c.requests().iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(paths, ["pets/List pets", "pets/createPet", "GET /pets/{petId}", "Delete pet", "POST /upload"]);
        assert_eq!(c.item[0].description.as_deref(), Some("Everything about pets"));

        let list = c.resolve("List pets").unwrap();
        let r = list.request.as_ref().unwrap();
        assert_eq!(r.url.raw, "{{baseUrl}}/pets?limit=20");
        assert!(r.url.query[1].disabled);
        assert_eq!((r.header[0].key.as_str(), r.header[0].value.as_str()), ("X-Request-Id", "abc"));
        let ex: Value = serde_json::from_str(&list.response[0].body).unwrap();
        assert_eq!(ex[0]["name"], "Rex");
        assert_eq!(ex[0]["status"], "available");
        assert_eq!(ex[0]["owner"]["email"], "user@example.com");

        let create = c.resolve("createPet").unwrap();
        let body: Value = serde_json::from_str(create.request.as_ref().unwrap().body.as_ref().unwrap().raw.as_ref().unwrap()).unwrap();
        assert!(body.get("id").is_none(), "readOnly properties are left out of request bodies");
        assert_eq!(body["born"], "2024-01-01");
        assert_eq!(create.response.iter().map(|e| e.code).collect::<Vec<_>>(), [201, 400]);
        assert_eq!(serde_json::from_str::<Value>(&create.response[0].body).unwrap()["id"], 7);

        let get = c.resolve("GET /pets/{petId}").unwrap().request.as_ref().unwrap();
        assert_eq!(get.url.raw, "{{baseUrl}}/pets/:petId");
        assert_eq!(get.url.variable[0].value, "42");
        assert_eq!(get.auth.as_ref().unwrap().kind, "noauth");
        let up = c.resolve("POST /upload").unwrap().request.as_ref().unwrap().body.as_ref().unwrap();
        assert_eq!(up.mode, "formdata");
        assert!(up.formdata[0].is_file());
        assert_eq!(up.formdata[1].value, "hi");
    }

    #[test]
    fn swagger2_json_and_har() {
        let doc = json!({
            "swagger": "2.0", "info": {"title": "Old"}, "host": "api.old.test", "basePath": "/v2", "schemes": ["http"],
            "securityDefinitions": {"key": {"type": "apiKey", "name": "X-Key", "in": "header"}},
            "paths": {"/login": {"post": {
                "consumes": ["application/x-www-form-urlencoded"],
                "parameters": [{"name": "user", "in": "formData", "type": "string", "default": "bob"}],
                "responses": {"200": {"description": "ok", "schema": {"type": "object", "properties": {"token": {"type": "string"}}}}}
            }}, "/items": {"put": {"parameters": [{"name": "body", "in": "body", "schema": {"type": "object", "properties": {"n": {"type": "integer"}}}}], "responses": {}}}}
        });
        let Imported::Collection(c) = crate::import::parse(&doc.to_string()).unwrap() else { panic!() };
        assert_eq!(c.variables()["baseUrl"], "http://api.old.test/v2");
        assert_eq!(c.auth.as_ref().unwrap().get("key"), "X-Key");
        let login = c.resolve("POST /login").unwrap();
        assert_eq!(login.request.as_ref().unwrap().body.as_ref().unwrap().urlencoded[0].value, "bob");
        assert!(login.response[0].body.contains("token"));
        assert!(c.resolve("PUT /items").unwrap().request.as_ref().unwrap().body.as_ref().unwrap().raw.as_ref().unwrap().contains("\"n\": 0"));

        let har = json!({"log": {"entries": [
            {"request": {"method": "POST", "url": "https://h.test/api/login?x=1", "headers": [{"name": ":authority", "value": "h.test"}, {"name": "Content-Type", "value": "application/json"}, {"name": "Cookie", "value": "a=b"}],
                         "postData": {"mimeType": "application/json", "text": "{\"u\":1}"}},
             "response": {"status": 200, "statusText": "OK", "headers": [{"name": "Content-Type", "value": "application/json"}], "content": {"text": "{\"ok\":true}"}}},
            {"request": {"method": "GET", "url": "https://h.test/img.png", "headers": []}, "response": {"status": 200, "content": {"text": "AAAA", "encoding": "base64"}}}
        ]}});
        let Imported::Collection(c) = crate::import::parse(&har.to_string()).unwrap() else { panic!() };
        assert_eq!(c.requests().len(), 2);
        let first = &c.item[0];
        assert_eq!(first.name, "POST /api/login");
        let r = first.request.as_ref().unwrap();
        assert_eq!(r.header.len(), 1);
        assert_eq!(r.body.as_ref().unwrap().language(), "json");
        assert_eq!(first.response[0].body, "{\"ok\":true}");
        assert!(c.item[1].response.is_empty());
    }
}
