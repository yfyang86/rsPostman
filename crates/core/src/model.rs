//! Data model. The on-disk format is Postman Collection v2.1 / Postman environment JSON,
//! parsed leniently (v2.0 quirks such as string URLs and object-style auth are accepted).

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Map, Value};

pub const SCHEMA_V21: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

pub type VarMap = IndexMap<String, String>;

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn default_true() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

pub fn value_to_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(value_to_string(&Value::deserialize(d)?))
}

fn description<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Null => None,
        Value::String(s) => Some(s),
        Value::Object(o) => o.get("content").map(value_to_string),
        other => Some(other.to_string()),
    })
}

fn string_or_lines<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s.lines().map(|l| l.to_string()).collect(),
        Value::Array(a) => a.iter().map(value_to_string).collect(),
        _ => vec![],
    })
}

fn kv_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<KV>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s
            .lines()
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| KV::new(k.trim(), v.trim()))
            .collect(),
        Value::Array(a) => a
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) => s.split_once(':').map(|(k, v)| KV::new(k.trim(), v.trim())),
                other => serde_json::from_value(other).ok(),
            })
            .collect(),
        _ => vec![],
    })
}

/// Generic key/value row used for headers, query params, form fields and variables.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct KV {
    #[serde(default, deserialize_with = "lenient_string")]
    pub key: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub value: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "description")]
    pub description: Option<String>,
    /// `text` / `file` for form-data, `string`/`secret` for variables.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// File path(s) for form-data rows of type `file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<Value>,
}

impl KV {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        KV { key: key.into(), value: value.into(), ..Default::default() }
    }
    pub fn file_paths(&self) -> Vec<String> {
        match &self.src {
            Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
            Some(Value::Array(a)) => a.iter().map(value_to_string).collect(),
            _ => vec![],
        }
    }
    pub fn is_file(&self) -> bool {
        self.kind.as_deref() == Some("file")
    }
}

// ------------------------------------------------------------------ Url

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Url {
    pub raw: String,
    pub query: Vec<KV>,
    /// Path variables (`/users/:id`).
    pub variable: Vec<KV>,
}

pub fn parse_query(q: &str) -> Vec<KV> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => KV::new(k, v),
            None => KV::new(p, ""),
        })
        .collect()
}

