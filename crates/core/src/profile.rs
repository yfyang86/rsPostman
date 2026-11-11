//! Performance tooling: LLM-provider streaming profiles and request benchmarks.
//!
//! * [`profile_llm`] evaluates the health of an OpenAI-compatible streaming endpoint:
//!   time to first token, output rate, stream completeness and token usage, repeated
//!   over several runs and aggregated with percentiles.
//! * [`benchmark`] repeats one request (or every request of a collection) for a number
//!   of iterations and reports latency percentiles (P10/P50/P90/P95/P99), throughput
//!   and error counts.
//!
//! Both reuse one HTTP client for the whole run, so – like most load tools – only the first
//! request of each connection pays for the TCP/TLS handshake. Pre-request and test scripts
//! are not executed, and URLs in reports have their credentials redacted.

use crate::http::{self, Prepared, Settings};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Percentile of already-sorted samples with linear interpolation (numpy-style).
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (p / 100.0) * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64)
    }
}

/// Latency summary of a set of samples (milliseconds unless noted otherwise).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LatencyStats {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub stddev: f64,
    pub p10: f64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
}

impl LatencyStats {
    pub fn of(samples: &[f64]) -> Option<LatencyStats> {
        if samples.is_empty() {
            return None;
        }
        let mut v = samples.to_vec();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = v.len() as f64;
        let mean = v.iter().sum::<f64>() / n;
        let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
        Some(LatencyStats {
            count: v.len(),
            min: v[0],
            max: v[v.len() - 1],
            mean,
            stddev: var.sqrt(),
            p10: percentile(&v, 10.0),
            p50: percentile(&v, 50.0),
            p90: percentile(&v, 90.0),
            p95: percentile(&v, 95.0),
            p99: percentile(&v, 99.0),
        })
    }
}

// ---------------------------------------------------------------- LLM profile

/// Token usage reported by an OpenAI-compatible API (absent unless the provider sends it).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}

/// One streaming run against the provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileSample {
    pub status: u16,
    pub status_text: String,
    /// Time from the request start to the first stream event, in ms.
    pub ttfb_ms: u64,
    /// Total time until the stream finished, in ms.
    pub total_ms: u64,
    /// Number of stream events (SSE `data:` messages).
    pub events: usize,
    /// Response bytes read (including framing).
    pub bytes: usize,
    /// Output rate: completion tokens per second when usage is reported, otherwise events per second.
    pub rate_per_s: f64,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    /// Concatenated generated text (truncated for display).
    pub completion: String,
    /// The stream ended with the SSE `data: [DONE]` sentinel.
    pub done: bool,
    /// The response was a `text/event-stream` (a provider that ignores `stream: true` answers with plain JSON).
    #[serde(default)]
    pub streamed: bool,
    pub error: Option<String>,
}

/// Aggregate report over every profiling run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileReport {
    pub url: String,
    pub iterations: usize,
    /// `healthy`, `degraded` or `unhealthy`.
    pub verdict: String,
    pub notes: Vec<String>,
    pub samples: Vec<ProfileSample>,
    pub ttfb: Option<LatencyStats>,
    pub total: Option<LatencyStats>,
    pub rate: Option<LatencyStats>,
}

impl ProfileReport {
    pub fn healthy(&self) -> bool {
        self.verdict == "healthy"
    }
}

/// Extract the generated text of one SSE data payload, e.g. `choices[0].delta.content`.
fn completion_delta(payload: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    v["choices"][0]["delta"]["content"].as_str().map(|s| s.to_string())
}

