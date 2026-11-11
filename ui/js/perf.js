// Performance tools: LLM provider profiler and request/collection benchmark.
import { helpButton } from "./help.js";
import { api } from "./api.js";
import { state, views, tools, openSingleton, collectionById } from "./state.js";
import { h, clear, toast, guard, kvEditor, cleanRows, humanSize, humanTime, statusClass, secretInput, isReference } from "./util.js";
import { benchCharts } from "./charts.js";
import { normalizeRequest, newRequest, folders, countRequests, makeAuth } from "./model.js";

const fmtMs = (x) => (x === null || x === undefined ? "–" : x < 10 ? x.toFixed(1) : String(Math.round(x)));

function statsTable(title, rows) {
  return h("div", h("div.section-title", title),
    h("table.data", { testid: "stats-table" },
      h("thead", h("tr", ["", "MIN", "P10", "P50", "P90", "P95", "P99", "MAX", "MEAN"].map((x) => h("th", x)))),
      h("tbody", rows.filter((r) => r.length > 1).map((r) => h("tr", h("td", r[0]), r.slice(1).map((v) => h("td", fmtMs(v))))))));
}

// ---------------------------------------------------------------- LLM provider profile

views.profile = (tab) => {
  const cfg = { url: "", apiKey: "", headers: [], model: "gpt-4o-mini", prompt: "Say hello in a few words.", maxTokens: "", iterations: "3", delay: "0" };
  let job = null; // { id, done, report, error }
  let timer = null;
  const form = h("div.card");
  const results = h("div", { testid: "profile-results" });

  const drawForm = () => {
    const running = job && !job.done;
    const startBtn = running
      ? h("button.btn.danger", { testid: "profile-stop", onClick: () => guard(() => api.post(`/profile/${job.id}/cancel`)) }, "Stop")
      : h("button.btn.primary", { testid: "profile-start", disabled: !cfg.url.trim(), onClick: start }, "Profile");
    clear(form,
      h("div.form-grid",
        h("label", { for: "prof-url" }, "Endpoint URL"),
        h("input", { id: "prof-url", type: "text", placeholder: "https://api.example.com/v1/chat/completions — {{variables}} are supported", value: cfg.url, testid: "profile-url", spellcheck: "false", disabled: running, onInput: (e) => { cfg.url = e.target.value; startBtn.disabled = !cfg.url.trim(); } }),
        h("label", { for: "prof-key" }, "API key (Bearer)"),
        (() => {
          const s = secretInput({ id: "prof-key", testid: "profile-key", placeholder: "sk-… or {{variable}} — sent as Authorization: Bearer, never saved", value: cfg.apiKey, disabled: running,
            onInput: (e) => { cfg.apiKey = e.target.value; s.refresh(); } }, () => !isReference(cfg.apiKey));
          return s.el;
        })(),
        h("label", { for: "prof-model" }, "Model"),
        h("input", { id: "prof-model", type: "text", value: cfg.model, testid: "profile-model", disabled: running, onInput: (e) => { cfg.model = e.target.value; } }),
        h("label", { for: "prof-prompt" }, "Prompt"),
        h("input", { id: "prof-prompt", type: "text", value: cfg.prompt, testid: "profile-prompt", disabled: running, onInput: (e) => { cfg.prompt = e.target.value; } }),
        h("label", { for: "prof-max-tokens" }, "Max tokens"),
        h("input", { id: "prof-max-tokens", type: "number", min: "0", placeholder: "provider default", value: cfg.maxTokens, testid: "profile-max-tokens", disabled: running, onInput: (e) => { cfg.maxTokens = e.target.value; } }),
        h("label", { for: "prof-iterations" }, "Runs"),
        h("input", { id: "prof-iterations", type: "number", min: "1", value: cfg.iterations, testid: "profile-iterations", disabled: running, onInput: (e) => { cfg.iterations = e.target.value; } }),
        h("label", { for: "prof-delay" }, "Delay between runs (ms)"),
        h("input", { id: "prof-delay", type: "number", min: "0", value: cfg.delay, testid: "profile-delay", disabled: running, onInput: (e) => { cfg.delay = e.target.value; } })),
      h("details", { style: "margin:0 0 10px" }, h("summary.muted", { style: "cursor:pointer" }, "Extra headers"), kvEditor(cfg.headers, { testid: "profile-headers", maskSensitive: true })),
      h("div.page-actions",
        startBtn,
        h("span.muted", "Sends a streaming chat request and measures health, time to first token, output rate and token usage. Uses the active environment.")));
  };

  const drawResults = () => {
    if (!job) return clear(results, h("div.placeholder", "Enter the endpoint of an OpenAI-compatible provider and press Profile.", h("br"), "Each run opens a real streaming request; the verdict summarises all runs."));
    const parts = [];
    if (job.error) parts.push(h("div.error-box", { role: "alert" }, h("b", "Profiling failed"), job.error));
    const r = job.report;
    if (r) {
      const ok = r.samples.filter((s) => !s.error).length;
      parts.push(h("div.stat-row", { testid: "profile-stats" },
        h("div.stat." + (r.verdict === "healthy" ? "good" : r.verdict === "degraded" ? "" : "bad"), h("b", { testid: "profile-verdict" }, r.verdict), h("span", `Verdict (${ok}/${r.samples.length} runs ok)`)),
        h("div.stat", h("b", r.ttfb ? humanTime(Math.round(r.ttfb.p50)) : "–"), h("span", "TTFT p50")),
        h("div.stat", h("b", r.total ? humanTime(Math.round(r.total.p50)) : "–"), h("span", "Total p50")),
        h("div.stat", h("b", r.rate ? r.rate.p50.toFixed(1) : "–"), h("span", r.samples.some((s) => s.usage) ? "Tokens/s p50" : "Events/s p50")),
        h("div.stat", h("b", r.samples.find((s) => s.model)?.model || "–"), h("span", "Model"))));
      const hasUsage = r.samples.some((s) => s.usage);
      parts.push(h("div.section-title", "Runs"));
      parts.push(...r.samples.map((s, i) => h("details.run-row" + (s.error ? ".bad" : ""), { testid: "profile-run" },
        h("summary",
          h("span.muted", `run ${i + 1}`),
          s.error ? h("span.status-pill.err", "ERROR") : h("span.status-pill." + statusClass(s.status), `${s.status} ${s.status_text || ""}`),
          h("span.muted", s.error || `ttft ${humanTime(s.ttfb_ms)} · total ${humanTime(s.total_ms)} · ${hasUsage ? (s.usage ? `${s.usage.completion_tokens} completion tokens` : "usage n/a") : `${s.events} events`} · ${s.rate_per_s.toFixed(1)} ${hasUsage ? "tok/s" : "evt/s"}`)),
        !s.error && s.completion ? h("div.log-row", h("span.lvl", "out"), h("span", s.completion)) : null,
        s.error ? h("div.test-row.fail", h("span.tag", "ERROR"), h("div", s.error)) : null)));
      if (r.notes.length) parts.push(h("div.card", { style: "margin-top:10px" }, r.notes.map((n) => h("div.log-row.warn", h("span.lvl", "note"), h("span", n)))));
      parts.push(statsTable("Aggregates (successful runs)", [
        ["TTFT (ms)", ...(r.ttfb ? [r.ttfb.min, r.ttfb.p10, r.ttfb.p50, r.ttfb.p90, r.ttfb.p95, r.ttfb.p99, r.ttfb.max, r.ttfb.mean] : [])],
        ["Total (ms)", ...(r.total ? [r.total.min, r.total.p10, r.total.p50, r.total.p90, r.total.p95, r.total.p99, r.total.max, r.total.mean] : [])],
        [hasUsage ? "Rate (tok/s)" : "Rate (evt/s)", ...(r.rate ? [r.rate.min, r.rate.p10, r.rate.p50, r.rate.p90, r.rate.p95, r.rate.p99, r.rate.max, r.rate.mean] : [])],
      ]));
    } else if (!job.done) {
      parts.push(h("div.progress", h("i", { style: "width:40%" })), h("p.hint", "Profiling…"));
    }
    clear(results, parts);
  };

  const poll = async () => {
    if (!job) return;
    try {
      const r = await api.get(`/profile/${job.id}`);
      job.done = r.done;
      job.report = r.report;
      job.error = r.error;
      if (r.done) {
        clearInterval(timer);
        timer = null;
        drawForm();
      }
      drawResults();
    } catch (e) {
      clearInterval(timer);
      timer = null;
      job.done = true;
      job.error = e.message;
      drawForm();
      drawResults();
    }
  };

  async function start() {
    const request = normalizeRequest(newRequest("POST", cfg.url.trim()));
    request.header = cleanRows(cfg.headers);
    if (cfg.apiKey.trim()) request.auth = makeAuth("bearer", { token: cfg.apiKey.trim() });
    const r = await guard(() => api.post("/profile", {
      request,
      model: cfg.model.trim(),
      prompt: cfg.prompt,
      max_tokens: parseInt(cfg.maxTokens, 10) || 0,
      iterations: parseInt(cfg.iterations, 10) || 3,
      delay_ms: parseInt(cfg.delay, 10) || 0,
    }));
    if (!r) return;
    job = { id: r.id, done: false, report: null, error: null };
    drawForm();
    drawResults();
    timer = setInterval(poll, 300);
    poll();
  }

  tab.onClose = () => { if (timer) clearInterval(timer); };
  drawForm();
  drawResults();
  return h("div.page", { testid: "profile" }, h("h2", "Profile LLM provider", helpButton("profile-an-llm-provider")), h("div.sub", "Evaluate the streaming performance and health of an OpenAI-compatible LLM endpoint."), form, results);
};

