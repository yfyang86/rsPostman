//! Collection runner (the local equivalent of the Postman Collection Runner / Newman).

use crate::engine::{Exchange, Session};
use crate::model::{Collection, VarMap};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RunOptions {
    /// Run only this folder (or single request), by item id.
    pub folder: Option<String>,
    /// Explicit selection and order of request ids (overrides `folder`).
    pub item_ids: Option<Vec<String>>,
    /// Number of iterations; defaults to the number of data rows (or 1).
    pub iterations: Option<usize>,
    /// Iteration data, one variable map per iteration.
    pub data: Vec<VarMap>,
    /// Delay between requests in milliseconds.
    pub delay_ms: u64,
    /// Stop at the first failing request.
    pub bail: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub iteration: usize,
    /// `Folder/Request` path.
    pub path: String,
    pub exchange: Exchange,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RunStats {
    pub iterations: usize,
    pub requests: usize,
    pub requests_failed: usize,
    pub requests_skipped: usize,
    pub tests: usize,
    pub tests_failed: usize,
    pub tests_skipped: usize,
    pub response_time_ms: u64,
    pub response_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub collection_id: String,
    pub collection_name: String,
    pub environment: Option<String>,
    pub started: DateTime<Utc>,
    pub duration_ms: u64,
    pub executions: Vec<Execution>,
    pub stats: RunStats,
    /// Why the run ended early (bail, flow control error), if it did.
    pub aborted: Option<String>,
}

impl RunSummary {
    pub fn success(&self) -> bool {
        self.stats.requests_failed == 0 && self.stats.tests_failed == 0 && !matches!(&self.aborted, Some(a) if !a.starts_with("stopped by"))
    }
}

pub enum RunEvent<'a> {
    Start { requests: usize, iterations: usize },
    Iteration { index: usize, total: usize },
    Request(&'a Execution),
}

/// Parse CSV text (RFC 4180: quoted fields, doubled quotes, embedded newlines).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut it = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut any = false;
    while let Some(c) = it.next() {
        any = true;
        if quoted {
            match c {
                '"' if it.peek() == Some(&'"') => {
                    it.next();
                    field.push('"');
                }
                '"' => quoted = false,
                c => field.push(c),
            }
        } else {
            match c {
                '"' if field.is_empty() => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                    any = false;
                }
                c => field.push(c),
            }
        }
    }
    if any || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    rows
}

/// Parse iteration data: a JSON array of objects or CSV with a header row.
pub fn parse_data(text: &str) -> Result<Vec<VarMap>> {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with('[') || t.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(t).context("data file is not valid JSON")?;
        let arr = v.as_array().ok_or_else(|| anyhow!("JSON data file must be an array of objects"))?;
        let mut out = vec![];
        for (i, row) in arr.iter().enumerate() {
            let obj = row.as_object().ok_or_else(|| anyhow!("data row {} is not an object", i + 1))?;
            out.push(obj.iter().map(|(k, v)| (k.clone(), crate::model::value_to_string(v))).collect());
        }
        return Ok(out);
    }
    let rows = parse_csv(text);
    let Some((header, body)) = rows.split_first() else { bail!("data file is empty") };
    Ok(body.iter().map(|r| header.iter().enumerate().map(|(i, h)| (h.trim().to_string(), r.get(i).cloned().unwrap_or_default())).collect()).collect())
}

pub fn load_data(path: &str) -> Result<Vec<VarMap>> {
    parse_data(&std::fs::read_to_string(path).with_context(|| format!("cannot read data file {path}"))?)
}