impl Url {
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim().to_string();
        let no_hash = raw.split('#').next().unwrap_or("");
        let query = no_hash.split_once('?').map(|(_, q)| parse_query(q)).unwrap_or_default();
        let mut u = Url { raw, query, variable: vec![] };
        u.sync_path_variables();
        u
    }

    /// URL without query string and fragment.
    pub fn base(&self) -> String {
        let s = self.raw.split('#').next().unwrap_or("");
        s.split('?').next().unwrap_or("").to_string()
    }

    /// Names of `:name` path segments.
    pub fn path_variable_names(&self) -> Vec<String> {
        let base = self.base();
        let after = base.split_once("://").map(|(_, r)| r).unwrap_or(&base);
        after
            .split('/')
            .skip(1)
            .filter(|s| s.starts_with(':') && s.len() > 1)
            .map(|s| s[1..].to_string())
            .collect()
    }

    /// Keep `variable` in sync with `:name` segments of the path.
    pub fn sync_path_variables(&mut self) {
        let names = self.path_variable_names();
        let old = std::mem::take(&mut self.variable);
        self.variable = names
            .into_iter()
            .map(|n| old.iter().find(|k| k.key == n).cloned().unwrap_or_else(|| KV::new(n, "")))
            .collect();
    }

    /// Rebuild `raw` from the base and the enabled query rows.
    pub fn sync_raw_from_query(&mut self) {
        let mut s = self.base();
        let q: Vec<String> = self
            .query
            .iter()
            .filter(|k| !k.disabled && !(k.key.is_empty() && k.value.is_empty()))
            .map(|k| format!("{}={}", k.key, k.value))
            .collect();
        if !q.is_empty() {
            s.push('?');
            s.push_str(&q.join("&"));
        }
        self.raw = s;
    }

    fn parts(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("raw".into(), json!(self.raw));
        let base = self.base();
        let rest = match base.split_once("://") {
            Some((p, r)) => {
                m.insert("protocol".into(), json!(p));
                r.to_string()
            }
            None => base.clone(),
        };
        let (hostport, path) = match rest.split_once('/') {
            Some((h, p)) => (h.to_string(), Some(p.to_string())),
            None => (rest, None),
        };
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h.to_string(), Some(p.to_string())),
            _ => (hostport, None),
        };
        if !host.is_empty() {
            let segs: Vec<&str> = if host.starts_with("{{") { vec![host.as_str()] } else { host.split('.').collect() };
            m.insert("host".into(), json!(segs));
        }
        if let Some(p) = port {
            m.insert("port".into(), json!(p));
        }
        if let Some(p) = path {
            m.insert("path".into(), json!(p.split('/').collect::<Vec<_>>()));
        }
        if !self.query.is_empty() {
            m.insert("query".into(), serde_json::to_value(&self.query).unwrap_or(Value::Null));
        }
        if !self.variable.is_empty() {
            m.insert("variable".into(), serde_json::to_value(&self.variable).unwrap_or(Value::Null));
        }
        m
    }

    fn from_value(v: Value) -> Url {
        match v {
            Value::String(s) => Url::parse(&s),
            Value::Object(o) => {
                let list = |k: &str| -> Option<Vec<KV>> {
                    o.get(k).and_then(|v| v.as_array()).map(|a| {
                        a.iter().filter_map(|x| serde_json::from_value::<KV>(x.clone()).ok()).collect()
                    })
                };
                let join = |k: &str, sep: &str| -> String {
                    match o.get(k) {
                        Some(Value::Array(a)) => a.iter().map(value_to_string).collect::<Vec<_>>().join(sep),
                        Some(v) => value_to_string(v),
                        None => String::new(),
                    }
                };
                let raw = match o.get("raw").and_then(|r| r.as_str()) {
                    Some(r) => r.to_string(),
                    None => {
                        let mut s = String::new();
                        if let Some(p) = o.get("protocol").and_then(|p| p.as_str()) {
                            s.push_str(p);
                            s.push_str("://");
                        }
                        s.push_str(&join("host", "."));
                        let port = join("port", "");
                        if !port.is_empty() {
                            s.push(':');
                            s.push_str(&port);
                        }
                        let path = join("path", "/");
                        if !path.is_empty() {
                            s.push('/');
                            s.push_str(path.trim_start_matches('/'));
                        }
                        if let Some(q) = list("query") {
                            let q: Vec<String> =
                                q.iter().filter(|k| !k.disabled).map(|k| format!("{}={}", k.key, k.value)).collect();
                            if !q.is_empty() {
                                s.push('?');
                                s.push_str(&q.join("&"));
                            }
                        }
                        s
                    }
                };
                let mut u = Url::parse(&raw);
                if let Some(q) = list("query") {
                    u.query = q;
                }
                if let Some(vars) = list("variable") {
                    for v in vars {
                        match u.variable.iter_mut().find(|k| k.key == v.key) {
                            Some(slot) => *slot = v,
                            None => u.variable.push(v),
                        }
                    }
                }
                u
            }
            _ => Url::default(),
        }
    }
}

impl Serialize for Url {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Value::Object(self.parts()).serialize(s)
    }
}
impl<'de> Deserialize<'de> for Url {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Url::from_value(Value::deserialize(d)?))
    }
}

// ------------------------------------------------------------------ Auth

/// Auth helper configuration. `kind` is the Postman auth type
/// (`noauth`, `basic`, `bearer`, `apikey`, `digest`, `oauth1`, `oauth2`, `awsv4`, `jwt`).
/// An absent auth (`None` on the owner) means "inherit from parent".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Auth {
    pub kind: String,
    pub params: IndexMap<String, String>,
}