tools.push({ id: "profile", label: "Profile LLM", icon: "✦", title: "Profile an LLM provider (streaming health, TTFT, token rate)", action: () => openSingleton("profile", { name: "Profile LLM" }) });

// ---------------------------------------------------------------- benchmark

const STAT_COLS = [["MIN", "min"], ["P10", "p10"], ["P50", "p50"], ["P75", "p75"], ["P90", "p90"], ["P95", "p95"], ["P99", "p99"], ["P99.9", "p999"], ["MAX", "max"], ["MEAN", "mean"]];

function latencyTable(rows) {
  return h("table.data", { testid: "stats-table" },
    h("thead", h("tr", h("th", "Latency (ms)"), STAT_COLS.map(([label]) => h("th", label)))),
    h("tbody", rows.map(([label, st]) => h("tr", h("td", { style: "font-family:inherit" }, label),
      st ? STAT_COLS.map(([, key]) => h("td", fmtMs(st[key]))) : h("td.muted", { colspan: STAT_COLS.length }, "no answered requests")))));
}

views.benchmark = (tab) => {
  const cfg = { scope: "url", url: "", method: "GET", collectionId: "", folder: "", mode: "iterations", iterations: "10", duration: "10", concurrency: "1", rate: "", delay: "0", warmup: "0", keepAlive: true, thresholds: "" };
  let job = null; // { id, total, durationMs, done, report, comparison, error, doneCount, elapsedMs }
  let timer = null;
  let chartsDrawnAt = 0;
  const form = h("div.card");
  const results = h("div", { testid: "benchmark-results" });
  // persistent containers so charts are not rebuilt on every poll
  const head = h("div"), histEl = h("div"), timeEl = h("div"), tail = h("div");
  const chartRow = h("div", { style: "display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:12px;margin:12px 0" }, histEl, timeEl);

  const scopeOptions = () => {
    const c = collectionById(cfg.collectionId);
    return [
      h("option", { value: "" }, c ? `Entire collection (${countRequests(c.item)} requests)` : "—"),
      c ? folders(c).map((f) => h("option", { value: f.id, selected: f.id === cfg.folder }, "Folder: " + f.label)) : null,
    ];
  };

  const num = (id, key, attrs = {}) => h("input", { id, type: "number", min: "0", value: cfg[key], testid: id, disabled: job && !job.done, onInput: (e) => { cfg[key] = e.target.value; }, ...attrs });

  const drawForm = () => {
    const running = job && !job.done;
    const canStart = cfg.scope === "url" ? !!cfg.url.trim() : !!cfg.collectionId;
    const startBtn = running
      ? h("button.btn.danger", { testid: "benchmark-stop", onClick: () => guard(() => api.post(`/benchmark/${job.id}/cancel`)) }, "Stop")
      : h("button.btn.primary", { testid: "benchmark-start", disabled: !canStart, onClick: start }, "Run benchmark");
    const thresholds = h("textarea", { id: "bench-thresholds", testid: "bench-thresholds", spellcheck: "false", disabled: running, style: "min-height:76px",
      placeholder: "one per line, e.g.\np95<200ms\nerror_rate<=1%", onInput: (e) => { cfg.thresholds = e.target.value; } });
    thresholds.value = cfg.thresholds;
    clear(form,
      h("div.form-grid",
        h("label", { for: "bench-scope" }, "Target"),
        h("select", { id: "bench-scope", testid: "bench-scope", disabled: running, onChange: (e) => { cfg.scope = e.target.value; drawForm(); } },
          h("option", { value: "url", selected: cfg.scope === "url" }, "Ad-hoc URL"),
          state.collections.length ? h("option", { value: "collection", selected: cfg.scope === "collection" }, "Collection / folder") : null),
        cfg.scope === "url"
          ? [
              h("label", { for: "bench-method" }, "Method"),
              h("select", { id: "bench-method", testid: "bench-method", disabled: running, onChange: (e) => { cfg.method = e.target.value; } },
                ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].map((m) => h("option", { selected: m === cfg.method }, m))),
              h("label", { for: "bench-url" }, "URL"),
              h("input", { id: "bench-url", type: "text", placeholder: "https://api.example.com/users — {{variables}} are supported", value: cfg.url, testid: "bench-url", spellcheck: "false", disabled: running, onInput: (e) => { cfg.url = e.target.value; startBtn.disabled = !cfg.url.trim(); } }),
            ]
          : [
              h("label", { for: "bench-collection" }, "Collection"),
              h("select", { id: "bench-collection", testid: "bench-collection", disabled: running, onChange: (e) => { cfg.collectionId = e.target.value; cfg.folder = ""; drawForm(); } },
                state.collections.length ? state.collections.map((x) => h("option", { value: x.info._postman_id, selected: x.info._postman_id === cfg.collectionId }, x.info.name)) : h("option", { value: "" }, "No collections")),
              h("label", { for: "bench-folder" }, "Scope"),
              h("select", { id: "bench-folder", testid: "bench-folder", disabled: running, onChange: (e) => { cfg.folder = e.target.value; } }, scopeOptions()),
            ],
        h("label", { for: "bench-mode" }, "Run"),
        h("div.field-row", { style: "margin:0" },
          h("select", { id: "bench-mode", testid: "bench-mode", disabled: running, onChange: (e) => { cfg.mode = e.target.value; drawForm(); } },
            h("option", { value: "iterations", selected: cfg.mode === "iterations" }, "a number of iterations"),
            h("option", { value: "duration", selected: cfg.mode === "duration" }, "for a fixed time")),
          cfg.mode === "iterations"
            ? [num("bench-iterations", "iterations", { min: "1", style: "width:110px", "aria-label": "Iterations per request" }), h("span.muted", "per request")]
            : [num("bench-duration", "duration", { min: "1", style: "width:110px", "aria-label": "Duration in seconds" }), h("span.muted", "seconds")]),
        h("label", { for: "bench-concurrency" }, "Concurrent workers"),
        num("bench-concurrency", "concurrency", { min: "1", max: "256" }),
        h("label", { for: "bench-rate" }, "Request rate (per second)"),
        num("bench-rate", "rate", { placeholder: "as fast as possible", step: "any" }),
        h("label", { for: "bench-warmup" }, "Warm-up rounds (not measured)"),
        num("bench-warmup", "warmup"),
        h("label", { for: "bench-delay" }, "Delay per worker (ms)"),
        num("bench-delay", "delay"),
        h("label", { for: "bench-thresholds" }, "Pass / fail thresholds"),
        thresholds,
        h("label", "Connections"),
        h("label", { style: "color:inherit;display:inline-flex;gap:6px;align-items:center" },
          h("input", { type: "checkbox", checked: cfg.keepAlive, testid: "bench-keepalive", disabled: running, onChange: (e) => { cfg.keepAlive = e.target.checked; } }), "Reuse connections (keep-alive)")),
      h("div.page-actions",
        startBtn,
        h("span.muted", "Scripts are not run. Uses the active environment. Threshold metrics: p50 p90 p95 p99 mean max, ttfb_p95 …, error_rate, rps.")));
  };

  const drawResults = () => {
    if (!job) {
      chartRow.remove();
      return clear(results, h("div.placeholder", "Choose a target and press Run benchmark.", h("br"), "You get latency percentiles, time to first byte, throughput, status codes, a latency distribution and a timeline."));
    }
    const r = job.report;
    const done = r ? r.total_requests : job.doneCount || 0;
    const top = [];
    if (job.error) top.push(h("div.error-box", { role: "alert" }, h("b", "Benchmark failed"), job.error));
    const pct = job.done ? 100 : job.durationMs ? Math.min(100, ((job.elapsedMs || 0) / job.durationMs) * 100) : job.total ? Math.min(100, (done / job.total) * 100) : 5;
    top.push(h("div.progress", { role: "progressbar", "aria-label": "Benchmark progress", "aria-valuenow": Math.round(pct), "aria-valuemax": 100 }, h("i", { style: `width:${pct}%` })));
    const bottom = [];
    if (r) {
      const o = r.overall;
      if (job.done && r.thresholds && r.thresholds.length) {
        top.push(h("div.field-row", { style: "margin-top:12px" }, h("span.status-pill." + (r.passed ? "s2" : "s5"), { testid: "benchmark-verdict" }, r.passed ? "Thresholds passed" : "Thresholds failed")));
      }
      top.push(h("div.stat-row", { testid: "benchmark-stats" },
        h("div.stat", h("b", { testid: "benchmark-requests" }, r.total_requests), h("span", job.done ? "Requests" : "Requests so far")),
        h("div.stat", h("b", { testid: "benchmark-rps" }, r.requests_per_second.toFixed(1)), h("span", "Requests/s")),
        h("div.stat", h("b", o ? humanTime(Math.round(o.p50)) : "–"), h("span", "Latency p50")),
        h("div.stat", h("b", o ? humanTime(Math.round(o.p95)) : "–"), h("span", "Latency p95")),
        h("div.stat", h("b", o ? humanTime(Math.round(o.p99)) : "–"), h("span", "Latency p99")),
        h("div.stat." + (r.errors ? "bad" : "good"), h("b", { testid: "benchmark-errors" }, r.errors), h("span", "No response")),
        h("div.stat." + (r.http_errors ? "bad" : "good"), h("b", { testid: "benchmark-http-errors" }, r.http_errors || 0), h("span", "HTTP 4xx / 5xx")),
        h("div.stat", h("b", humanSize(r.bytes)), h("span", "Received"))));

      bottom.push(latencyTable([
        ["All requests – total", o],
        ["All requests – first byte", r.ttfb],
        ...(r.by_name.length > 1 ? r.by_name.map((n) => [n.name, n.stats]) : []),
      ]));
      const codes = Object.entries(r.status_counts || {});
      if (codes.length || r.errors) {
        bottom.push(h("div.section-title", "Responses"),
          h("table.data", { testid: "status-table", style: "max-width:420px" }, h("thead", h("tr", h("th", "Status"), h("th", "Count"), h("th", "Share"))),
            h("tbody",
              codes.map(([code, n]) => h("tr", h("td", code), h("td", n), h("td", `${((n * 100) / Math.max(1, r.total_requests)).toFixed(1)}%`))),
              r.errors ? h("tr", h("td", { style: "font-family:inherit" }, "no response"), h("td", r.errors), h("td", `${((r.errors * 100) / Math.max(1, r.total_requests)).toFixed(1)}%`)) : null)));
      }
      if (job.done && r.thresholds && r.thresholds.length) {
        bottom.push(h("div.section-title", "Thresholds"),
          h("div", { testid: "threshold-results" }, r.thresholds.map((t) => h("div.test-row." + (t.passed ? "pass" : "fail"), { testid: "threshold-row" },
            h("span.tag", t.passed ? "PASS" : "FAIL"), h("div", h("code.mono", t.expr), h("span.muted", t.actual !== null && t.actual !== undefined ? `  measured ${t.actual.toFixed(2)}` : `  ${t.note || ""}`))))));
      }
      if (job.comparison) {
        const c = job.comparison;
        bottom.push(h("div.section-title", `Compared with the baseline (tolerance ${c.tolerance_pct}%)`),
          h("div.field-row", h("span.status-pill." + (c.regressed ? "s5" : "s2"), { testid: "compare-verdict" }, c.regressed ? "Regression" : "No regression")),
          h("table.data", { testid: "compare-table" }, h("thead", h("tr", ["Metric", "Baseline", "This run", "Change", ""].map((x) => h("th", x)))),
            h("tbody", c.rows.map((row) => h("tr", h("td", { style: "font-family:inherit" }, row.metric), h("td", row.baseline.toFixed(2)), h("td", row.current.toFixed(2)),
              h("td", `${row.change_pct >= 0 ? "+" : ""}${row.change_pct.toFixed(1)}%`), h("td", { style: "font-family:inherit" }, row.regression ? "✗ regression" : "✓ ok"))))));
      }
      const notes = [r.aborted, r.warmup_requests ? `${r.warmup_requests} warm-up request(s) were sent first and are not included.` : null].filter(Boolean);
      if (notes.length) bottom.push(notes.map((n) => h("p.hint", n)));
      const failed = r.samples.filter((s) => s.error);
      if (failed.length) {
        bottom.push(h("div.section-title", `Failed requests (first ${failed.length})`));
        bottom.push(...failed.map((s) => h("div.test-row.fail", h("span.tag", "ERROR"), h("div", `${s.name} (iteration ${s.iteration + 1})`, h("div.err", s.error)))));
      }
      if (job.done) {
        const baseline = h("input", { type: "file", accept: ".json,application/json", class: "hidden", testid: "compare-file", onChange: async (e) => {
          const f = e.target.files[0];
          if (!f) return;
          let parsed;
          try { parsed = JSON.parse(await f.text()); } catch (x) { return toast("The baseline file is not valid JSON", "error"); }
          const c = await guard(() => api.post(`/benchmark/${job.id}/compare`, { baseline: parsed, tolerance: 10 }));
          if (c) { job.comparison = c; drawResults(); }
        } });
        bottom.push(h("div.page-actions", { style: "margin-top:16px" },
          h("a.btn", { href: `/api/benchmark/${job.id}/report?format=html`, target: "_blank", rel: "noopener", testid: "bench-report-html" }, "HTML report"),
          h("a.btn", { href: `/api/benchmark/${job.id}/report?format=json`, testid: "bench-report-json", title: "Also usable later as a baseline" }, "JSON"),
          h("a.btn", { href: `/api/benchmark/${job.id}/report?format=csv`, testid: "bench-report-csv" }, "CSV (samples)"),
          baseline, h("button.btn", { testid: "compare-open", onClick: () => baseline.click() }, "Compare with a baseline…")));
      }
    } else if (!job.done) {
      top.push(h("p.hint", "Running…"));
    }
    clear(head, top);
    clear(tail, bottom);
    if (!results.contains(head)) clear(results, head, chartRow, tail);
    // charts: at most once a second while running, always when finished
    if (r && (job.done || Date.now() - chartsDrawnAt > 1000)) {
      chartsDrawnAt = Date.now();
      benchCharts(histEl, timeEl, r);
    } else if (!r) {
      clear(histEl);
      clear(timeEl);
    }
  };

  const poll = async () => {
    if (!job) return;
    try {
      const r = await api.get(`/benchmark/${job.id}`);
      Object.assign(job, { done: r.done, total: r.total, durationMs: r.duration_ms, elapsedMs: r.elapsed_ms, doneCount: r.done_count, report: r.report, error: r.error, comparison: r.comparison || job.comparison });
      if (r.done) {
        clearInterval(timer);
        timer = null;
        drawForm();
      }
      drawResults();
    } catch (e) {
      clearInterval(timer);
      timer = null;
      job.done = true;
      job.error = e.message;
      drawForm();
      drawResults();
    }
  };

  async function start() {
    const body = {
      concurrency: parseInt(cfg.concurrency, 10) || 1,
      delay_ms: parseInt(cfg.delay, 10) || 0,
      warmup: parseInt(cfg.warmup, 10) || 0,
      keep_alive: cfg.keepAlive,
      thresholds: cfg.thresholds.split(/[\n,;]+/).map((t) => t.trim()).filter(Boolean),
    };
    if (cfg.mode === "duration") body.duration_ms = Math.round((parseFloat(cfg.duration) || 10) * 1000);
    else body.iterations = parseInt(cfg.iterations, 10) || 10;
    const rate = parseFloat(cfg.rate);
    if (rate > 0) body.rate = rate;
    if (cfg.scope === "url") {
      body.request = normalizeRequest(newRequest(cfg.method, cfg.url.trim()));
    } else {
      body.collection_id = cfg.collectionId;
      if (cfg.folder) body.folder_id = cfg.folder;
    }
    const r = await guard(() => api.post("/benchmark", body));
    if (!r) return;
    job = { id: r.id, total: r.total, durationMs: r.duration_ms, done: false, report: null, comparison: null, error: null };
    chartsDrawnAt = 0;
    drawForm();
    drawResults();
    timer = setInterval(poll, 300);
    poll();
  }

  tab.onShow = () => { if (!(job && !job.done)) drawForm(); };
  tab.onClose = () => { if (timer) clearInterval(timer); };
  if (!cfg.collectionId && state.collections[0]) cfg.collectionId = state.collections[0].info._postman_id;
  drawForm();
  drawResults();
  return h("div.page", { testid: "benchmark" }, h("h2", "Benchmark", helpButton("benchmark")), h("div.sub", "Measure latency percentiles, time to first byte and throughput of an endpoint or of every request in a collection."), form, results);
};

tools.push({ id: "benchmark", label: "Benchmark", icon: "⏱", title: "Benchmark an endpoint or collection (percentiles, throughput, thresholds)", action: () => openSingleton("benchmark", { name: "Benchmark" }) });
