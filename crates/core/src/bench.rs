//! Request benchmark: repeat one request (or every request of a collection) and report
//! latency percentiles, time to first byte, throughput, status codes and errors.
//!
//! * Load shape: a fixed number of iterations or a fixed duration, with N concurrent workers,
//!   an optional overall request rate, an optional per-worker delay and optional warm-up
//!   requests that are sent but not measured.
//! * One HTTP client is shared by the whole run, so – like most load tools – only the first
//!   request on each connection pays for the TCP/TLS handshake (`keep_alive: false` opens a
//!   new connection per request instead).
//! * Thresholds such as `p95<200ms` or `error_rate<=1%` turn a run into a pass/fail check, and
//!   [`compare`] measures a run against a saved baseline report.
//!
//! Pre-request and test scripts are not executed. URLs in reports have credentials redacted.

use crate::http::{self, Prepared, Settings};
use crate::profile::LatencyStats;
use anyhow::{anyhow, bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Runs shorter than this are not judged on throughput when compared with a baseline.
pub const MIN_THROUGHPUT_RUN_MS: u64 = 1000;
/// Upper bound for benchmark worker threads.
pub const MAX_CONCURRENCY: usize = 256;
/// A duration-based run stops after this many samples to bound memory use.
pub const MAX_SAMPLES: usize = 2_000_000;
/// Samples written to a report; statistics always cover every sample.
pub const MAX_REPORT_SAMPLES: usize = 50_000;

/// A prepared request with the name shown in reports.
#[derive(Debug, Clone)]
pub struct BenchTask {
    pub name: String,
    pub prepared: Prepared,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BenchOptions {
    /// Rounds over all tasks. Ignored when `duration_ms` is set.
    pub iterations: usize,
    /// Run for this long instead of a fixed number of iterations.
    pub duration_ms: Option<u64>,
    pub concurrency: usize,
    /// Pause between the requests of each worker.
    pub delay_ms: u64,
    /// Target overall request rate (requests per second); `None` = as fast as possible.
    pub rate: Option<f64>,
    /// Unmeasured rounds over all tasks before the clock starts.
    pub warmup: usize,
    /// Reuse connections between requests.
    pub keep_alive: bool,
    /// Pass/fail conditions, e.g. `p95<200ms`, `error_rate<=1%`, `rps>=50`.
    pub thresholds: Vec<String>,
}

impl Default for BenchOptions {
    fn default() -> Self {
        BenchOptions { iterations: 10, duration_ms: None, concurrency: 1, delay_ms: 0, rate: None, warmup: 0, keep_alive: true, thresholds: vec![] }
    }
}

/// One timed request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchSample {
    pub iteration: usize,
    pub name: String,
    /// When the request started, in ms since the start of the measured run.
    #[serde(default)]
    pub started_ms: u64,
    pub time_ms: u64,
    /// Time to first byte (response headers), in ms.
    #[serde(default)]
    pub ttfb_ms: u64,
    pub status: u16,
    pub size: usize,
    /// Transport-level failure (connection refused, timeout, TLS …).
    pub error: Option<String>,
}

impl BenchSample {
    pub fn failed(&self) -> bool {
        self.error.is_some() || self.status >= 400
    }
}

/// Latency stats of the answered requests of one named task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NameStats {
    pub name: String,
    pub requests: usize,
    pub errors: usize,
    #[serde(default)]
    pub http_errors: usize,
    pub stats: Option<LatencyStats>,
}

/// Histogram bucket: latencies in `(from_ms, to_ms]`; `to_ms` is `None` for the open last bucket.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Bucket {
    pub from_ms: u64,
    pub to_ms: Option<u64>,
    pub count: usize,
}

/// One second of the run (by completion time).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Tick {
    pub second: u64,
    pub requests: usize,
    pub failures: usize,
    /// Mean latency of the answered requests that completed in this second.
    pub mean_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ThresholdResult {
    pub expr: String,
    /// The measured value; `None` when the metric could not be computed.
    pub actual: Option<f64>,
    pub passed: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// Aggregated result of a benchmark run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchReport {
    pub started: Option<DateTime<Utc>>,
    pub duration_ms: u64,
    pub options: BenchOptions,
    /// Configured iterations (per task).
    pub iterations: usize,
    pub total_requests: usize,
    /// Requests that got no response at all.
    pub errors: usize,
    /// Responses with a status of 400 or above (they are still timed).
    pub http_errors: usize,
    /// `(errors + http_errors) / total_requests`, in percent.
    pub error_rate: f64,
    /// Answered requests per second of wall-clock time.
    pub requests_per_second: f64,
    pub bytes: usize,
    pub warmup_requests: usize,
    pub aborted: Option<String>,
    /// Latency stats across every answered request.
    pub overall: Option<LatencyStats>,
    /// Time-to-first-byte stats across every answered request.
    pub ttfb: Option<LatencyStats>,
    pub by_name: Vec<NameStats>,
    /// Responses per status code.
    pub status_counts: BTreeMap<u16, usize>,
    pub histogram: Vec<Bucket>,
    pub timeline: Vec<Tick>,
    pub thresholds: Vec<ThresholdResult>,
    /// All thresholds hold (true when none were given).
    pub passed: bool,
    pub samples: Vec<BenchSample>,
    /// `samples` was cut to [`MAX_REPORT_SAMPLES`]; the statistics cover all of them.
    pub samples_truncated: bool,
}