impl Auth {
    pub fn new(kind: &str, params: &[(&str, &str)]) -> Self {
        Auth { kind: kind.into(), params: params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }
    pub fn get(&self, key: &str) -> &str {
        self.params.get(key).map(|s| s.as_str()).unwrap_or("")
    }
}

impl Serialize for Auth {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = Map::new();
        m.insert("type".into(), json!(self.kind));
        if !self.params.is_empty() {
            let arr: Vec<Value> =
                self.params.iter().map(|(k, v)| json!({"key": k, "value": v, "type": "string"})).collect();
            m.insert(self.kind.clone(), Value::Array(arr));
        }
        Value::Object(m).serialize(s)
    }
}
impl<'de> Deserialize<'de> for Auth {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("noauth").to_string();
        let mut params = IndexMap::new();
        match v.get(&kind) {
            Some(Value::Array(a)) => {
                for e in a {
                    if let Some(k) = e.get("key").and_then(|k| k.as_str()) {
                        params.insert(k.to_string(), value_to_string(e.get("value").unwrap_or(&Value::Null)));
                    }
                }
            }
            Some(Value::Object(o)) => {
                for (k, val) in o {
                    params.insert(k.clone(), value_to_string(val));
                }
            }
            _ => {}
        }
        Ok(Auth { kind, params })
    }
}

fn auth_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Auth>, D::Error> {
    let v = Value::deserialize(d)?;
    if v.is_null() {
        return Ok(None);
    }
    Ok(serde_json::from_value(v).ok())
}

// ------------------------------------------------------------------ Body

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GraphQl {
    #[serde(default, deserialize_with = "lenient_string")]
    pub query: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub variables: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Body {
    /// `raw`, `urlencoded`, `formdata`, `file`, `graphql` (or `none`).
    #[serde(default, deserialize_with = "lenient_string")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "kv_list")]
    pub urlencoded: Vec<KV>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "kv_list")]
    pub formdata: Vec<KV>,
    /// `{ "src": "/path/to/file" }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graphql: Option<GraphQl>,
    /// `{ "raw": { "language": "json" } }`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
}

impl Body {
    pub fn raw(text: &str, language: &str) -> Self {
        Body {
            mode: "raw".into(),
            raw: Some(text.into()),
            options: Some(json!({"raw": {"language": language}})),
            ..Default::default()
        }
    }
    pub fn language(&self) -> &str {
        self.options
            .as_ref()
            .and_then(|o| o.get("raw"))
            .and_then(|r| r.get("language"))
            .and_then(|l| l.as_str())
            .unwrap_or("text")
    }
    pub fn file_path(&self) -> Option<String> {
        self.file.as_ref().and_then(|f| f.get("src")).map(value_to_string).filter(|s| !s.is_empty())
    }
}

// ------------------------------------------------------------------ Request / Item

fn default_method() -> String {
    "GET".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    #[serde(default = "default_method", deserialize_with = "lenient_string")]
    pub method: String,
    #[serde(default, deserialize_with = "kv_list")]
    pub header: Vec<KV>,
    #[serde(default)]
    pub url: Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Body>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "auth_opt")]
    pub auth: Option<Auth>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "description")]
    pub description: Option<String>,
}

impl Default for Request {
    fn default() -> Self {
        Request { method: "GET".into(), header: vec![], url: Url::default(), body: None, auth: None, description: None }
    }
}

impl Request {
    pub fn new(method: &str, url: &str) -> Self {
        Request { method: method.to_uppercase(), url: Url::parse(url), ..Default::default() }
    }
}

fn request_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Request>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Null => None,
        Value::String(s) => Some(Request::new("GET", &s)),
        other => Some(serde_json::from_value(other).map_err(serde::de::Error::custom)?),
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Script {
    #[serde(default = "script_type", rename = "type")]
    pub kind: String,
    #[serde(default, deserialize_with = "string_or_lines")]
    pub exec: Vec<String>,
}
fn script_type() -> String {
    "text/javascript".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Event {
    /// `prerequest` or `test`
    pub listen: String,
    #[serde(default)]
    pub script: Script,
}

/// Saved example response.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Example {
    #[serde(default = "new_id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "originalRequest", skip_serializing_if = "Option::is_none", deserialize_with = "request_opt")]
    pub original_request: Option<Request>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub status: String,
    #[serde(default)]
    pub code: u16,
    #[serde(default, deserialize_with = "kv_list")]
    pub header: Vec<KV>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub body: String,
    #[serde(default, rename = "_postman_previewlanguage", skip_serializing_if = "Option::is_none")]
    pub preview_language: Option<String>,
}

