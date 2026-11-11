//! Run reports: JSON, JUnit XML and a self-contained HTML page.

use crate::http::{human_size, human_time};
use crate::runner::RunSummary;

pub fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' && c != '\r' => {}
            c => o.push(c),
        }
    }
    o
}

/// A copy of the summary without credentials, used by every report format.
pub fn redacted(summary: &RunSummary) -> RunSummary {
    let mut s = summary.clone();
    for x in s.executions.iter_mut() {
        x.exchange = crate::secrets::redact_exchange(&x.exchange);
        x.exchange.response = x.exchange.response.as_ref().map(|r| crate::secrets::scrub_response(r, &summary.scrub));
    }
    s
}

pub fn json(summary: &RunSummary) -> String {
    serde_json::to_string_pretty(&redacted(summary)).unwrap_or_default()
}

/// JUnit XML: one `<testsuite>` per request execution, one `<testcase>` per assertion.
pub fn junit(s: &RunSummary) -> String {
    let s = &redacted(s);
    let e = xml_escape;
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<testsuites name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" time=\"{:.3}\">\n",
        e(&s.collection_name),
        s.stats.tests,
        s.stats.tests_failed,
        s.executions.iter().filter(|x| x.exchange.error.is_some() || x.exchange.script_error.is_some()).count(),
        s.duration_ms as f64 / 1000.0
    ));
    for x in &s.executions {
        let ex = &x.exchange;
        let errors = ex.error.iter().chain(ex.script_error.iter()).count();
        let time = ex.response.as_ref().map(|r| r.time_ms).unwrap_or(0) as f64 / 1000.0;
        out.push_str(&format!(
            "  <testsuite name=\"{}\" id=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{errors}\" skipped=\"{}\" time=\"{time:.3}\" timestamp=\"{}\">\n",
            e(&format!("{} (iteration {})", x.path, x.iteration + 1)),
            e(&ex.id),
            ex.tests.len().max(errors),
            ex.failed_tests(),
            ex.tests.iter().filter(|t| t.skipped).count(),
            s.started.format("%Y-%m-%dT%H:%M:%S")
        ));
        for err in ex.error.iter().chain(ex.script_error.iter()) {
            out.push_str(&format!(
                "    <testcase name=\"request\" classname=\"{}\" time=\"{time:.3}\">\n      <error message=\"{}\"/>\n    </testcase>\n",
                e(&x.path),
                e(err)
            ));
        }
        for t in &ex.tests {
            out.push_str(&format!("    <testcase name=\"{}\" classname=\"{}\" time=\"{time:.3}\"", e(&t.name), e(&x.path)));
            if t.skipped {
                out.push_str(">\n      <skipped/>\n    </testcase>\n");
            } else if t.passed {
                out.push_str("/>\n");
            } else {
                let msg = t.error.clone().unwrap_or_else(|| "assertion failed".into());
                out.push_str(&format!(">\n      <failure type=\"AssertionError\" message=\"{}\">{}</failure>\n    </testcase>\n", e(&msg), e(&msg)));
            }
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

pub fn html(s: &RunSummary) -> String {
    let s = &redacted(s);
    let e = xml_escape;
    let st = &s.stats;
    let ok = s.success();
    let mut rows = String::new();
    for x in &s.executions {
        let ex = &x.exchange;
        let (method, url) = ex.request.as_ref().map(|p| (p.method.clone(), p.url.clone())).unwrap_or_default();
        let status = match (&ex.response, &ex.error) {
            (Some(r), _) => format!("{} {}", r.status, r.status_text),
            (None, Some(_)) => "ERROR".into(),
            _ if ex.skipped => "skipped".into(),
            _ => "-".into(),
        };
        let meta = ex.response.as_ref().map(|r| format!("{} · {}", human_time(r.time_ms), human_size(r.size))).unwrap_or_default();
        let mut detail = String::new();
        for err in ex.error.iter().chain(ex.script_error.iter()) {
            detail.push_str(&format!("<div class=\"t fail\">✗ {}</div>", e(err)));
        }
        for t in &ex.tests {
            if t.skipped {
                detail.push_str(&format!("<div class=\"t skip\">– {} <i>(skipped)</i></div>", e(&t.name)));
            } else if t.passed {
                detail.push_str(&format!("<div class=\"t pass\">✓ {}</div>", e(&t.name)));
            } else {
                detail.push_str(&format!("<div class=\"t fail\">✗ {}<pre>{}</pre></div>", e(&t.name), e(t.error.as_deref().unwrap_or(""))));
            }
        }
        for l in &ex.console {
            detail.push_str(&format!("<div class=\"t log\">[{}] {}</div>", e(&l.level), e(&l.message)));
        }
        rows.push_str(&format!(
            "<details class=\"{}\"{}><summary><span class=\"it\">#{}</span><b class=\"m\">{}</b><span class=\"n\">{}</span><span class=\"u\">{}</span><span class=\"s\">{}</span><span class=\"meta\">{}</span></summary>{}</details>\n",
            if ex.failed() { "bad" } else { "good" },
            if ex.failed() { " open" } else { "" },
            x.iteration + 1,
            e(&method),
            e(&x.path),
            e(&url),
            e(&status),
            e(&meta),
            if detail.is_empty() { "<div class=\"t log\">no tests</div>".to_string() } else { detail }
        ));
    }
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name} – rsPostman run report</title>
<style>
body{{font:14px/1.5 system-ui,sans-serif;margin:0;background:#f6f7f9;color:#1f2430}}
header{{background:#1f2430;color:#fff;padding:20px 32px}} header h1{{margin:0 0 4px;font-size:20px}} header p{{margin:0;opacity:.75}}
main{{max-width:1100px;margin:24px auto;padding:0 16px}}
.cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:12px;margin-bottom:20px}}
.card{{background:#fff;border-radius:8px;padding:14px 16px;border:1px solid #e3e6eb}} .card b{{display:block;font-size:22px}} .card span{{color:#667085;font-size:12px;text-transform:uppercase;letter-spacing:.04em}}
.banner{{padding:10px 16px;border-radius:8px;margin-bottom:20px;font-weight:600}} .banner.ok{{background:#e7f6ec;color:#17663a}} .banner.ko{{background:#fdeaea;color:#a12626}}
details{{background:#fff;border:1px solid #e3e6eb;border-left:4px solid #2e9e5b;border-radius:6px;margin-bottom:8px}} details.bad{{border-left-color:#d64545}}
summary{{cursor:pointer;padding:8px 12px;display:flex;gap:10px;align-items:baseline;flex-wrap:wrap}}
.it{{color:#98a1b0;font-size:12px}} .m{{font-size:12px;min-width:48px}} .n{{font-weight:600}} .u{{color:#667085;font-size:12px;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:120px}} .s{{font-weight:600}} .meta{{color:#667085;font-size:12px}}
.t{{padding:4px 16px 4px 28px;border-top:1px solid #f0f1f4}} .pass{{color:#17663a}} .fail{{color:#a12626}} .skip,.log{{color:#667085}} pre{{margin:4px 0 0;white-space:pre-wrap;font-size:12px}}
</style></head><body>
<header><h1>{name}</h1><p>{started} · environment: {env} · duration {duration}</p></header>
<main>
<div class="banner {cls}">{verdict}</div>
<div class="cards">
<div class="card"><b>{iterations}</b><span>Iterations</span></div>
<div class="card"><b>{requests}</b><span>Requests</span></div>
<div class="card"><b>{rfailed}</b><span>Failed requests</span></div>
<div class="card"><b>{tests}</b><span>Assertions</span></div>
<div class="card"><b>{tfailed}</b><span>Failed assertions</span></div>
<div class="card"><b>{avg}</b><span>Avg response time</span></div>
</div>
{rows}
</main></body></html>
"#,
        name = e(&s.collection_name),
        started = s.started.format("%Y-%m-%d %H:%M:%S UTC"),
        env = e(s.environment.as_deref().unwrap_or("none")),
        duration = human_time(s.duration_ms),
        cls = if ok { "ok" } else { "ko" },
        verdict = match (&s.aborted, ok) {
            (Some(a), _) => format!("Run ended early: {}", e(a)),
            (None, true) => "All requests and assertions passed".to_string(),
            (None, false) => "Run finished with failures".to_string(),
        },
        iterations = st.iterations,
        requests = st.requests,
        rfailed = st.requests_failed,
        tests = st.tests,
        tfailed = st.tests_failed,
        avg = human_time(if st.requests > 0 { st.response_time_ms / st.requests as u64 } else { 0 }),
        rows = rows
    )
}