/// Run one streaming request and measure it. Returns the sample (with `error` set on failure).
pub fn profile_once(client: &reqwest::blocking::Client, p: &Prepared, max_capture: usize) -> ProfileSample {
    let started = Instant::now();
    let run = || -> Result<ProfileSample> {
        let mut s = ProfileSample::default();
        let method = reqwest::Method::from_bytes(p.method.as_bytes())?;
        let mut rb = client.request(method, p.url.clone());
        for (k, v) in &p.headers {
            rb = rb.header(k, v);
        }
        if let Some(b) = p.body.bytes()? {
            rb = rb.body(b);
        }
        let resp = rb.send().map_err(http::explain)?;
        s.status = resp.status().as_u16();
        s.status_text = resp.status().canonical_reason().unwrap_or("").to_string();
        if !resp.status().is_success() {
            let mut body = resp.bytes().unwrap_or_default().to_vec();
            body.truncate(2048);
            let text = String::from_utf8_lossy(&body);
            anyhow::bail!("HTTP {} {}{}", s.status, s.status_text, if text.is_empty() { String::new() } else { format!(": {}", text.trim()) });
        }
        let declared_sse = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(|v| v.to_lowercase().starts_with("text/event-stream")).unwrap_or(false);
        let mut reader = BufReader::new(resp);
        let mut payload = String::new();
        let mut line = String::new();
        let mut arrivals: Vec<u64> = vec![];
        let mut completion = String::new();
        let mut saw_done = false;
        let mut is_sse = declared_sse;
        let handle = |payload: &str, s: &mut ProfileSample, arrivals: &mut Vec<u64>, completion: &mut String, saw_done: &mut bool| {
            if payload == "[DONE]" {
                *saw_done = true;
                return;
            }
            arrivals.push(started.elapsed().as_millis() as u64);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
                if s.model.is_none() {
                    s.model = v["model"].as_str().map(|m| m.to_string());
                }
                if let Some(u) = v.get("usage") {
                    s.usage = serde_json::from_value(u.clone()).ok();
                }
                if let Some(d) = completion_delta(payload) {
                    if completion.len() < max_capture {
                        completion.push_str(&d.chars().take(max_capture - completion.len()).collect::<String>());
                    }
                }
            } else if completion.len() < max_capture {
                completion.push_str(&payload.chars().take(max_capture - completion.len()).collect::<String>());
            }
        };
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            s.bytes += n;
            let t = line.trim_end();
            if t.is_empty() {
                if !payload.is_empty() {
                    handle(&payload, &mut s, &mut arrivals, &mut completion, &mut saw_done);
                    payload.clear();
                }
                continue;
            }
            if let Some(d) = t.strip_prefix("data:") {
                is_sse = true;
                if !payload.is_empty() {
                    payload.push('\n');
                }
                payload.push_str(d.trim_start());
            }
            // comments (: keep-alive) and other SSE fields (event:, id:, retry:) are ignored
        }
        if !payload.is_empty() {
            handle(&payload, &mut s, &mut arrivals, &mut completion, &mut saw_done);
        }
        s.events = arrivals.len();
        s.streamed = is_sse;
        s.done = if is_sse { saw_done } else { true };
        s.ttfb_ms = arrivals.first().copied().unwrap_or(0);
        s.total_ms = started.elapsed().as_millis() as u64;
        s.completion = completion;
        let units = s.usage.as_ref().filter(|u| u.completion_tokens > 0).map(|u| u.completion_tokens).unwrap_or(arrivals.len().saturating_sub(1) as u64);
        let span_ms = if arrivals.len() >= 2 { arrivals[arrivals.len() - 1] - arrivals[0] } else { 0 };
        let window_ms = if span_ms > 0 { span_ms } else { s.total_ms };
        if units > 0 && window_ms > 0 {
            s.rate_per_s = units as f64 / (window_ms as f64 / 1000.0);
        }
        Ok(s)
    };
    match run() {
        Ok(sample) => sample,
        Err(e) => ProfileSample { error: Some(format!("{e:#}")), total_ms: started.elapsed().as_millis() as u64, ..Default::default() },
    }
}