/// A folder (has `item`) or a request (has `request`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Item {
    #[serde(default = "new_id", alias = "_postman_id")]
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "description")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<Item>>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "request_opt")]
    pub request: Option<Request>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub response: Vec<Example>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event: Vec<Event>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "auth_opt")]
    pub auth: Option<Auth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "kv_list")]
    pub variable: Vec<KV>,
}

impl Item {
    pub fn folder(name: &str) -> Self {
        Item { id: new_id(), name: name.into(), item: Some(vec![]), ..Default::default() }
    }
    pub fn request(name: &str, request: Request) -> Self {
        Item { id: new_id(), name: name.into(), request: Some(request), ..Default::default() }
    }
    pub fn is_folder(&self) -> bool {
        self.item.is_some()
    }
    pub fn script(&self, listen: &str) -> String {
        script_of(&self.event, listen)
    }
    pub fn set_script(&mut self, listen: &str, code: &str) {
        set_script(&mut self.event, listen, code)
    }
}

pub fn script_of(events: &[Event], listen: &str) -> String {
    events.iter().filter(|e| e.listen == listen).map(|e| e.script.exec.join("\n")).collect::<Vec<_>>().join("\n")
}

pub fn set_script(events: &mut Vec<Event>, listen: &str, code: &str) {
    events.retain(|e| e.listen != listen);
    if !code.trim().is_empty() {
        events.push(Event {
            listen: listen.into(),
            script: Script { kind: script_type(), exec: code.lines().map(|l| l.to_string()).collect() },
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Info {
    #[serde(default = "new_id", rename = "_postman_id")]
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "description")]
    pub description: Option<String>,
    #[serde(default = "schema")]
    pub schema: String,
}
fn schema() -> String {
    SCHEMA_V21.into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Collection {
    pub info: Info,
    #[serde(default)]
    pub item: Vec<Item>,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "auth_opt")]
    pub auth: Option<Auth>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event: Vec<Event>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "kv_list")]
    pub variable: Vec<KV>,
}

fn find_in<'a>(items: &'a [Item], id: &str, trail: &mut Vec<&'a Item>) -> bool {
    for it in items {
        trail.push(it);
        if it.id == id {
            return true;
        }
        if let Some(children) = &it.item {
            if find_in(children, id, trail) {
                return true;
            }
        }
        trail.pop();
    }
    false
}

fn find_mut_in<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Item> {
    for it in items {
        if it.id == id {
            return Some(it);
        }
        if let Some(children) = it.item.as_mut() {
            if let Some(f) = find_mut_in(children, id) {
                return Some(f);
            }
        }
    }
    None
}

fn remove_in(items: &mut Vec<Item>, id: &str) -> Option<Item> {
    if let Some(pos) = items.iter().position(|i| i.id == id) {
        return Some(items.remove(pos));
    }
    for it in items.iter_mut() {
        if let Some(children) = it.item.as_mut() {
            if let Some(r) = remove_in(children, id) {
                return Some(r);
            }
        }
    }
    None
}

fn flatten<'a>(items: &'a [Item], prefix: &str, out: &mut Vec<(String, &'a Item)>, folders: bool) {
    for it in items {
        let path = if prefix.is_empty() { it.name.clone() } else { format!("{prefix}/{}", it.name) };
        match &it.item {
            Some(children) => {
                if folders {
                    out.push((path.clone(), it));
                }
                flatten(children, &path, out, folders)
            }
            None => out.push((path, it)),
        }
    }
}

fn ensure_ids(items: &mut [Item], seen: &mut std::collections::HashSet<String>) {
    for it in items {
        if it.id.is_empty() || !seen.insert(it.id.clone()) {
            it.id = new_id();
            seen.insert(it.id.clone());
        }
        for ex in it.response.iter_mut() {
            if ex.id.is_empty() {
                ex.id = new_id();
            }
        }
        if let Some(children) = it.item.as_mut() {
            ensure_ids(children, seen);
        }
    }
}

impl Collection {
    pub fn new(name: &str) -> Self {
        Collection {
            info: Info { id: new_id(), name: name.into(), description: None, schema: schema() },
            item: vec![],
            auth: None,
            event: vec![],
            variable: vec![],
        }
    }

    pub fn id(&self) -> &str {
        &self.info.id
    }

