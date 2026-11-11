// Performance tools: LLM provider profiler and request/collection benchmark.
import { api } from "./api.js";
import { state, views, tools, openSingleton, collectionById } from "./state.js";
import { h, clear, guard, kvEditor, cleanRows, humanSize, humanTime, statusClass, secretInput, isReference } from "./util.js";
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
  return h("div.page", { testid: "profile" }, h("h2", "Profile LLM provider"), h("div.sub", "Evaluate the streaming performance and health of an OpenAI-compatible LLM endpoint."), form, results);
};

tools.push({ id: "profile", label: "Profile LLM", icon: "✦", title: "Profile an LLM provider (streaming health, TTFT, token rate)", action: () => openSingleton("profile", { name: "Profile LLM" }) });

// ---------------------------------------------------------------- benchmark

views.benchmark = (tab) => {
  const cfg = { scope: "url", url: "", method: "GET", collectionId: "", folder: "", iterations: "10", concurrency: "1", delay: "0" };
  let job = null; // { id, total, done, report, error }
  let timer = null;
  const form = h("div.card");
  const results = h("div", { testid: "benchmark-results" });

  const scopeOptions = () => {
    const c = collectionById(cfg.collectionId);
    return [
      h("option", { value: "" }, c ? `Entire collection (${countRequests(c.item)} requests)` : "—"),
      c ? folders(c).map((f) => h("option", { value: f.id, selected: f.id === cfg.folder }, "Folder: " + f.label)) : null,
    ];
  };

  const drawForm = () => {
    const running = job && !job.done;
    const canStart = cfg.scope === "url" ? !!cfg.url.trim() : !!cfg.collectionId;
    const startBtn = running
      ? h("button.btn.danger", { testid: "benchmark-stop", onClick: () => guard(() => api.post(`/benchmark/${job.id}/cancel`)) }, "Stop")
      : h("button.btn.primary", { testid: "benchmark-start", disabled: !canStart, onClick: start }, "Run benchmark");
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
        h("label", { for: "bench-iterations" }, "Iterations (per request)"),
        h("input", { id: "bench-iterations", type: "number", min: "1", value: cfg.iterations, testid: "bench-iterations", disabled: running, onInput: (e) => { cfg.iterations = e.target.value; } }),
        h("label", { for: "bench-concurrency" }, "Concurrency"),
        h("input", { id: "bench-concurrency", type: "number", min: "1", value: cfg.concurrency, testid: "bench-concurrency", disabled: running, onInput: (e) => { cfg.concurrency = e.target.value; } }),
        h("label", { for: "bench-delay" }, "Delay between requests (ms)"),
        h("input", { id: "bench-delay", type: "number", min: "0", value: cfg.delay, testid: "bench-delay", disabled: running, onInput: (e) => { cfg.delay = e.target.value; } })),
      h("div.page-actions",
        startBtn,
        h("span.muted", "Runs every request the given number of times (up to 256 workers) and reports latency percentiles, throughput and errors. Uses the active environment.")));
  };

  const drawResults = () => {
    if (!job) return clear(results, h("div.placeholder", "Choose a target and press Run benchmark.", h("br"), "Percentiles P10/P50/P90/P95/P99 are computed over all answered requests. Scripts are not run; connections are reused."));
    const parts = [];
    if (job.error) parts.push(h("div.error-box", { role: "alert" }, h("b", "Benchmark failed"), job.error));
    const done = job.report ? job.report.total_requests : job.doneCount || 0;
    parts.push(h("div.progress", { role: "progressbar", "aria-valuenow": done, "aria-valuemax": job.total || done }, h("i", { style: `width:${job.done ? 100 : job.total ? Math.min(100, (done / job.total) * 100) : 5}%` })));
    const r = job.report;
    if (r) {
      const o = r.overall;
      parts.push(h("div.stat-row", { testid: "benchmark-stats" },
        h("div.stat", h("b", { testid: "benchmark-requests" }, r.total_requests), h("span", "Requests")),
        h("div.stat." + (r.errors ? "bad" : "good"), h("b", { testid: "benchmark-errors" }, r.errors), h("span", "No response")),
        h("div.stat." + (r.http_errors ? "bad" : "good"), h("b", { testid: "benchmark-http-errors" }, r.http_errors || 0), h("span", "HTTP 4xx / 5xx")),
        h("div.stat", h("b", o ? humanTime(Math.round(o.p50)) : "–"), h("span", "Latency p50")),
        h("div.stat", h("b", o ? humanTime(Math.round(o.p99)) : "–"), h("span", "Latency p99")),
        h("div.stat", h("b", r.duration_ms ? ((r.total_requests - r.errors) / (r.duration_ms / 1000)).toFixed(2) : "0.00"), h("span", "Requests/s")),
        h("div.stat", h("b", humanSize(r.bytes)), h("span", "Received"))));
      parts.push(statsTable("Latency (answered requests, ms)", [
        ["all requests", ...(o ? [o.min, o.p10, o.p50, o.p90, o.p95, o.p99, o.max, o.mean] : [])],
        ...r.by_name.map((n) => [n.name, ...(n.stats ? [n.stats.min, n.stats.p10, n.stats.p50, n.stats.p90, n.stats.p95, n.stats.p99, n.stats.max, n.stats.mean] : [])]),
      ]));
      if (r.aborted) parts.push(h("p.hint", r.aborted));
      const failed = r.samples.filter((s) => s.error);
      if (failed.length) {
        parts.push(h("div.section-title", `Failed requests (${failed.length})`));
        parts.push(...failed.slice(0, 50).map((s) => h("div.test-row.fail", h("span.tag", "ERROR"), h("div", `${s.name} (iteration ${s.iteration + 1})`, h("div.err", s.error)))));
      }
    } else if (!job.done) {
      parts.push(h("p.hint", `Running… ${done}/${job.total || "?"} requests completed.`));
    }
    clear(results, parts);
  };

  const poll = async () => {
    if (!job) return;
    try {
      const r = await api.get(`/benchmark/${job.id}`);
      job.done = r.done;
      job.total = r.total;
      job.doneCount = r.done_count;
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
    const body = {
      iterations: parseInt(cfg.iterations, 10) || 10,
      concurrency: parseInt(cfg.concurrency, 10) || 1,
      delay_ms: parseInt(cfg.delay, 10) || 0,
    };
    if (cfg.scope === "url") {
      const request = normalizeRequest(newRequest(cfg.method, cfg.url.trim()));
      body.request = request;
    } else {
      body.collection_id = cfg.collectionId;
      if (cfg.folder) body.folder_id = cfg.folder;
    }
    const r = await guard(() => api.post("/benchmark", body));
    if (!r) return;
    job = { id: r.id, total: r.total, done: false, report: null, error: null };
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
  return h("div.page", { testid: "benchmark" }, h("h2", "Benchmark"), h("div.sub", "Measure latency percentiles and throughput of an endpoint or of every request in a collection."), form, results);
};

tools.push({ id: "benchmark", label: "Benchmark", icon: "⏱", title: "Benchmark an endpoint or collection (P10/P50/P90/P99)", action: () => openSingleton("benchmark", { name: "Benchmark" }) });
