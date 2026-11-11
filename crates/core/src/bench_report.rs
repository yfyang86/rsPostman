//! Standalone HTML report for a benchmark run: summary tiles, percentile tables, status
//! codes, thresholds, an optional baseline comparison and two charts.
//!
//! The charts are drawn by the same module the GUI uses (`ui/js/charts.js`, embedded here);
//! every number they show is also in a plain table, so the page is complete without scripts.

use crate::bench::{BenchReport, Comparison};
use crate::http::{human_size, human_time};
use crate::profile::LatencyStats;
use crate::report::xml_escape as esc;

const CHARTS_JS: &str = include_str!("../../../ui/js/charts.js");

fn ms(x: f64) -> String {
    if x < 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.0}")
    }
}

fn stats_row(label: &str, s: &Option<LatencyStats>) -> String {
    match s {
        Some(s) => format!(
            "<tr><td>{}</td>{}</tr>",
            esc(label),
            [s.min, s.p10, s.p50, s.p75, s.p90, s.p95, s.p99, s.p999, s.max, s.mean].iter().map(|v| format!("<td class=\"n\">{}</td>", ms(*v))).collect::<String>()
        ),
        None => format!("<tr><td>{}</td><td colspan=\"10\" class=\"muted\">no answered requests</td></tr>", esc(label)),
    }
}

fn load_shape(r: &BenchReport) -> String {
    let o = &r.options;
    let mut parts = vec![match o.duration_ms {
        Some(d) => format!("for {}", human_time(d)),
        None => format!("{} iteration(s) per request", o.iterations),
    }];
    parts.push(format!("{} worker(s)", o.concurrency));
    if let Some(rate) = o.rate {
        parts.push(format!("target {rate} req/s"));
    }
    if o.delay_ms > 0 {
        parts.push(format!("{} ms delay", o.delay_ms));
    }
    if r.warmup_requests > 0 {
        parts.push(format!("{} warm-up request(s)", r.warmup_requests));
    }
    if !o.keep_alive {
        parts.push("new connection per request".into());
    }
    parts.join(" · ")
}