    /// Parse a Postman collection (v2.0 / v2.1) export.
    pub fn from_json(text: &str) -> anyhow::Result<Self> {
        let mut v: Value = serde_json::from_str(text)?;
        // Some exports wrap the collection: { "collection": { ... } }
        if v.get("info").is_none() {
            if let Some(inner) = v.get_mut("collection") {
                v = inner.take();
            }
        }
        if v.get("info").is_none() {
            anyhow::bail!("not a Postman collection (missing `info`)");
        }
        let mut c: Collection = serde_json::from_value(v)?;
        c.info.schema = schema();
        c.normalize();
        Ok(c)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Give every item a unique non-empty id.
    pub fn normalize(&mut self) {
        if self.info.id.is_empty() {
            self.info.id = new_id();
        }
        let mut seen = std::collections::HashSet::new();
        ensure_ids(&mut self.item, &mut seen);
    }

    /// Chain of items from the root down to (and including) the item with `id`.
    pub fn trail(&self, id: &str) -> Option<Vec<&Item>> {
        let mut t = vec![];
        find_in(&self.item, id, &mut t).then_some(t)
    }

    pub fn find(&self, id: &str) -> Option<&Item> {
        self.trail(id).and_then(|t| t.last().copied())
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Item> {
        find_mut_in(&mut self.item, id)
    }

    /// All requests (depth first, document order) as `(path, item)`.
    pub fn requests(&self) -> Vec<(String, &Item)> {
        let mut out = vec![];
        flatten(&self.item, "", &mut out, false);
        out
    }

    /// All items including folders as `(path, item)`.
    pub fn all_items(&self) -> Vec<(String, &Item)> {
        let mut out = vec![];
        flatten(&self.item, "", &mut out, true);
        out
    }

    /// Requests below a folder (by id).
    pub fn requests_in(&self, folder_id: &str) -> Vec<(String, &Item)> {
        let mut out = vec![];
        if let Some(f) = self.find(folder_id) {
            match &f.item {
                Some(children) => flatten(children, &f.name, &mut out, false),
                None => out.push((f.name.clone(), f)),
            }
        }
        out
    }

    /// Resolve an item by id, by `Folder/Sub/Name` path, or by unique name.
    pub fn resolve(&self, reference: &str) -> Option<&Item> {
        if let Some(i) = self.find(reference) {
            return Some(i);
        }
        let all = self.all_items();
        if let Some((_, i)) = all.iter().find(|(p, _)| p == reference) {
            return Some(i);
        }
        let lower = reference.to_lowercase();
        let by_name: Vec<_> = all.iter().filter(|(_, i)| i.name.to_lowercase() == lower).collect();
        if by_name.len() == 1 {
            return Some(by_name[0].1);
        }
        all.iter().find(|(p, _)| p.to_lowercase() == lower).map(|(_, i)| *i)
    }

    pub fn add_item(&mut self, parent: Option<&str>, item: Item) -> anyhow::Result<()> {
        match parent {
            None => self.item.push(item),
            Some(pid) => {
                let p = self.find_mut(pid).ok_or_else(|| anyhow::anyhow!("parent folder not found"))?;
                p.item.as_mut().ok_or_else(|| anyhow::anyhow!("parent is not a folder"))?.push(item);
            }
        }
        Ok(())
    }

    pub fn remove_item(&mut self, id: &str) -> Option<Item> {
        remove_in(&mut self.item, id)
    }

    /// Move an item into `parent` (None = root) at `index` (None = end).
    pub fn move_item(&mut self, id: &str, parent: Option<&str>, index: Option<usize>) -> anyhow::Result<()> {
        if let Some(pid) = parent {
            if pid == id || self.trail(pid).map(|t| t.iter().any(|i| i.id == id)).unwrap_or(false) {
                anyhow::bail!("cannot move a folder into itself");
            }
            if !self.find(pid).map(|p| p.is_folder()).unwrap_or(false) {
                anyhow::bail!("target is not a folder");
            }
        }
        let item = self.remove_item(id).ok_or_else(|| anyhow::anyhow!("item not found"))?;
        let list = match parent {
            None => &mut self.item,
            Some(pid) => self.find_mut(pid).and_then(|p| p.item.as_mut()).ok_or_else(|| anyhow::anyhow!("target vanished"))?,
        };
        let idx = index.unwrap_or(list.len()).min(list.len());
        list.insert(idx, item);
        Ok(())
    }

    /// Auth after applying inheritance (request -> folders -> collection).
    pub fn effective_auth(&self, id: &str) -> Option<Auth> {
        let trail = self.trail(id)?;
        for it in trail.iter().rev() {
            let a = it.request.as_ref().and_then(|r| r.auth.as_ref()).or(it.auth.as_ref());
            if let Some(a) = a {
                return Some(a.clone());
            }
        }
        self.auth.clone()
    }

    pub fn variables(&self) -> VarMap {
        self.variable.iter().filter(|k| !k.disabled).map(|k| (k.key.clone(), k.value.clone())).collect()
    }

    pub fn set_variables(&mut self, vars: &VarMap) {
        self.variable.retain(|k| k.disabled || vars.contains_key(&k.key));
        for (k, v) in vars {
            match self.variable.iter_mut().find(|x| !x.disabled && &x.key == k) {
                Some(slot) => slot.value = v.clone(),
                None => self.variable.push(KV::new(k.clone(), v.clone())),
            }
        }
    }
}

// ------------------------------------------------------------------ Environment

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EnvVar {
    #[serde(default, deserialize_with = "lenient_string")]
    pub key: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub value: String,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Postman environment / globals file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Environment {
    #[serde(default = "new_id")]
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(default)]
    pub values: Vec<EnvVar>,
    #[serde(default, rename = "_postman_variable_scope", skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl Environment {
    pub fn new(name: &str) -> Self {
        Environment { id: new_id(), name: name.into(), values: vec![], scope: Some("environment".into()) }
    }
    pub fn from_json(text: &str) -> anyhow::Result<Self> {
        let v: Value = serde_json::from_str(text)?;
        if !v.get("values").map(|x| x.is_array()).unwrap_or(false) {
            anyhow::bail!("not a Postman environment (missing `values`)");
        }
        let mut e: Environment = serde_json::from_value(v)?;
        if e.id.is_empty() {
            e.id = new_id();
        }
        Ok(e)
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn map(&self) -> VarMap {
        self.values.iter().filter(|v| v.enabled).map(|v| (v.key.clone(), v.value.clone())).collect()
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.iter().find(|v| v.enabled && v.key == key).map(|v| v.value.as_str())
    }
    pub fn set(&mut self, key: &str, value: &str) {
        match self.values.iter_mut().find(|v| v.key == key) {
            Some(v) => {
                v.value = value.into();
                v.enabled = true;
            }
            None => self.values.push(EnvVar { key: key.into(), value: value.into(), enabled: true, kind: None }),
        }
    }
    pub fn unset(&mut self, key: &str) -> bool {
        let n = self.values.len();
        self.values.retain(|v| v.key != key);
        n != self.values.len()
    }
    /// Write a variable map back, keeping disabled rows and ordering.
    pub fn apply_map(&mut self, vars: &VarMap) {
        self.values.retain(|v| !v.enabled || vars.contains_key(&v.key));
        for (k, v) in vars {
            match self.values.iter_mut().find(|x| x.enabled && &x.key == k) {
                Some(slot) => slot.value = v.clone(),
                None => self.values.push(EnvVar { key: k.clone(), value: v.clone(), enabled: true, kind: None }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "info": {"_postman_id": "c1", "name": "Sample", "description": {"content": "desc", "type": "text/plain"},
               "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
      "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{tok}}", "type": "string"}]},
      "variable": [{"key": "base", "value": "http://localhost:1"}, {"key": "n", "value": 5}],
      "item": [
        {"name": "Users", "item": [
           {"name": "Get user",
            "event": [{"listen": "test", "script": {"exec": ["pm.test('ok', () => {});"], "type": "text/javascript"}}],
            "request": {"method": "GET", "header": [{"key": "Accept", "value": "application/json"}],
              "url": {"raw": "{{base}}/users/:id?verbose=true", "host": ["{{base}}"], "path": ["users", ":id"],
                      "query": [{"key": "verbose", "value": "true"}, {"key": "off", "value": "1", "disabled": true}],
                      "variable": [{"key": "id", "value": "42"}]}},
            "response": [{"name": "ok", "status": "OK", "code": 200, "header": [{"key":"Content-Type","value":"application/json"}], "body": "{\"id\":42}"}]},
           {"name": "Basic one", "request": {"method": "POST", "url": "http://x.test/a?b=1",
              "auth": {"type": "basic", "basic": {"username": "u", "password": "p"}},
              "header": "X-A: 1\nX-B: 2",
              "body": {"mode": "raw", "raw": "{}", "options": {"raw": {"language": "json"}}}}}
        ]},
        {"name": "Plain", "request": "http://x.test/plain"}
      ]
    }"#;

    #[test]
    fn parses_lenient_collection() {
        let c = Collection::from_json(SAMPLE).unwrap();
        assert_eq!(c.info.description.as_deref(), Some("desc"));
        assert_eq!(c.variables().get("n").unwrap(), "5");
        let reqs = c.requests();
        assert_eq!(reqs.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["Users/Get user", "Users/Basic one", "Plain"]);
        let get = reqs[0].1.request.as_ref().unwrap();
        assert_eq!(get.url.query.len(), 2);
        assert!(get.url.query[1].disabled);
        assert_eq!(get.url.variable[0].value, "42");
        assert_eq!(reqs[0].1.script("test"), "pm.test('ok', () => {});");
        let basic = reqs[1].1.request.as_ref().unwrap();
        assert_eq!(basic.header.len(), 2);
        assert_eq!(basic.auth.as_ref().unwrap().get("username"), "u");
        assert_eq!(basic.url.query[0].key, "b");
        assert_eq!(reqs[2].1.request.as_ref().unwrap().url.raw, "http://x.test/plain");
    }

    #[test]
    fn round_trips() {
        let c = Collection::from_json(SAMPLE).unwrap();
        let again = Collection::from_json(&c.to_json()).unwrap();
        assert_eq!(c, again);
        let v: Value = serde_json::from_str(&c.to_json()).unwrap();
        assert_eq!(v["item"][0]["item"][0]["request"]["url"]["path"], json!(["users", ":id"]));
        assert_eq!(v["auth"]["bearer"][0]["key"], "token");
    }

    #[test]
    fn auth_inheritance_and_tree_ops() {
        let mut c = Collection::from_json(SAMPLE).unwrap();
        let ids: Vec<String> = c.requests().iter().map(|(_, i)| i.id.clone()).collect();
        assert_eq!(c.effective_auth(&ids[0]).unwrap().kind, "bearer");
        assert_eq!(c.effective_auth(&ids[1]).unwrap().kind, "basic");
        let folder = c.resolve("Users").unwrap().id.clone();
        c.find_mut(&folder).unwrap().auth = Some(Auth::new("noauth", &[]));
        assert_eq!(c.effective_auth(&ids[0]).unwrap().kind, "noauth");
        assert_eq!(c.effective_auth(&ids[2]).unwrap().kind, "bearer");

        c.move_item(&ids[2], Some(&folder), Some(0)).unwrap();
        assert_eq!(c.requests()[0].0, "Users/Plain");
        assert!(c.move_item(&folder, Some(&folder), None).is_err());
        assert!(c.remove_item(&ids[0]).is_some());
        assert_eq!(c.requests().len(), 2);
        assert_eq!(c.resolve("users/plain").unwrap().id, ids[2]);
    }

    #[test]
    fn url_sync() {
        let mut u = Url::parse("http://h/a/:x/b?p=1&q=2#frag");
        assert_eq!(u.base(), "http://h/a/:x/b");
        assert_eq!(u.variable[0].key, "x");
        u.query[0].disabled = true;
        u.sync_raw_from_query();
        assert_eq!(u.raw, "http://h/a/:x/b?q=2");
    }

    #[test]
    fn environment_roundtrip() {
        let e = Environment::from_json(r#"{"name":"dev","values":[{"key":"a","value":"1","enabled":true},{"key":"b","value":2,"enabled":false}]}"#).unwrap();
        assert_eq!(e.map().len(), 1);
        let mut e2 = e.clone();
        let mut m = e2.map();
        m.insert("c".into(), "3".into());
        m.shift_remove("a");
        e2.apply_map(&m);
        assert_eq!(e2.values.iter().map(|v| v.key.as_str()).collect::<Vec<_>>(), ["b", "c"]);
    }
}