/// Profile a streaming LLM endpoint `iterations` times and aggregate the results.
pub fn profile_llm(p: &Prepared, st: &Settings, iterations: usize, delay_ms: u64, cancel: Option<&AtomicBool>) -> ProfileReport {
    let iterations = iterations.max(1);
    let mut report = ProfileReport { url: crate::secrets::redact_url(&p.url), iterations, ..Default::default() };
    let mut notes = vec![];
    let client = match http::build_client(st) {
        Ok(c) => c,
        Err(e) => {
            report.verdict = "unhealthy".into();
            report.notes = vec![format!("cannot create the HTTP client: {e:#}")];
            return report;
        }
    };
    for i in 0..iterations {
        if cancel.map(|c| c.load(Ordering::SeqCst)).unwrap_or(false) {
            notes.push(format!("stopped after run {}/{}", i, iterations));
            break;
        }
        if i > 0 && delay_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
        let mut sample = profile_once(&client, p, 4096);
        sample.completion = sample.completion.trim().to_string();
        if let Some(e) = &sample.error {
            notes.push(format!("run {}: {e}", i + 1));
        } else {
            if !sample.streamed {
                notes.push(format!("run {}: the response was not an event stream (does the endpoint support `stream: true`?)", i + 1));
            } else if sample.events == 0 {
                notes.push(format!("run {}: empty stream (no events)", i + 1));
            }
            if !sample.done {
                notes.push(format!("run {}: stream ended without the [DONE] sentinel", i + 1));
            }
        }
        report.samples.push(sample);
    }
    let ok: Vec<&ProfileSample> = report.samples.iter().filter(|s| s.error.is_none()).collect();
    let errors = report.samples.len() - ok.len();
    let verdict = if ok.is_empty() {
        "unhealthy"
    } else if errors > 0 || ok.iter().any(|s| s.events == 0 || !s.done) {
        "degraded"
    } else {
        "healthy"
    };
    report.verdict = verdict.to_string();
    report.notes = notes;
    report.ttfb = LatencyStats::of(&ok.iter().map(|s| s.ttfb_ms as f64).collect::<Vec<_>>());
    report.total = LatencyStats::of(&ok.iter().map(|s| s.total_ms as f64).collect::<Vec<_>>());
    report.rate = LatencyStats::of(&ok.iter().map(|s| s.rate_per_s).collect::<Vec<_>>());
    report
}

// ---------------------------------------------------------------- benchmark

/// A prepared request with the name shown in reports.
#[derive(Debug, Clone)]
pub struct BenchTask {
    pub name: String,
    pub prepared: Prepared,
}

/// One timed request of a benchmark.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchSample {
    pub iteration: usize,
    pub name: String,
    pub time_ms: u64,
    pub status: u16,
    pub size: usize,
    /// Transport-level failure (connection refused, timeout, TLS …).
    pub error: Option<String>,
}

/// Upper bound for benchmark worker threads.
pub const MAX_CONCURRENCY: usize = 256;

/// Latency stats of the answered requests of one named task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NameStats {
    pub name: String,
    pub requests: usize,
    pub errors: usize,
    pub stats: Option<LatencyStats>,
}

/// Aggregated result of a benchmark run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchReport {
    pub started: Option<DateTime<Utc>>,
    pub duration_ms: u64,
    /// Configured iterations (per task).
    pub iterations: usize,
    pub total_requests: usize,
    /// Requests that got no response at all.
    pub errors: usize,
    /// Responses with a status of 400 or above (they are still timed).
    #[serde(default)]
    pub http_errors: usize,
    pub bytes: usize,
    pub aborted: Option<String>,
    /// Latency stats across every answered request.
    pub overall: Option<LatencyStats>,
    pub by_name: Vec<NameStats>,
    pub samples: Vec<BenchSample>,
}

impl BenchReport {
    /// Answered requests per second of wall-clock time.
    pub fn rps(&self) -> f64 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        (self.total_requests - self.errors) as f64 / (self.duration_ms as f64 / 1000.0)
    }
}