impl BenchReport {
    /// Answered requests per second of wall-clock time.
    pub fn rps(&self) -> f64 {
        self.requests_per_second
    }

    /// Whether the run should be treated as successful: the thresholds when there are any,
    /// otherwise "every request was answered with a status below 400".
    pub fn success(&self) -> bool {
        if self.options.thresholds.is_empty() {
            self.errors == 0 && self.http_errors == 0 && self.total_requests > 0
        } else {
            self.passed
        }
    }
}

const BOUNDS_MS: &[u64] = &[1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 30000];

/// Latency distribution over the occupied range.
///
/// A narrow distribution (largest value within 20× of the smallest) gets up to about a dozen
/// equal-width buckets with a clean step; a wide one falls back to 1-2-5 logarithmic bounds
/// so that both a 3 ms and a 30 s response stay visible.
fn histogram(latencies: &[f64]) -> Vec<Bucket> {
    if latencies.is_empty() {
        return vec![];
    }
    let min = latencies.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = latencies.iter().cloned().fold(0.0, f64::max);
    if max <= min.max(1.0) * 20.0 {
        // linear buckets (from, to] with a 1-2-5 step of at least 1 ms
        let raw = ((max - min) / 12.0).max(1.0);
        let pow = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * pow).find(|s| *s >= raw).unwrap_or(10.0 * pow).max(1.0) as u64;
        let upper = |v: f64| ((v / step as f64).ceil() as u64).max(1) * step;
        let (lo, hi) = (upper(min) - step, upper(max));
        let mut buckets: Vec<Bucket> = (0..(hi - lo) / step).map(|i| Bucket { from_ms: lo + i * step, to_ms: Some(lo + (i + 1) * step), count: 0 }).collect();
        for l in latencies {
            let i = ((upper(*l) - step - lo) / step) as usize;
            if let Some(b) = buckets.get_mut(i) {
                b.count += 1;
            }
        }
        return buckets;
    }
    let mut buckets: Vec<Bucket> = BOUNDS_MS
        .iter()
        .enumerate()
        .map(|(i, to)| Bucket { from_ms: if i == 0 { 0 } else { BOUNDS_MS[i - 1] }, to_ms: Some(*to), count: 0 })
        .collect();
    buckets.push(Bucket { from_ms: *BOUNDS_MS.last().expect("bounds"), to_ms: None, count: 0 });
    for l in latencies {
        let i = BOUNDS_MS.iter().position(|b| *l <= *b as f64).unwrap_or(BOUNDS_MS.len());
        buckets[i].count += 1;
    }
    // keep the occupied range only
    let first = buckets.iter().position(|b| b.count > 0);
    let last = buckets.iter().rposition(|b| b.count > 0);
    match (first, last) {
        (Some(f), Some(l)) => buckets[f..=l].to_vec(),
        _ => vec![],
    }
}

fn timeline(samples: &[BenchSample]) -> Vec<Tick> {
    let mut map: BTreeMap<u64, (usize, usize, f64, usize, f64)> = BTreeMap::new();
    for s in samples {
        let e = map.entry((s.started_ms + s.time_ms) / 1000).or_default();
        e.0 += 1;
        if s.failed() {
            e.1 += 1;
        }
        if s.error.is_none() {
            e.2 += s.time_ms as f64;
            e.3 += 1;
            e.4 = e.4.max(s.time_ms as f64);
        }
    }
    let Some(last) = map.keys().next_back().copied() else { return vec![] };
    (0..=last)
        .map(|second| {
            let (requests, failures, sum, answered, max) = map.get(&second).copied().unwrap_or_default();
            Tick { second, requests, failures, mean_ms: if answered > 0 { sum / answered as f64 } else { 0.0 }, max_ms: max }
        })
        .collect()
}

// ---------------------------------------------------------------- thresholds

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
}

const STATS: &[&str] = &["min", "mean", "max", "stddev", "p10", "p50", "p75", "p90", "p95", "p99", "p999"];

