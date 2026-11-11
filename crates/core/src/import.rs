//! Import with format auto-detection.

use crate::model::{Collection, Environment, Request};
use anyhow::{bail, Result};

#[derive(Debug, Clone)]
pub enum Imported {
    Collection(Collection),
    Environment(Environment),
    Globals(Environment),
    /// A single request (curl command).
    Request(Request),
}

impl Imported {
    pub fn kind(&self) -> &'static str {
        match self {
            Imported::Collection(_) => "collection",
            Imported::Environment(_) => "environment",
            Imported::Globals(_) => "globals",
            Imported::Request(_) => "request",
        }
    }
}

/// Detect the format of `text` and parse it.
/// Supported: Postman collection v2.0/v2.1, Postman environment / globals, curl command.
pub fn parse(text: &str) -> Result<Imported> {
    let t = text.trim_start_matches('\u{feff}').trim();
    if t.starts_with("curl ") || t.starts_with("curl.exe ") {
        return Ok(Imported::Request(crate::curl::parse(t)?));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
        if v.get("info").is_some() || v.get("collection").and_then(|c| c.get("info")).is_some() {
            return Ok(Imported::Collection(Collection::from_json(t)?));
        }
        if v.get("values").map(|x| x.is_array()).unwrap_or(false) {
            let env = Environment::from_json(t)?;
            return Ok(if env.scope.as_deref() == Some("globals") { Imported::Globals(env) } else { Imported::Environment(env) });
        }
        if let Some(i) = crate::import_ext::parse_json(&v)? {
            return Ok(i);
        }
        bail!("unrecognised JSON document (expected a Postman collection, environment, OpenAPI/Swagger or HAR file)");
    }
    if let Some(i) = crate::import_ext::parse_text(t)? {
        return Ok(i);
    }
    bail!("unrecognised import format (expected Postman collection/environment JSON, OpenAPI, HAR or a curl command)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_formats() {
        assert_eq!(parse("curl http://x").unwrap().kind(), "request");
        assert_eq!(parse(r#"{"info":{"name":"n"},"item":[]}"#).unwrap().kind(), "collection");
        assert_eq!(parse(r#"{"name":"e","values":[]}"#).unwrap().kind(), "environment");
        assert_eq!(parse(r#"{"name":"g","values":[],"_postman_variable_scope":"globals"}"#).unwrap().kind(), "globals");
        assert!(parse("{}").is_err());
        assert!(parse("hello").is_err());
    }
}