/// Run `iterations` rounds of every task and collect timing samples.
///
/// With `concurrency` > 1 the work items (iteration × task) are pulled from a shared
/// counter by that many worker threads; the delay then applies between the requests of
/// each worker. `cancel` stops the run between requests.
pub fn benchmark(
    tasks: &[BenchTask],
    st: &Settings,
    iterations: usize,
    delay_ms: u64,
    concurrency: usize,
    cancel: Option<&AtomicBool>,
    on_sample: &(dyn Fn(&BenchSample) + Sync),
) -> BenchReport {
    let started = Utc::now();
    let clock = Instant::now();
    let iterations = iterations.max(1);
    let task_count = tasks.len();
    let total = task_count * iterations;
    let samples: Arc<Mutex<Vec<BenchSample>>> = Arc::new(Mutex::new(vec![]));
    let next = Arc::new(AtomicUsize::new(0));
    let workers = concurrency.clamp(1, MAX_CONCURRENCY).min(total.max(1));
    let aborted = Arc::new(Mutex::new(None::<String>));
    // a poisoned lock only means another worker panicked; the data is still usable
    fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }
    let client = match http::build_client(st) {
        Ok(c) => c,
        Err(e) => {
            return BenchReport { started: Some(started), iterations, aborted: Some(format!("cannot create the HTTP client: {e:#}")), ..Default::default() };
        }
    };

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (samples, next, aborted, client) = (samples.clone(), next.clone(), aborted.clone(), client.clone());
            scope.spawn(move || {
                let mut first = true;
                loop {
                    if cancel.map(|c| c.load(Ordering::SeqCst)).unwrap_or(false) {
                        *locked(&aborted) = Some("cancelled".into());
                        break;
                    }
                    let item = next.fetch_add(1, Ordering::SeqCst);
                    if item >= total {
                        break;
                    }
                    if !first && delay_ms > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    }
                    first = false;
                    let (iteration, task) = (item / task_count, &tasks[item % task_count]);
                    let sample = match execute_one(&client, task, st) {
                        Ok((time_ms, status, size)) => BenchSample { iteration, name: task.name.clone(), time_ms, status, size, error: None },
                        Err(e) => BenchSample { iteration, name: task.name.clone(), time_ms: 0, status: 0, size: 0, error: Some(format!("{e:#}")) },
                    };
                    on_sample(&sample);
                    locked(&samples).push(sample);
                }
            });
        }
    });

    let mut samples = std::mem::take(&mut *locked(&samples));
    samples.sort_by_key(|s| (s.iteration, s.name.clone()));
    let errors = samples.iter().filter(|s| s.error.is_some()).count();
    let http_errors = samples.iter().filter(|s| s.error.is_none() && s.status >= 400).count();
    let bytes = samples.iter().map(|s| s.size).sum();
    let ok: Vec<f64> = samples.iter().filter(|s| s.error.is_none()).map(|s| s.time_ms as f64).collect();
    let mut by_name: Vec<NameStats> = vec![];
    for task in tasks {
        let mine: Vec<f64> = samples
            .iter()
            .filter(|s| s.error.is_none() && s.name == task.name)
            .map(|s| s.time_ms as f64)
            .collect();
        let errs = samples.iter().filter(|s| s.error.is_some() && s.name == task.name).count();
        by_name.push(NameStats { name: task.name.clone(), requests: mine.len(), errors: errs, stats: LatencyStats::of(&mine) });
    }
    let aborted = locked(&aborted).clone();
    BenchReport {
        started: Some(started),
        duration_ms: clock.elapsed().as_millis() as u64,
        iterations,
        total_requests: samples.len(),
        errors,
        http_errors,
        bytes,
        aborted,
        overall: LatencyStats::of(&ok),
        by_name,
        samples,
    }
}