/// Metrics a threshold can refer to.
pub fn threshold_metrics() -> Vec<String> {
    let mut v: Vec<String> = STATS.iter().map(|s| s.to_string()).collect();
    v.extend(STATS.iter().map(|s| format!("ttfb_{s}")));
    v.extend(["error_rate", "errors", "http_errors", "rps", "requests"].iter().map(|s| s.to_string()));
    v
}

/// Parse `metric OP value[ms|s|%]`, e.g. `p95<200ms`, `error_rate <= 1%`, `rps>=50`.
fn parse_threshold(expr: &str) -> Result<(String, Op, f64)> {
    let e: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
    let (pos, op, len) = [("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)]
        .iter()
        .filter_map(|(sym, op)| e.find(sym).map(|p| (p, *op, sym.len())))
        .min_by_key(|(p, _, len)| (*p, usize::MAX - *len))
        .ok_or_else(|| anyhow!("threshold `{expr}` needs one of <, <=, >, >= (example: p95<200ms)"))?;
    let metric = e[..pos].to_lowercase();
    if !threshold_metrics().contains(&metric) {
        bail!("unknown threshold metric `{metric}` in `{expr}` (available: {})", threshold_metrics().join(", "));
    }
    let raw = e[pos + len..].to_lowercase();
    let (number, scale) = if let Some(n) = raw.strip_suffix("ms") {
        (n, 1.0)
    } else if let Some(n) = raw.strip_suffix('s') {
        (n, 1000.0)
    } else if let Some(n) = raw.strip_suffix('%') {
        (n, 1.0)
    } else {
        (raw.as_str(), 1.0)
    };
    let value: f64 = number.parse().map_err(|_| anyhow!("threshold `{expr}` has no valid number after the operator"))?;
    Ok((metric, op, value * scale))
}

/// Check threshold expressions before a run starts.
pub fn validate_thresholds(exprs: &[String]) -> Result<()> {
    for e in exprs {
        parse_threshold(e)?;
    }
    Ok(())
}

fn stat(s: &LatencyStats, name: &str) -> f64 {
    match name {
        "min" => s.min,
        "mean" => s.mean,
        "max" => s.max,
        "stddev" => s.stddev,
        "p10" => s.p10,
        "p50" => s.p50,
        "p75" => s.p75,
        "p90" => s.p90,
        "p95" => s.p95,
        "p99" => s.p99,
        _ => s.p999,
    }
}

fn metric_value(r: &BenchReport, metric: &str) -> Option<f64> {
    match metric {
        "error_rate" => Some(r.error_rate),
        "errors" => Some(r.errors as f64),
        "http_errors" => Some(r.http_errors as f64),
        "rps" => Some(r.requests_per_second),
        "requests" => Some(r.total_requests as f64),
        m => match m.strip_prefix("ttfb_") {
            Some(name) => r.ttfb.as_ref().map(|s| stat(s, name)),
            None => r.overall.as_ref().map(|s| stat(s, m)),
        },
    }
}

fn evaluate(r: &BenchReport, exprs: &[String]) -> Vec<ThresholdResult> {
    exprs
        .iter()
        .map(|expr| match parse_threshold(expr) {
            Err(e) => ThresholdResult { expr: expr.clone(), actual: None, passed: false, note: Some(e.to_string()) },
            Ok((metric, op, limit)) => match metric_value(r, &metric) {
                None => ThresholdResult { expr: expr.clone(), actual: None, passed: false, note: Some("no request was answered".into()) },
                Some(actual) => {
                    let passed = match op {
                        Op::Lt => actual < limit,
                        Op::Le => actual <= limit,
                        Op::Gt => actual > limit,
                        Op::Ge => actual >= limit,
                    };
                    ThresholdResult { expr: expr.clone(), actual: Some(actual), passed, note: None }
                }
            },
        })
        .collect()
}

// ---------------------------------------------------------------- report from samples

/// Build a report from samples. Also used for interim results while a run is in progress.
pub fn summarize(
    names: &[String],
    mut samples: Vec<BenchSample>,
    opts: &BenchOptions,
    started: Option<DateTime<Utc>>,
    duration_ms: u64,
    warmup_requests: usize,
    aborted: Option<String>,
) -> BenchReport {
    samples.sort_by(|a, b| (a.started_ms, a.iteration).cmp(&(b.started_ms, b.iteration)));
    let errors = samples.iter().filter(|s| s.error.is_some()).count();
    let http_errors = samples.iter().filter(|s| s.error.is_none() && s.status >= 400).count();
    let answered: Vec<&BenchSample> = samples.iter().filter(|s| s.error.is_none()).collect();
    let latencies: Vec<f64> = answered.iter().map(|s| s.time_ms as f64).collect();
    let ttfbs: Vec<f64> = answered.iter().map(|s| s.ttfb_ms as f64).collect();
    let mut status_counts = BTreeMap::new();
    for s in &answered {
        *status_counts.entry(s.status).or_insert(0) += 1;
    }
    let by_name = names
        .iter()
        .map(|name| {
            let mine: Vec<&BenchSample> = samples.iter().filter(|s| &s.name == name).collect();
            let ok: Vec<f64> = mine.iter().filter(|s| s.error.is_none()).map(|s| s.time_ms as f64).collect();
            NameStats {
                name: name.clone(),
                requests: ok.len(),
                errors: mine.iter().filter(|s| s.error.is_some()).count(),
                http_errors: mine.iter().filter(|s| s.error.is_none() && s.status >= 400).count(),
                stats: LatencyStats::of(&ok),
            }
        })
        .collect();
    let total = samples.len();
    let mut report = BenchReport {
        started,
        duration_ms,
        options: opts.clone(),
        iterations: opts.iterations,
        total_requests: total,
        errors,
        http_errors,
        error_rate: if total > 0 { (errors + http_errors) as f64 * 100.0 / total as f64 } else { 0.0 },
        requests_per_second: if duration_ms > 0 { answered.len() as f64 / (duration_ms as f64 / 1000.0) } else { 0.0 },
        bytes: samples.iter().map(|s| s.size).sum(),
        warmup_requests,
        aborted,
        overall: LatencyStats::of(&latencies),
        ttfb: LatencyStats::of(&ttfbs),
        by_name,
        status_counts,
        histogram: histogram(&latencies),
        timeline: timeline(&samples),
        thresholds: vec![],
        passed: true,
        samples: vec![],
        samples_truncated: total > MAX_REPORT_SAMPLES,
    };
    report.thresholds = evaluate(&report, &opts.thresholds);
    report.passed = report.thresholds.iter().all(|t| t.passed);
    samples.truncate(MAX_REPORT_SAMPLES);
    report.samples = samples;
    report
}

// ---------------------------------------------------------------- running

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // a poisoned lock only means another worker panicked; the data is still usable
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sleep until `deadline`, waking regularly to honour cancellation. Returns false when cancelled.
fn sleep_until(deadline: Instant, cancel: Option<&AtomicBool>) -> bool {
    loop {
        if cancel.map(|c| c.load(Ordering::SeqCst)).unwrap_or(false) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(50)));
    }
}

