//! Additional import formats (OpenAPI, HAR). Filled in sprint 2.

use crate::import::Imported;
use anyhow::Result;

pub fn parse_json(_v: &serde_json::Value) -> Result<Option<Imported>> {
    Ok(None)
}

pub fn parse_text(_t: &str) -> Result<Option<Imported>> {
    Ok(None)
}