fn execute_one(client: &reqwest::blocking::Client, task: &BenchTask, st: &Settings) -> Result<(u64, u16, usize)> {
    let started = Instant::now();
    let resp = http::execute_with(client, &task.prepared, st, None)?;
    Ok((started.elapsed().as_millis() as u64, resp.status, resp.size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_interpolation() {
        let v: Vec<f64> = (1..=100).map(|x| x as f64).collect();
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 50.0), 50.5);
        assert_eq!(percentile(&v, 100.0), 100.0);
        assert!((percentile(&v, 99.0) - 99.01).abs() < 1e-9);
        assert_eq!(percentile(&[7.0], 95.0), 7.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
    }

    #[test]
    fn latency_stats_fields() {
        let s = LatencyStats::of(&[10.0, 20.0, 30.0, 40.0]).unwrap();
        assert_eq!(s.count, 4);
        assert_eq!(s.min, 10.0);
        assert_eq!(s.max, 40.0);
        assert_eq!(s.p50, 25.0);
        assert_eq!(s.p10, 13.0);
        assert!(LatencyStats::of(&[]).is_none());
    }

    fn task(url: String) -> BenchTask {
        BenchTask { name: "t".into(), prepared: Prepared { method: "GET".into(), url, ..Default::default() } }
    }

    #[test]
    fn benchmark_counts_http_and_transport_errors() {
        let srv = crate::echo::start("127.0.0.1", 0).unwrap();
        let st = Settings { no_system_proxy: true, ..Default::default() };
        let seen = AtomicUsize::new(0);
        let tasks = vec![task(format!("{}/get", srv.url())), BenchTask { name: "bad".into(), ..task(format!("{}/status/500", srv.url())) }, BenchTask { name: "dead".into(), ..task("http://127.0.0.1:1/".into()) }];
        let r = benchmark(&tasks, &st, 4, 0, 3, None, &|_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!((r.total_requests, r.errors, r.http_errors, seen.load(Ordering::SeqCst)), (12, 4, 4, 12));
        assert_eq!(r.overall.as_ref().unwrap().count, 8);
        assert_eq!(r.by_name.iter().map(|n| (n.name.as_str(), n.requests, n.errors)).collect::<Vec<_>>(), [("t", 4, 0), ("bad", 4, 0), ("dead", 0, 4)]);
        assert!(r.rps() > 0.0);
        // cancellation is honoured before the first request
        let cancel = AtomicBool::new(true);
        let r = benchmark(&tasks[..1], &st, 5, 0, 1, Some(&cancel), &|_| {});
        assert_eq!((r.total_requests, r.aborted.as_deref()), (0, Some("cancelled")));
    }

    #[test]
    fn llm_profile_verdicts() {
        let srv = crate::echo::start("127.0.0.1", 0).unwrap();
        let st = Settings { no_system_proxy: true, ..Default::default() };
        let post = |path: &str| Prepared { method: "POST".into(), url: format!("{}{path}?api_key=SECRET", srv.url()), body: crate::http::BodySpec::Raw { text: r#"{"model":"m1","stream":true}"#.into() }, ..Default::default() };
        let r = profile_llm(&post("/llm"), &st, 2, 0, None);
        assert_eq!((r.verdict.as_str(), r.samples.len()), ("healthy", 2), "{:?}", r.notes);
        let s = &r.samples[0];
        assert!(s.streamed && s.done && s.events == 13 && s.rate_per_s > 0.0);
        assert_eq!(s.model.as_deref(), Some("m1"));
        assert_eq!(s.usage.as_ref().unwrap().completion_tokens, 13);
        assert_eq!(s.completion, "Hello, this is a streamed test reply from the echo LLM.");
        assert!(r.url.ends_with("api_key=<redacted>"), "{}", r.url);

        let r = profile_llm(&post("/llm/fail"), &st, 1, 0, None);
        assert_eq!(r.verdict, "degraded");
        assert!(r.notes[0].contains("empty stream"), "{:?}", r.notes);
        let r = profile_llm(&post("/post"), &st, 1, 0, None);
        assert_eq!(r.verdict, "degraded");
        assert!(r.notes[0].contains("not an event stream"), "{:?}", r.notes);
        let r = profile_llm(&post("/status/401"), &st, 2, 0, None);
        assert_eq!(r.verdict, "unhealthy");
        assert!(r.samples[0].error.as_ref().unwrap().starts_with("HTTP 401"));
    }

    #[test]
    fn completion_delta_parsing() {
        assert_eq!(completion_delta(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#), Some("Hi".into()));
        assert_eq!(completion_delta(r#"{"choices":[{"delta":{}}]}"#), None);
        assert_eq!(completion_delta("not json"), None);
    }
}