/// Run the benchmark described by `opts` over `tasks`.
///
/// Work items (iteration × task) are pulled from a shared counter by `concurrency` worker
/// threads. With a `rate`, item *n* is not started before `n / rate` seconds into the run; if
/// the workers cannot keep up, the achieved rate is lower and shows in the report.
pub fn benchmark(tasks: &[BenchTask], st: &Settings, opts: &BenchOptions, cancel: Option<&AtomicBool>, on_sample: &(dyn Fn(&BenchSample) + Sync)) -> BenchReport {
    let started = Utc::now();
    let mut opts = opts.clone();
    opts.iterations = opts.iterations.max(1);
    opts.concurrency = opts.concurrency.clamp(1, MAX_CONCURRENCY);
    opts.rate = opts.rate.filter(|r| r.is_finite() && *r > 0.0);
    let names: Vec<String> = tasks.iter().map(|t| t.name.clone()).collect();
    let fail = |why: String| summarize(&names, vec![], &opts, Some(started), 0, 0, Some(why));
    if tasks.is_empty() {
        return fail("nothing to benchmark".into());
    }
    let client = match http::build_client_with(st, opts.keep_alive) {
        Ok(c) => c,
        Err(e) => return fail(format!("cannot create the HTTP client: {e:#}")),
    };
    let cancelled = || cancel.map(|c| c.load(Ordering::SeqCst)).unwrap_or(false);

    // warm-up: sent, not measured
    let mut warmup_requests = 0;
    'warm: for _ in 0..opts.warmup {
        for task in tasks {
            if cancelled() {
                break 'warm;
            }
            let _ = http::execute_with(&client, &task.prepared, st, None);
            warmup_requests += 1;
        }
    }

    let task_count = tasks.len();
    let total = task_count * opts.iterations;
    let duration = opts.duration_ms.map(Duration::from_millis);
    let workers = match duration {
        Some(_) => opts.concurrency,
        None => opts.concurrency.min(total.max(1)),
    };
    let samples: Arc<Mutex<Vec<BenchSample>>> = Arc::new(Mutex::new(vec![]));
    let next = Arc::new(AtomicUsize::new(0));
    let aborted = Arc::new(Mutex::new(None::<String>));
    let clock = Instant::now();
    let (opts_ref, cancelled_ref) = (&opts, &cancelled);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (samples, next, aborted, client) = (samples.clone(), next.clone(), aborted.clone(), client.clone());
            scope.spawn(move || {
                let mut first = true;
                loop {
                    if cancelled_ref() {
                        *locked(&aborted) = Some("cancelled".into());
                        break;
                    }
                    if duration.map(|d| clock.elapsed() >= d).unwrap_or(false) {
                        break;
                    }
                    let item = next.fetch_add(1, Ordering::SeqCst);
                    match duration {
                        None if item >= total => break,
                        Some(_) if item >= MAX_SAMPLES => {
                            *locked(&aborted) = Some(format!("stopped after {MAX_SAMPLES} requests (sample limit)"));
                            break;
                        }
                        _ => {}
                    }
                    if let Some(rate) = opts_ref.rate {
                        let slot = clock + Duration::from_secs_f64(item as f64 / rate);
                        if duration.map(|d| slot >= clock + d).unwrap_or(false) {
                            break;
                        }
                        if !sleep_until(slot, cancel) {
                            *locked(&aborted) = Some("cancelled".into());
                            break;
                        }
                    } else if !first && opts_ref.delay_ms > 0 && !sleep_until(Instant::now() + Duration::from_millis(opts_ref.delay_ms), cancel) {
                        *locked(&aborted) = Some("cancelled".into());
                        break;
                    }
                    first = false;
                    let (iteration, task) = (item / task_count, &tasks[item % task_count]);
                    let started_ms = clock.elapsed().as_millis() as u64;
                    let begin = Instant::now();
                    let sample = match http::execute_with(&client, &task.prepared, st, None) {
                        Ok(resp) => BenchSample {
                            iteration,
                            name: task.name.clone(),
                            started_ms,
                            time_ms: begin.elapsed().as_millis() as u64,
                            ttfb_ms: resp.ttfb_ms,
                            status: resp.status,
                            size: resp.size,
                            error: None,
                        },
                        Err(e) => BenchSample { iteration, name: task.name.clone(), started_ms, time_ms: begin.elapsed().as_millis() as u64, error: Some(format!("{e:#}")), ..Default::default() },
                    };
                    on_sample(&sample);
                    locked(&samples).push(sample);
                }
            });
        }
    });

    let samples = std::mem::take(&mut *locked(&samples));
    let aborted = locked(&aborted).clone();
    summarize(&names, samples, &opts, Some(started), clock.elapsed().as_millis() as u64, warmup_requests, aborted)
}