pub fn html(r: &BenchReport, comparison: Option<&Comparison>) -> String {
    let title = match r.by_name.as_slice() {
        [one] => one.name.clone(),
        many => format!("{} requests", many.len()),
    };
    let verdict = if r.options.thresholds.is_empty() {
        String::new()
    } else {
        format!("<div class=\"banner {}\">{}</div>", if r.passed { "ok" } else { "ko" }, if r.passed { "✓ All thresholds passed" } else { "✗ Thresholds failed" })
    };
    let tile = |value: String, label: &str| format!("<div class=\"tile\"><b>{}</b><span>{}</span></div>", esc(&value), esc(label));
    let tiles = [
        tile(r.total_requests.to_string(), "Requests"),
        tile(format!("{:.1}", r.requests_per_second), "Requests / s"),
        tile(r.overall.as_ref().map(|s| format!("{} ms", ms(s.p50))).unwrap_or_else(|| "–".into()), "Latency p50"),
        tile(r.overall.as_ref().map(|s| format!("{} ms", ms(s.p95))).unwrap_or_else(|| "–".into()), "Latency p95"),
        tile(r.overall.as_ref().map(|s| format!("{} ms", ms(s.p99))).unwrap_or_else(|| "–".into()), "Latency p99"),
        tile(format!("{:.2}%", r.error_rate), "Error rate"),
        tile(human_size(r.bytes), "Received"),
        tile(human_time(r.duration_ms), "Duration"),
    ]
    .join("");

    let mut tables = String::new();
    tables.push_str("<h2>Latency (ms)</h2><table><thead><tr><th></th><th class=\"n\">MIN</th><th class=\"n\">P10</th><th class=\"n\">P50</th><th class=\"n\">P75</th><th class=\"n\">P90</th><th class=\"n\">P95</th><th class=\"n\">P99</th><th class=\"n\">P99.9</th><th class=\"n\">MAX</th><th class=\"n\">MEAN</th></tr></thead><tbody>");
    tables.push_str(&stats_row("All requests – total time", &r.overall));
    tables.push_str(&stats_row("All requests – time to first byte", &r.ttfb));
    if r.by_name.len() > 1 {
        for n in &r.by_name {
            tables.push_str(&stats_row(&n.name, &n.stats));
        }
    }
    tables.push_str("</tbody></table>");

    tables.push_str("<h2>Responses</h2><table><thead><tr><th>Status</th><th class=\"n\">Count</th><th class=\"n\">Share</th></tr></thead><tbody>");
    let total = r.total_requests.max(1) as f64;
    for (code, n) in &r.status_counts {
        tables.push_str(&format!("<tr><td>{code}</td><td class=\"n\">{n}</td><td class=\"n\">{:.1}%</td></tr>", *n as f64 * 100.0 / total));
    }
    if r.errors > 0 {
        tables.push_str(&format!("<tr><td>no response (connection error, timeout)</td><td class=\"n\">{}</td><td class=\"n\">{:.1}%</td></tr>", r.errors, r.errors as f64 * 100.0 / total));
    }
    tables.push_str("</tbody></table>");

    if !r.thresholds.is_empty() {
        tables.push_str("<h2>Thresholds</h2><table><thead><tr><th>Condition</th><th class=\"n\">Measured</th><th>Result</th></tr></thead><tbody>");
        for t in &r.thresholds {
            tables.push_str(&format!(
                "<tr><td><code>{}</code></td><td class=\"n\">{}</td><td class=\"{}\">{}</td></tr>",
                esc(&t.expr),
                t.actual.map(|a| format!("{a:.2}")).unwrap_or_else(|| esc(t.note.as_deref().unwrap_or("–"))),
                if t.passed { "pass" } else { "fail" },
                if t.passed { "✓ passed" } else { "✗ failed" }
            ));
        }
        tables.push_str("</tbody></table>");
    }

    if let Some(c) = comparison {
        tables.push_str(&format!(
            "<h2>Compared with the baseline</h2><p class=\"muted\">Tolerance {}%. {}</p><table><thead><tr><th>Metric</th><th class=\"n\">Baseline</th><th class=\"n\">This run</th><th class=\"n\">Change</th><th></th></tr></thead><tbody>",
            c.tolerance_pct,
            if c.regressed { "This run is worse than the baseline." } else { "No regression." }
        ));
        for row in &c.rows {
            tables.push_str(&format!(
                "<tr><td>{}</td><td class=\"n\">{:.2}</td><td class=\"n\">{:.2}</td><td class=\"n\">{:+.1}%</td><td class=\"{}\">{}</td></tr>",
                esc(&row.metric),
                row.baseline,
                row.current,
                row.change_pct,
                if row.regression { "fail" } else { "pass" },
                if row.regression { "✗ regression" } else { "✓ ok" }
            ));
        }
        tables.push_str("</tbody></table>");
    }

    let failures: Vec<&crate::bench::BenchSample> = r.samples.iter().filter(|s| s.error.is_some()).take(20).collect();
    if !failures.is_empty() {
        tables.push_str("<h2>Failed requests (first 20)</h2><table><thead><tr><th>Request</th><th>Iteration</th><th>Error</th></tr></thead><tbody>");
        for s in failures {
            tables.push_str(&format!("<tr><td>{}</td><td class=\"n\">{}</td><td>{}</td></tr>", esc(&s.name), s.iteration + 1, esc(s.error.as_deref().unwrap_or(""))));
        }
        tables.push_str("</tbody></table>");
    }

    // chart data: only what the charts need, with `<` escaped so it cannot close the script
    let data = serde_json::json!({ "histogram": r.histogram, "timeline": r.timeline }).to_string().replace('<', "\\u003c");
    let charts_js = CHARTS_JS.replace("export const ", "const ").replace("export function ", "function ");
    let notes = [r.aborted.as_ref().map(|a| format!("Run ended early: {a}.")), r.samples_truncated.then(|| "Statistics cover every request.".to_string())]
        .into_iter()
        .flatten()
        .map(|n| format!("<p class=\"muted\">{}</p>", esc(&n)))
        .collect::<String>();

    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Benchmark – {title}</title>
<style>
:root {{ color-scheme: light dark; --bg: #f9f9f7; --ink: #0b0b0b; --ink2: #52514e; --line: #e1e0d9; --card: #fcfcfb; --ok: #006300; --bad: #b3261e; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg: #0d0d0d; --ink: #f0efec; --ink2: #c3c2b7; --line: #2c2c2a; --card: #1a1a19; --ok: #0ca30c; --bad: #e66767; }} }}
body {{ font: 14px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; margin: 0; background: var(--bg); color: var(--ink); }}
main {{ max-width: 1040px; margin: 0 auto; padding: 24px 16px 48px; }}
h1 {{ font-size: 20px; margin: 0 0 2px; overflow-wrap: anywhere; }} h2 {{ font-size: 14px; margin: 26px 0 8px; }}
.muted {{ color: var(--ink2); }} p {{ margin: 4px 0; }}
.tiles {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(118px, 1fr)); gap: 10px; margin: 16px 0; }}
.tile {{ background: var(--card); border: 1px solid var(--line); border-radius: 8px; padding: 10px 12px; }}
.tile b {{ display: block; font-size: 19px; font-variant-numeric: tabular-nums; }} .tile span {{ color: var(--ink2); font-size: 12px; }}
.charts {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: 12px; }}
.table-wrap {{ overflow-x: auto; }}
table {{ border-collapse: collapse; width: 100%; background: var(--card); }}
th, td {{ border-bottom: 1px solid var(--line); padding: 6px 10px; text-align: left; overflow-wrap: anywhere; }}
th {{ font-size: 12px; color: var(--ink2); font-weight: 600; }} td.n, th.n {{ text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }}
td.pass {{ color: var(--ok); white-space: nowrap; }} td.fail {{ color: var(--bad); font-weight: 600; white-space: nowrap; }}
.banner {{ padding: 9px 14px; border-radius: 8px; font-weight: 600; margin-top: 14px; border: 1px solid currentColor; }} .banner.ok {{ color: var(--ok); }} .banner.ko {{ color: var(--bad); }}
code {{ font-family: ui-monospace, Menlo, Consolas, monospace; font-size: 13px; }}
</style></head><body><main>
<h1>Benchmark: {title}</h1>
<p class="muted">{started} · {shape}</p>
{notes}{verdict}
<div class="tiles">{tiles}</div>
<div class="charts"><div id="chart-histogram"></div><div id="chart-timeline"></div></div>
<div class="table-wrap">{tables}</div>
</main>
<script>
{charts_js}
const DATA = {data};
const draw = () => benchCharts(document.getElementById("chart-histogram"), document.getElementById("chart-timeline"), DATA);
draw();
let pending = 0;
window.addEventListener("resize", () => {{ clearTimeout(pending); pending = setTimeout(draw, 150); }});
</script>
</body></html>
"#,
        title = esc(&title),
        started = r.started.map(|s| s.format("%Y-%m-%d %H:%M:%S UTC").to_string()).unwrap_or_default(),
        shape = esc(&load_shape(r)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{compare, summarize, BenchOptions, BenchSample};

    #[test]
    fn report_is_self_contained_and_escaped() {
        let samples: Vec<BenchSample> = (0..40u64).map(|i| BenchSample { iteration: i as usize, name: "GET <b>/x</b>".into(), started_ms: i * 60, time_ms: 5 + i, ttfb_ms: 3, status: if i == 7 { 503 } else { 200 }, size: 100, error: None }).collect();
        let opts = BenchOptions { thresholds: vec!["p95<1ms".into(), "error_rate<5%".into()], duration_ms: Some(2500), rate: Some(16.0), ..Default::default() };
        let r = summarize(&["GET <b>/x</b>".to_string()], samples, &opts, Some(chrono::Utc::now()), 2500, 3, None);
        let page = html(&r, Some(&compare(&r, &r, 10.0)));
        assert!(page.contains("<title>Benchmark – GET &lt;b&gt;/x&lt;/b&gt;</title>") && !page.contains("<b>/x</b>"));
        for needle in ["Thresholds failed", "<code>p95&lt;1ms</code>", "✗ failed", "✓ passed", "P99.9", "time to first byte", "<td>503</td>", "Compared with the baseline", "No regression.", "target 16 req/s", "3 warm-up request(s)", "function benchCharts(", "const DATA = {"] {
            assert!(page.contains(needle), "missing {needle}");
        }
        // the embedded module has no ES-module syntax left, and the page loads nothing external
        assert!(!page.contains("export ") && !page.contains("src=\"http") && !page.contains("href=\"http"));
    }
}