/// Run a collection. `on_event` is called as the run progresses; returning `false` cancels the run.
pub fn run(collection: &Collection, session: &mut Session, opts: &RunOptions, on_event: &mut dyn FnMut(RunEvent) -> bool) -> Result<RunSummary> {
    let all = collection.requests();
    let list: Vec<(String, String, String)> = match (&opts.item_ids, &opts.folder) {
        (Some(ids), _) => {
            let mut v = vec![];
            for id in ids {
                let (path, item) = all.iter().find(|(_, i)| &i.id == id).ok_or_else(|| anyhow!("request `{id}` not found in collection"))?;
                v.push((path.clone(), item.id.clone(), item.name.clone()));
            }
            v
        }
        (None, Some(folder)) => {
            if collection.find(folder).is_none() {
                bail!("folder `{folder}` not found in collection");
            }
            collection.requests_in(folder).into_iter().map(|(p, i)| (p, i.id.clone(), i.name.clone())).collect()
        }
        (None, None) => all.iter().map(|(p, i)| (p.clone(), i.id.clone(), i.name.clone())).collect(),
    };
    let iterations = opts.iterations.unwrap_or(opts.data.len().max(1)).max(1);
    let started = Utc::now();
    let clock = std::time::Instant::now();
    let mut summary = RunSummary {
        id: crate::model::new_id(),
        collection_id: collection.id().to_string(),
        collection_name: collection.info.name.clone(),
        environment: session.environment.as_ref().map(|e| e.name.clone()),
        started,
        duration_ms: 0,
        executions: vec![],
        stats: RunStats::default(),
        aborted: None,
    };
    session.iteration_count = iterations;
    if !on_event(RunEvent::Start { requests: list.len(), iterations }) {
        summary.aborted = Some("cancelled".into());
    }

    'outer: for iteration in 0..iterations {
        if summary.aborted.is_some() {
            break;
        }
        session.iteration = iteration;
        session.scopes.data = if opts.data.is_empty() { VarMap::new() } else { opts.data[iteration.min(opts.data.len() - 1)].clone() };
        summary.stats.iterations += 1;
        if !on_event(RunEvent::Iteration { index: iteration, total: iterations }) {
            summary.aborted = Some("cancelled".into());
            break;
        }
        let mut index = 0;
        let mut steps = 0;
        while index < list.len() {
            steps += 1;
            if steps > 10_000 {
                summary.aborted = Some("more than 10000 requests in one iteration (setNextRequest loop?)".into());
                break 'outer;
            }
            let (path, id, _) = &list[index];
            if steps > 1 && opts.delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(opts.delay_ms));
            }
            let exchange = session.send_item(collection, id);
            let st = &mut summary.stats;
            st.requests += 1;
            if exchange.skipped {
                st.requests_skipped += 1;
            }
            if exchange.failed() {
                st.requests_failed += 1;
            }
            st.tests += exchange.tests.len();
            st.tests_failed += exchange.failed_tests();
            st.tests_skipped += exchange.tests.iter().filter(|t| t.skipped).count();
            if let Some(r) = &exchange.response {
                st.response_time_ms += r.time_ms;
                st.response_bytes += r.size;
            }
            let failed = exchange.failed();
            let next = exchange.next.clone();
            summary.executions.push(Execution { iteration, path: path.clone(), exchange });
            if !on_event(RunEvent::Request(summary.executions.last().expect("just pushed"))) {
                summary.aborted = Some("cancelled".into());
                break 'outer;
            }
            if failed && opts.bail {
                summary.aborted = Some(format!("bailed after failure in `{path}`"));
                break 'outer;
            }
            match next {
                None => index += 1,
                Some(n) => match n.name {
                    None => {
                        summary.aborted = Some("stopped by setNextRequest(null)".into());
                        break 'outer;
                    }
                    Some(name) => match list.iter().position(|(_, id, n)| n == &name || id == &name) {
                        Some(pos) => index = pos,
                        None => {
                            summary.aborted = Some(format!("setNextRequest: no request named `{name}` in this run"));
                            break 'outer;
                        }
                    },
                },
            }
        }
    }
    session.scopes.data = VarMap::new();
    summary.duration_ms = clock.elapsed().as_millis() as u64;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parser() {
        let rows = parse_csv("name,note\r\nann,\"hello, \"\"world\"\"\"\nbob,\"two\nlines\"\n\n");
        assert_eq!(rows, vec![vec!["name", "note"], vec!["ann", "hello, \"world\""], vec!["bob", "two\nlines"]]);
        let data = parse_data("a,b\n1,2\n3\n").unwrap();
        assert_eq!(data[1]["a"], "3");
        assert_eq!(data[1]["b"], "");
        let data = parse_data(r#"[{"a": 1, "b": {"x": true}}, {"a": "two"}]"#).unwrap();
        assert_eq!((data[0]["a"].as_str(), data[0]["b"].as_str(), data[1]["a"].as_str()), ("1", r#"{"x":true}"#, "two"));
        assert!(parse_data("[1]").is_err());
        assert!(parse_data("").is_err());
    }
}