// ---------------------------------------------------------------- comparison with a baseline

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CompareRow {
    pub metric: String,
    pub baseline: f64,
    pub current: f64,
    /// Relative change in percent (positive = the value went up).
    pub change_pct: f64,
    /// The change is in the bad direction and beyond the tolerance.
    pub regression: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Comparison {
    pub tolerance_pct: f64,
    pub rows: Vec<CompareRow>,
    pub regressed: bool,
}

/// Compare a run with a baseline report.
///
/// A latency metric regresses when it is more than `tolerance_pct` percent *and* more than
/// 1 ms slower; throughput when it drops by more than the tolerance (judged only when both
/// runs lasted at least [`MIN_THROUGHPUT_RUN_MS`], because the requests/s of a run that is
/// over in a fraction of a second is mostly noise); the error rate when it rises by more than
/// half a percentage point.
pub fn compare(baseline: &BenchReport, current: &BenchReport, tolerance_pct: f64) -> Comparison {
    let tolerance_pct = tolerance_pct.max(0.0);
    let change = |b: f64, c: f64| if b.abs() < f64::EPSILON { if c.abs() < f64::EPSILON { 0.0 } else { 100.0 } } else { (c - b) * 100.0 / b };
    let mut rows = vec![];
    if let (Some(b), Some(c)) = (&baseline.overall, &current.overall) {
        for m in ["p50", "p90", "p95", "p99", "mean"] {
            let (bv, cv) = (stat(b, m), stat(c, m));
            let pct = change(bv, cv);
            rows.push(CompareRow { metric: format!("{m} (ms)"), baseline: bv, current: cv, change_pct: pct, regression: pct > tolerance_pct && cv - bv > 1.0 });
        }
    }
    let pct = change(baseline.requests_per_second, current.requests_per_second);
    let long_enough = baseline.duration_ms >= MIN_THROUGHPUT_RUN_MS && current.duration_ms >= MIN_THROUGHPUT_RUN_MS;
    rows.push(CompareRow { metric: "requests/s".into(), baseline: baseline.requests_per_second, current: current.requests_per_second, change_pct: pct, regression: long_enough && pct < -tolerance_pct });
    rows.push(CompareRow {
        metric: "error rate (%)".into(),
        baseline: baseline.error_rate,
        current: current.error_rate,
        change_pct: change(baseline.error_rate, current.error_rate),
        regression: current.error_rate - baseline.error_rate > 0.5,
    });
    let regressed = rows.iter().any(|r| r.regression);
    Comparison { tolerance_pct, rows, regressed }
}

/// Every sample as CSV.
pub fn csv(r: &BenchReport) -> String {
    let esc = |s: &str| if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() };
    let mut out = String::from("iteration,name,started_ms,time_ms,ttfb_ms,status,size,error\n");
    for s in &r.samples {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            s.iteration + 1,
            esc(&s.name),
            s.started_ms,
            s.time_ms,
            s.ttfb_ms,
            s.status,
            s.size,
            esc(s.error.as_deref().unwrap_or(""))
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str, url: String) -> BenchTask {
        BenchTask { name: name.into(), prepared: Prepared { method: "GET".into(), url, ..Default::default() } }
    }
    fn st() -> Settings {
        Settings { no_system_proxy: true, ..Default::default() }
    }
    fn opts() -> BenchOptions {
        BenchOptions::default()
    }

    #[test]
    fn counts_http_and_transport_errors_and_status_codes() {
        let srv = crate::echo::start("127.0.0.1", 0).unwrap();
        let seen = AtomicUsize::new(0);
        let tasks = vec![task("t", format!("{}/get", srv.url())), task("bad", format!("{}/status/500", srv.url())), task("dead", "http://127.0.0.1:1/".into())];
        let r = benchmark(&tasks, &st(), &BenchOptions { iterations: 4, concurrency: 3, ..opts() }, None, &|_| {
            seen.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!((r.total_requests, r.errors, r.http_errors, seen.load(Ordering::SeqCst)), (12, 4, 4, 12));
        assert_eq!(r.overall.as_ref().unwrap().count, 8);
        assert_eq!(r.ttfb.as_ref().unwrap().count, 8);
        assert_eq!(r.by_name.iter().map(|n| (n.name.as_str(), n.requests, n.errors, n.http_errors)).collect::<Vec<_>>(), [("t", 4, 0, 0), ("bad", 4, 0, 4), ("dead", 0, 4, 0)]);
        assert_eq!(r.status_counts, BTreeMap::from([(200, 4), (500, 4)]));
        assert!((r.error_rate - 66.666).abs() < 0.01);
        assert_eq!(r.histogram.iter().map(|b| b.count).sum::<usize>(), 8);
        assert_eq!(r.timeline.iter().map(|t| t.requests).sum::<usize>(), 12);
        assert!(r.rps() > 0.0 && !r.success() && r.passed);
        // cancellation is honoured before the first request
        let cancel = AtomicBool::new(true);
        let r = benchmark(&tasks[..1], &st(), &BenchOptions { iterations: 5, ..opts() }, Some(&cancel), &|_| {});
        assert_eq!((r.total_requests, r.aborted.as_deref()), (0, Some("cancelled")));
        assert_eq!(benchmark(&[], &st(), &opts(), None, &|_| {}).aborted.as_deref(), Some("nothing to benchmark"));
    }

    #[test]
    fn latency_is_measured_against_a_known_delay() {
        let srv = crate::echo::start("127.0.0.1", 0).unwrap();
        let tasks = vec![task("slow", format!("{}/delay/60", srv.url()))];
        let r = benchmark(&tasks, &st(), &BenchOptions { iterations: 6, concurrency: 3, warmup: 2, ..opts() }, None, &|_| {});
        let s = r.overall.as_ref().unwrap();
        assert_eq!((r.total_requests, r.warmup_requests, s.count), (6, 2, 6));
        assert!(s.min >= 60.0 && s.p50 < 400.0, "{s:?}");
        // the server sleeps before answering, so time to first byte carries the delay too
        assert!(r.ttfb.as_ref().unwrap().min >= 55.0, "{:?}", r.ttfb);
        // three workers finish six 60 ms requests in about two rounds, not six
        assert!(r.duration_ms < 60 * 6, "{}", r.duration_ms);
        assert!(r.histogram.iter().all(|b| b.from_ms >= 50), "{:?}", r.histogram);
    }

    #[test]
    fn duration_and_rate_shape_the_load() {
        let srv = crate::echo::start("127.0.0.1", 0).unwrap();
        let tasks = vec![task("t", format!("{}/get", srv.url()))];
        // 20 requests per second for one second -> about 20 requests, not thousands
        let r = benchmark(&tasks, &st(), &BenchOptions { duration_ms: Some(1000), rate: Some(20.0), concurrency: 4, ..opts() }, None, &|_| {});
        assert!((15..=21).contains(&r.total_requests), "{}", r.total_requests);
        assert!((900..1500).contains(&r.duration_ms), "{}", r.duration_ms);
        assert!(r.requests_per_second > 10.0 && r.requests_per_second < 25.0, "{}", r.requests_per_second);
        // starts are spread over the second rather than bunched at the beginning
        assert!(r.samples.iter().filter(|s| s.started_ms >= 500).count() >= 6, "{:?}", r.samples.iter().map(|s| s.started_ms).collect::<Vec<_>>());
        // without a rate, a duration run simply keeps going until the time is up
        let r = benchmark(&tasks, &st(), &BenchOptions { duration_ms: Some(300), concurrency: 2, ..opts() }, None, &|_| {});
        assert!(r.total_requests > 20 && r.duration_ms >= 300 && r.duration_ms < 1500, "{} in {} ms", r.total_requests, r.duration_ms);
        // a fixed number of iterations with a rate takes the expected time
        let r = benchmark(&tasks, &st(), &BenchOptions { iterations: 10, rate: Some(50.0), ..opts() }, None, &|_| {});
        assert!(r.total_requests == 10 && r.duration_ms >= 170, "{}", r.duration_ms);
        // without keep-alive every request still succeeds
        let r = benchmark(&tasks, &st(), &BenchOptions { iterations: 5, keep_alive: false, ..opts() }, None, &|_| {});
        assert_eq!((r.total_requests, r.errors), (5, 0));
    }

    fn report(latencies: &[u64], failures: usize, duration_ms: u64, thresholds: &[&str]) -> BenchReport {
        let mut samples: Vec<BenchSample> = latencies.iter().enumerate().map(|(i, l)| BenchSample { iteration: i, name: "t".into(), started_ms: i as u64 * 10, time_ms: *l, ttfb_ms: *l / 2, status: 200, size: 10, error: None }).collect();
        for i in 0..failures {
            samples.push(BenchSample { iteration: 1000 + i, name: "t".into(), status: 503, ..Default::default() });
        }
        summarize(&["t".to_string()], samples, &BenchOptions { thresholds: thresholds.iter().map(|s| s.to_string()).collect(), ..opts() }, None, duration_ms, 0, None)
    }

    #[test]
    fn thresholds() {
        assert_eq!(parse_threshold("p95<200ms").unwrap(), ("p95".into(), Op::Lt, 200.0));
        assert_eq!(parse_threshold(" error_rate <= 1% ").unwrap(), ("error_rate".into(), Op::Le, 1.0));
        assert_eq!(parse_threshold("mean<1.5s").unwrap(), ("mean".into(), Op::Lt, 1500.0));
        assert_eq!(parse_threshold("RPS>=50").unwrap(), ("rps".into(), Op::Ge, 50.0));
        assert_eq!(parse_threshold("ttfb_p99>3").unwrap(), ("ttfb_p99".into(), Op::Gt, 3.0));
        for bad in ["p95", "p96<1", "p95<fast", "<5", "p95=5"] {
            assert!(parse_threshold(bad).is_err(), "{bad}");
        }
        assert!(validate_thresholds(&["p50<1".into(), "nope>1".into()]).unwrap_err().to_string().contains("unknown threshold metric `nope`"));

        let latencies: Vec<u64> = (1..=100).collect();
        let r = report(&latencies, 0, 1000, &["p95<200ms", "p50<=50.5", "error_rate<=1%", "rps>=100", "max<50", "ttfb_p50<30"]);
        assert_eq!(r.thresholds.iter().map(|t| t.passed).collect::<Vec<_>>(), [true, true, true, true, false, true]);
        assert_eq!(r.thresholds[4].actual, Some(100.0));
        assert!(!r.passed && !r.success());
        // with thresholds, success follows them even though some responses were errors
        let r = report(&latencies, 2, 1000, &["error_rate<5%"]);
        assert!(r.passed && r.success() && r.http_errors == 2);
        // a latency threshold cannot pass when nothing was answered
        let r = summarize(&[], vec![BenchSample { error: Some("refused".into()), ..Default::default() }], &BenchOptions { thresholds: vec!["p95<100".into()], ..opts() }, None, 10, 0, None);
        assert!(!r.passed && r.thresholds[0].note.as_deref() == Some("no request was answered"));
    }

    #[test]
    fn histogram_timeline_and_csv() {
        let h = histogram(&[0.0, 1.0, 1.5, 7.0, 7.0, 450.0, 40_000.0]);
        assert_eq!(h.first().unwrap(), &Bucket { from_ms: 0, to_ms: Some(1), count: 2 });
        assert_eq!(h.iter().find(|b| b.to_ms == Some(10)).unwrap().count, 2);
        assert_eq!(h.last().unwrap(), &Bucket { from_ms: 30000, to_ms: None, count: 1 });
        assert_eq!(h.iter().map(|b| b.count).sum::<usize>(), 7);
        assert!(histogram(&[]).is_empty());
        // a narrow distribution gets fine, equal-width buckets instead of one coarse bar
        let narrow = histogram(&[12.0, 12.0, 13.0, 13.0, 13.0, 14.0]);
        assert_eq!(narrow, [Bucket { from_ms: 11, to_ms: Some(12), count: 2 }, Bucket { from_ms: 12, to_ms: Some(13), count: 3 }, Bucket { from_ms: 13, to_ms: Some(14), count: 1 }]);
        let mid = histogram(&(100..=340).map(|x| x as f64).collect::<Vec<_>>());
        assert!(mid.len() >= 8 && mid.len() <= 14, "{}", mid.len());
        assert_eq!((mid[0].from_ms, mid[0].to_ms, mid.iter().map(|b| b.count).sum::<usize>()), (80, Some(100), 241));
        assert!(mid.windows(2).all(|w| w[0].to_ms == Some(w[1].from_ms)));
        assert_eq!(histogram(&[0.0, 0.0]), [Bucket { from_ms: 0, to_ms: Some(1), count: 2 }]);
        assert_eq!(histogram(&[7.0]), [Bucket { from_ms: 6, to_ms: Some(7), count: 1 }]);

        let samples = vec![
            BenchSample { started_ms: 0, time_ms: 100, status: 200, ..Default::default() },
            BenchSample { started_ms: 500, time_ms: 300, status: 200, ..Default::default() },
            BenchSample { started_ms: 2100, time_ms: 50, status: 500, ..Default::default() },
            BenchSample { started_ms: 2200, time_ms: 10, error: Some("x".into()), ..Default::default() },
        ];
        let t = timeline(&samples);
        assert_eq!(t.len(), 3);
        assert_eq!((t[0].requests, t[0].failures, t[0].mean_ms, t[0].max_ms), (2, 0, 200.0, 300.0));
        assert_eq!((t[1].requests, t[1].mean_ms), (0, 0.0));
        assert_eq!((t[2].requests, t[2].failures, t[2].mean_ms), (2, 2, 50.0));

        let mut r = report(&[5, 6], 0, 100, &[]);
        r.samples[0].name = "a,\"b\"".into();
        r.samples[1].error = Some("line1\nline2".into());
        let text = csv(&r);
        assert!(text.starts_with("iteration,name,started_ms,time_ms,ttfb_ms,status,size,error\n1,\"a,\"\"b\"\"\",0,5,2,200,10,\n"), "{text}");
        assert!(text.contains("\"line1\nline2\""));
    }

    #[test]
    fn comparison_with_a_baseline() {
        let base = report(&(1..=100).map(|x| x + 100).collect::<Vec<_>>(), 0, 1000, &[]);
        let same = compare(&base, &base, 10.0);
        assert!(!same.regressed && same.rows.iter().all(|r| r.change_pct == 0.0));
        assert_eq!(same.rows.iter().map(|r| r.metric.as_str()).collect::<Vec<_>>(), ["p50 (ms)", "p90 (ms)", "p95 (ms)", "p99 (ms)", "mean (ms)", "requests/s", "error rate (%)"]);

        let slower = report(&(1..=100).map(|x| x + 150).collect::<Vec<_>>(), 0, 1000, &[]);
        let c = compare(&base, &slower, 10.0);
        assert!(c.regressed && c.rows[0].regression && c.rows[0].change_pct > 30.0);
        assert!(!compare(&base, &slower, 60.0).regressed, "within a generous tolerance");
        assert!(!compare(&slower, &base, 10.0).regressed, "getting faster is not a regression");

        let throttled = report(&(1..=100).map(|x| x + 100).collect::<Vec<_>>(), 0, 2000, &[]);
        let c = compare(&base, &throttled, 10.0);
        assert!(c.regressed && c.rows[5].regression && (c.rows[5].change_pct + 50.0).abs() < 0.01);
        let failing = report(&(1..=100).map(|x| x + 100).collect::<Vec<_>>(), 3, 1000, &[]);
        assert!(compare(&base, &failing, 10.0).rows[6].regression);
        // throughput of runs that are over in a fraction of a second is noise, not a verdict
        let (quick, quicker) = (report(&[1, 1, 1, 1], 0, 400, &[]), report(&[1, 1, 1, 1], 0, 200, &[]));
        let c = compare(&quicker, &quick, 10.0);
        assert!(!c.regressed && c.rows[5].change_pct < -40.0);
        // sub-millisecond noise is ignored even when it is a large percentage
        let (a, b) = (report(&[1, 1, 1, 1], 0, 100, &[]), report(&[2, 2, 2, 2], 0, 100, &[]));
        assert!(!compare(&a, &b, 10.0).rows[0].regression);
    }
}
