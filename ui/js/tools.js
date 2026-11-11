// Toolbar tools: import, code snippets, collection runner, mock servers, WebSocket client,
// cookie manager and settings.
import { api } from "./api.js";
import { state, tools, views, collectionActions, requestActions, emit, openTab, requestTab, collectionById, openSingleton, refreshData } from "./state.js";
import { h, clear, toast, guard, modal, kvEditor, cleanRows, confirmDialog, copyText, humanSize, humanTime, methodClass, statusClass, download } from "./util.js";
import { normalizeRequest, folders, countRequests } from "./model.js";

export function importDialog() {
  const text = h("textarea", { style: "width:100%;min-height:200px", spellcheck: "false", placeholder: "Paste a curl command, Postman collection / environment JSON, OpenAPI (JSON or YAML) or HAR…", testid: "import-text", "aria-label": "Import text" });
  const file = h("input", { type: "file", accept: ".json,.yaml,.yml,.har,.txt", testid: "import-file", "aria-label": "Import file", onChange: async (e) => {
    const f = e.target.files[0];
    if (f) text.value = await f.text();
  } });
  modal({
    title: "Import",
    wide: true,
    testid: "import-dialog",
    body: [h("p.hint", "Supported: Postman Collection v2.0 / v2.1, Postman environments and globals, OpenAPI 3 / Swagger 2, HAR, curl."), h("div.field-row", file), text],
    buttons: [
      { label: "Cancel" },
      { label: "Import", primary: true, testid: "import-confirm", action: async () => {
        if (!text.value.trim()) return text.focus(), false;
        const r = await guard(() => api.post("/import", { text: text.value }));
        if (!r) return false;
        if (r.kind === "collection") {
          state.collections.push(r.collection);
          state.collections.sort((a, b) => a.info.name.toLowerCase().localeCompare(b.info.name.toLowerCase()));
          state.expanded.add(r.collection.info._postman_id);
          state.side = "collections";
          emit("collections");
          toast(`Imported collection “${r.collection.info.name}”`, "ok");
        } else if (r.kind === "environment") {
          state.environments.push(r.environment);
          emit("environments");
          toast(`Imported environment “${r.environment.name}”`, "ok");
        } else if (r.kind === "globals") {
          state.globals = r.globals;
          emit("environments");
          toast("Global variables merged", "ok");
        } else if (r.kind === "request") {
          const req = normalizeRequest(r.request);
          openTab(requestTab({ name: `${req.method} ${req.url.raw.replace(/^https?:\/\//, "").split("?")[0]}`.slice(0, 60), request: req, dirty: true }));
          toast("curl command imported into a new tab", "ok");
        }
        return true;
      } },
    ],
  });
}

tools.push({ id: "import", label: "Import", icon: "⇩", title: "Import collections, environments, OpenAPI, HAR or curl", action: importDialog });

// ---------------------------------------------------------------- code snippets

function codeDialog(tab, getRequest) {
  let lang = "curl";
  try { lang = localStorage.getItem("rsp-codegen") || "curl"; } catch (e) {}
  const out = h("pre.code.nowrap", { style: "padding:12px;background:var(--code-bg);border-radius:8px;min-height:160px;overflow:auto", testid: "codegen-output" });
  const select = h("select", { testid: "codegen-lang", "aria-label": "Language", onChange: (e) => { lang = e.target.value; try { localStorage.setItem("rsp-codegen", lang); } catch (x) {} draw(); } },
    state.languages.map((l) => h("option", { value: l.id, selected: l.id === lang }, l.name)));
  const draw = async () => {
    out.textContent = "Generating…";
    try {
      const r = await api.post("/codegen", { lang, request: getRequest(), collection_id: tab.collectionId, item_id: tab.itemId });
      out.textContent = r.code;
    } catch (e) {
      out.textContent = "Could not generate the snippet:\n" + e.message;
    }
  };
  draw();
  modal({
    title: "Code snippet",
    wide: true,
    testid: "codegen-dialog",
    body: [h("div.field-row", h("label.muted", "Language"), select, h("span.muted", "Variables and authorization are resolved with the active environment – the snippet contains the real credentials.")), out],
    buttons: [{ label: "Copy", action: () => { copyText(out.textContent); return false; } }, { label: "Close", primary: true }],
  });
}
requestActions.push((tab, getRequest) => [h("button.btn", { testid: "codegen", title: "Generate a code snippet", onClick: () => codeDialog(tab, getRequest) }, "</>")]);

// ---------------------------------------------------------------- collection runner

export function openRunner(collectionId = null, folderId = null) {
  const tab = openSingleton("runner", { name: "Runner" });
  if (collectionId && tab.preset) tab.preset(collectionId, folderId);
  else if (collectionId) tab.initial = { collectionId, folderId };
}

views.runner = (tab) => {
  const cfg = { collectionId: (tab.initial && tab.initial.collectionId) || (state.collections[0] && state.collections[0].info._postman_id) || "", folder: (tab.initial && tab.initial.folderId) || "", iterations: "", delay: "0", data: "", dataName: "", bail: false, persist: true };
  let run = null; // { id, executions: [], done, summary, total, error }
  let timer = null;
  const form = h("div.card");
  const results = h("div", { testid: "run-results" });

  const drawForm = () => {
    const c = collectionById(cfg.collectionId);
    const running = run && !run.done;
    const file = h("input", { type: "file", accept: ".csv,.json,.txt", class: "hidden", testid: "run-data-file", onChange: async (e) => {
      const f = e.target.files[0];
      if (f) { cfg.data = await f.text(); cfg.dataName = f.name; drawForm(); }
    } });
    clear(form,
      h("div.form-grid",
        h("label", { for: "run-collection" }, "Collection"),
        h("select", { id: "run-collection", testid: "run-collection", disabled: running, onChange: (e) => { cfg.collectionId = e.target.value; cfg.folder = ""; drawForm(); } },
          state.collections.length ? state.collections.map((x) => h("option", { value: x.info._postman_id, selected: x.info._postman_id === cfg.collectionId }, x.info.name)) : h("option", { value: "" }, "No collections")),
        h("label", { for: "run-folder" }, "Run"),
        h("select", { id: "run-folder", testid: "run-folder", disabled: running, onChange: (e) => { cfg.folder = e.target.value; } },
          h("option", { value: "" }, c ? `Entire collection (${countRequests(c.item)} requests)` : "—"),
          c ? folders(c).map((f) => h("option", { value: f.id, selected: f.id === cfg.folder }, "Folder: " + f.label)) : null),
        h("label", { for: "run-iterations" }, "Iterations"),
        h("input", { id: "run-iterations", type: "number", min: "1", placeholder: cfg.data ? "one per data row" : "1", value: cfg.iterations, testid: "run-iterations", disabled: running, onInput: (e) => { cfg.iterations = e.target.value; } }),
        h("label", { for: "run-delay" }, "Delay between requests (ms)"),
        h("input", { id: "run-delay", type: "number", min: "0", value: cfg.delay, testid: "run-delay", disabled: running, onInput: (e) => { cfg.delay = e.target.value; } }),
        h("label", "Data file (CSV or JSON)"),
        h("div.field-row", { style: "margin:0" }, file, h("button.btn.small", { disabled: running, onClick: () => file.click() }, "Choose file…"),
          cfg.data ? h("span", { testid: "run-data-name" }, cfg.dataName || "pasted data", " ", h("button.icon-btn", { "aria-label": "Remove data file", onClick: () => { cfg.data = ""; cfg.dataName = ""; drawForm(); } }, "✕")) : h("span.muted", "none")),
        h("label", "Options"),
        h("div.radio-row", { style: "margin:0" },
          h("label", h("input", { type: "checkbox", checked: cfg.bail, testid: "run-bail", disabled: running, onChange: (e) => { cfg.bail = e.target.checked; } }), "Stop on first failure"),
          h("label", h("input", { type: "checkbox", checked: cfg.persist, testid: "run-persist", disabled: running, onChange: (e) => { cfg.persist = e.target.checked; } }), "Keep variable changes"))),
      h("div.page-actions",
        running
          ? h("button.btn.danger", { testid: "run-stop", onClick: () => guard(() => api.post(`/runs/${run.id}/cancel`)) }, "Stop run")
          : h("button.btn.primary", { testid: "run-start", disabled: !c, onClick: start }, "Run"),
        h("span.muted", "Uses the active environment. Scripts, tests and flow control (setNextRequest) are honoured.")));
  };

  const drawResults = () => {
    if (!run) return clear(results, h("div.placeholder", "Choose a collection and press Run.", h("br"), "Each request runs with its pre-request and test scripts; results appear here live."));
    const s = run.summary;
    const st = s ? s.stats : null;
    const done = run.executions.length;
    const failed = run.executions.filter((x) => x.failed).length;
    const parts = [];
    if (run.error) parts.push(h("div.error-box", { role: "alert" }, h("b", "Run failed"), run.error));
    parts.push(h("div.progress", { role: "progressbar", "aria-valuenow": done, "aria-valuemax": run.total || done, "aria-label": "Run progress" }, h("i", { style: `width:${run.done ? 100 : run.total ? Math.min(100, (done / run.total) * 100) : 5}%` })));
    parts.push(h("div.stat-row", { testid: "run-stats" },
      h("div.stat", h("b", st ? st.iterations : "…"), h("span", "Iterations")),
      h("div.stat", h("b", { testid: "run-requests" }, st ? st.requests : done), h("span", "Requests")),
      h("div.stat." + (failed ? "bad" : "good"), h("b", { testid: "run-failed" }, st ? st.requests_failed : failed), h("span", "Failed requests")),
      h("div.stat", h("b", st ? st.tests : run.executions.reduce((n, x) => n + x.tests.length, 0)), h("span", "Assertions")),
      h("div.stat." + (st && st.tests_failed ? "bad" : "good"), h("b", st ? st.tests_failed : run.executions.reduce((n, x) => n + x.tests.filter((t) => !t.passed).length, 0)), h("span", "Failed assertions")),
      h("div.stat", h("b", s ? humanTime(s.duration_ms) : h("span.spinner")), h("span", s ? "Duration" : "Running"))));
    if (s) {
      parts.push(h("div.field-row", { testid: "run-verdict" },
        h("span.status-pill." + (s.success ? "s2" : "s5"), s.success ? "Passed" : "Failed"),
        s.aborted ? h("span.muted", "Ended early: " + s.aborted) : null,
        h("span.spacer"),
        h("a.btn.small", { href: `/api/runs/${run.id}/report?format=html`, target: "_blank", rel: "noopener", testid: "run-report-html" }, "HTML report"),
        h("a.btn.small", { href: `/api/runs/${run.id}/report?format=junit`, testid: "run-report-junit" }, "JUnit XML"),
        h("a.btn.small", { href: `/api/runs/${run.id}/report?format=json`, testid: "run-report-json" }, "JSON")));
    }
    let lastIteration = -1;
    for (const x of run.executions) {
      if (x.iteration !== lastIteration && (run.iterations > 1 || x.iteration > 0)) {
        lastIteration = x.iteration;
        parts.push(h("div.section-title", `Iteration ${x.iteration + 1}`));
      }
      const m = x.method || "";
      parts.push(h("details.run-row" + (x.failed ? ".bad" : x.skipped ? ".skipped" : ""), { open: x.failed, testid: "run-row" },
        h("summary",
          h("span.method." + methodClass(m), { style: "width:auto" }, m),
          h("span.path", x.path),
          x.status ? h("span.status-pill." + statusClass(x.status), `${x.status} ${x.status_text || ""}`) : x.skipped ? h("span.muted", "skipped") : h("span.status-pill.err", "Error"),
          h("span.muted", x.time_ms !== null && x.time_ms !== undefined ? `${humanTime(x.time_ms)} · ${humanSize(x.size || 0)}` : ""),
          h("span.muted", x.tests.length ? `${x.tests.filter((t) => t.passed).length}/${x.tests.length} tests` : "")),
        x.url ? h("div.log-row", h("span.lvl", "url"), h("span", x.url)) : null,
        x.error ? h("div.test-row.fail", h("span.tag", "ERROR"), h("div", x.error)) : null,
        x.script_error ? h("div.test-row.fail", h("span.tag", "ERROR"), h("div", x.script_error)) : null,
        x.tests.map((t) => h("div.test-row." + (t.skipped ? "skip" : t.passed ? "pass" : "fail"), h("span.tag", t.skipped ? "SKIP" : t.passed ? "PASS" : "FAIL"), h("div", t.name, t.error ? h("div.err", t.error) : null))),
        x.console.map((l) => h("div.log-row." + l.level, h("span.lvl", l.level), h("span", l.message)))));
    }
    clear(results, parts);
  };

  const poll = async () => {
    if (!run) return;
    try {
      const r = await api.get(`/runs/${run.id}?since=${run.executions.length}`);
      run.executions.push(...r.executions);
      run.total = r.total;
      run.iterations = r.iterations;
      run.error = r.error;
      if (r.done) {
        run.done = true;
        run.summary = r.summary;
        clearInterval(timer);
        timer = null;
        drawForm();
        guard(refreshData);
      }
      drawResults();
    } catch (e) {
      clearInterval(timer);
      timer = null;
      run.done = true;
      run.error = e.message;
      drawForm();
      drawResults();
    }
  };

  async function start() {
    const iterations = parseInt(cfg.iterations, 10);
    const r = await guard(() => api.post("/runs", { collection_id: cfg.collectionId, folder: cfg.folder || null, iterations: iterations > 0 ? iterations : null, delay_ms: parseInt(cfg.delay, 10) || 0, data: cfg.data, bail: cfg.bail, persist: cfg.persist }));
    if (!r) return;
    run = { id: r.id, executions: [], done: false, summary: null, total: 0, iterations: 1, error: null };
    drawForm();
    drawResults();
    timer = setInterval(poll, 250);
    poll();
  }

  tab.preset = (collectionId, folderId) => { if (!(run && !run.done)) { cfg.collectionId = collectionId; cfg.folder = folderId || ""; drawForm(); } };
  tab.onShow = () => { if (!(run && !run.done)) drawForm(); };
  tab.onClose = () => { if (timer) clearInterval(timer); };
  drawForm();
  drawResults();
  return h("div.page", { testid: "runner" }, h("h2", "Collection runner"), h("div.sub", "Run every request of a collection or folder in order, with scripts, tests and optional iteration data."), form, results);
};

// ---------------------------------------------------------------- mock servers

export function openMocks(collectionId = null) {
  const tab = openSingleton("mock", { name: "Mock servers" });
  if (collectionId) {
    if (tab.preset) tab.preset(collectionId);
    else tab.initial = collectionId;
  }
}

views.mock = (tab) => {
  const cfg = { collectionId: tab.initial || (state.collections[0] && state.collections[0].info._postman_id) || "", port: "", delay: "0" };
  let mocks = [];
  let timer = null;
  const form = h("div.card");
  const list = h("div", { testid: "mock-list" });
  const drawForm = () => clear(form,
    h("div.form-grid",
      h("label", { for: "mock-collection" }, "Collection"),
      h("select", { id: "mock-collection", testid: "mock-collection", onChange: (e) => { cfg.collectionId = e.target.value; } },
        state.collections.length ? state.collections.map((x) => h("option", { value: x.info._postman_id, selected: x.info._postman_id === cfg.collectionId }, x.info.name)) : h("option", { value: "" }, "No collections")),
      h("label", { for: "mock-port" }, "Port"),
      h("input", { id: "mock-port", type: "number", min: "0", max: "65535", placeholder: "automatic", value: cfg.port, testid: "mock-port", onInput: (e) => { cfg.port = e.target.value; } }),
      h("label", { for: "mock-delay" }, "Response delay (ms)"),
      h("input", { id: "mock-delay", type: "number", min: "0", value: cfg.delay, testid: "mock-delay", onInput: (e) => { cfg.delay = e.target.value; } })),
    h("div.page-actions", h("button.btn.primary", { testid: "mock-start", disabled: !state.collections.length, onClick: () => guard(async () => {
      await api.post("/mocks", { collection_id: cfg.collectionId, port: parseInt(cfg.port, 10) || 0, delay_ms: parseInt(cfg.delay, 10) || 0 });
      toast("Mock server started", "ok");
      await refresh();
    }) }, "Start mock server"), h("span.muted", "Replies with the saved examples of the collection. Listens on this machine only.")));
  const drawList = () => clear(list, mocks.length ? mocks.map((m) => h("div.card", { testid: "mock-card" },
    h("div.field-row", h("b", m.collection_name), h("code.mono", { testid: "mock-url" }, m.url), h("button.btn.small", { onClick: () => copyText(m.url) }, "Copy URL"), h("span.spacer"),
      h("button.btn.small.danger", { testid: "mock-stop", onClick: () => guard(async () => { await api.del("/mocks/" + m.id); await refresh(); }) }, "Stop")),
    h("table.data", h("thead", h("tr", h("th", "Method"), h("th", "Path"), h("th", "Request"), h("th", "Example"))),
      h("tbody", m.routes.map((r) => h("tr", h("td", r.method), h("td", r.path), h("td", { style: "font-family:inherit" }, r.request), h("td", { style: "font-family:inherit" }, `${r.code} · ${r.example}`))))),
    h("div.section-title", `Request log (${m.log.length})`),
    m.log.length ? h("pre.code", { style: "padding:0;max-height:160px;overflow:auto", testid: "mock-log" }, m.log.slice(-100).join("\n")) : h("p.hint", "No requests received yet. Use the header x-mock-response-code or x-mock-response-name to pick a specific example.")))
    : h("div.placeholder", "No mock servers are running."));
  const refresh = async () => {
    try { mocks = await api.get("/mocks"); } catch (e) { return; }
    drawList();
  };
  tab.preset = (id) => { cfg.collectionId = id; drawForm(); };
  tab.onShow = () => { drawForm(); refresh(); if (!timer) timer = setInterval(refresh, 1500); };
  tab.onClose = () => { if (timer) clearInterval(timer); };
  drawForm();
  return h("div.page", { testid: "mocks" }, h("h2", "Mock servers"), h("div.sub", "Simulate an API from the examples saved in a collection."), form, list);
};

// ---------------------------------------------------------------- WebSocket client

let wsCounter = 0;
export function openWebSocket() {
  wsCounter += 1;
  openTab({ kind: "ws", name: wsCounter > 1 ? `WebSocket ${wsCounter}` : "WebSocket" });
}

views.ws = (tab) => {
  const cfg = { url: "", headers: [], message: "" };
  let conn = null; // { id, open }
  let events = [];
  let timer = null;
  const log = h("div.ws-log", { testid: "ws-log", role: "log", "aria-label": "WebSocket messages" });
  const bar = h("div.field-row");
  const composer = h("div.field-row", { style: "align-items:flex-start" });
  const urlInput = h("input", { type: "text", placeholder: "ws://localhost:8080/socket — {{variables}} are supported", style: "flex:1;font-family:var(--mono)", "aria-label": "WebSocket URL", testid: "ws-url", spellcheck: "false", onInput: (e) => { cfg.url = e.target.value; }, onKeydown: (e) => { if (e.key === "Enter") connect(); } });
  const msgInput = h("textarea", { placeholder: "Message", style: "flex:1;min-height:60px", "aria-label": "Message", testid: "ws-message", spellcheck: "false", onInput: (e) => { cfg.message = e.target.value; },
    onKeydown: (e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); send(); } } });

  const drawBar = () => {
    const open = conn && conn.open;
    urlInput.disabled = !!open;
    clear(bar, urlInput,
      open ? h("button.btn.danger", { testid: "ws-disconnect", onClick: () => guard(() => api.post(`/ws/${conn.id}/close`)) }, "Disconnect")
        : h("button.btn.primary", { testid: "ws-connect", onClick: connect }, "Connect"),
      h("span.status-pill." + (open ? "s2" : "s4"), { testid: "ws-state" }, open ? "Connected" : "Disconnected"));
    clear(composer, msgInput, h("button.btn.primary", { testid: "ws-send", disabled: !open, title: "Send (Ctrl+Enter)", onClick: send }, "Send"));
  };
  const drawLog = () => {
    const atBottom = log.scrollTop + log.clientHeight >= log.scrollHeight - 30;
    clear(log, events.length ? events.map((e) => h("div.ws-msg." + e.kind, { testid: "ws-event" },
      h("span.dir", { title: e.kind }, e.kind === "sent" ? "↑" : e.kind === "message" || e.kind === "binary" ? "↓" : "·"),
      h("span.t", new Date(e.time).toLocaleTimeString()), h("span", e.data || e.kind))) : h("div.placeholder", "Messages appear here once connected."));
    if (atBottom) log.scrollTop = log.scrollHeight;
  };
  const poll = async () => {
    if (!conn) return;
    try {
      const r = await api.get(`/ws/${conn.id}?since=${events.length}`);
      events.push(...r.events);
      const was = conn.open;
      conn.open = r.open;
      if (r.events.length) drawLog();
      if (was !== r.open) drawBar();
      if (!r.open && timer) { clearInterval(timer); timer = null; }
    } catch (e) {
      if (timer) { clearInterval(timer); timer = null; }
    }
  };
  async function connect() {
    if (!cfg.url.trim()) return urlInput.focus();
    const r = await guard(() => api.post("/ws", { url: cfg.url.trim(), headers: cleanRows(cfg.headers) }));
    if (!r) return;
    if (conn) api.del("/ws/" + conn.id).catch(() => {});
    conn = { id: r.id, open: true };
    events = [];
    tab.name = cfg.url.replace(/^wss?:\/\//, "").slice(0, 28) || tab.name;
    emit("tabmeta");
    drawBar();
    drawLog();
    timer = setInterval(poll, 250);
    poll();
  }
  async function send() {
    if (!conn || !conn.open || !cfg.message) return;
    await guard(() => api.post(`/ws/${conn.id}/send`, { text: cfg.message }));
    poll();
  }
  tab.send = send;
  tab.onClose = () => { if (timer) clearInterval(timer); if (conn) api.del("/ws/" + conn.id).catch(() => {}); };
  drawBar();
  drawLog();
  return h("div.page", { testid: "ws" }, h("h2", "WebSocket"), h("div.sub", "Connect to a ws:// or wss:// endpoint, send messages and watch the replies."),
    bar,
    h("details", { style: "margin-bottom:10px" }, h("summary.muted", { style: "cursor:pointer" }, "Handshake headers"), kvEditor(cfg.headers, { testid: "ws-headers", maskSensitive: true })),
    log, h("div", { style: "height:10px" }), composer);
};

// ---------------------------------------------------------------- cookies

views.cookies = (tab) => {
  let cookies = [];
  const table = h("div", { testid: "cookie-list" });
  const urlInput = h("input", { type: "text", placeholder: "https://example.com", style: "width:260px", "aria-label": "Cookie URL", testid: "cookie-url" });
  const cookieInput = h("input", { type: "text", placeholder: "name=value; Path=/; Max-Age=3600", style: "flex:1", "aria-label": "Cookie", testid: "cookie-value" });
  const refresh = async () => {
    cookies = (await guard(() => api.get("/cookies"))) || [];
    const domains = [...new Set(cookies.map((c) => c.domain))].sort();
    clear(table, cookies.length ? domains.map((d) => h("div.card",
      h("div.field-row", h("b", d), h("span.spacer"), h("button.btn.small.danger", { onClick: () => guard(async () => { await api.del("/cookies?domain=" + encodeURIComponent(d)); await refresh(); }) }, "Remove all for this domain")),
      h("table.data", h("thead", h("tr", ["Name", "Value", "Path", "Expires", "Flags", ""].map((x) => h("th", x)))),
        h("tbody", cookies.filter((c) => c.domain === d).map((c) => h("tr", { testid: "cookie-row" }, h("td.k", c.name), h("td", c.value), h("td", c.path), h("td", c.expires ? new Date(c.expires).toLocaleString() : "Session"),
          h("td", [c.secure && "Secure", c.http_only && "HttpOnly", c.same_site && "SameSite=" + c.same_site].filter(Boolean).join(", ")),
          h("td", { style: "text-align:right" }, h("button.icon-btn", { "aria-label": `Delete cookie ${c.name}`, testid: "cookie-delete", onClick: () => guard(async () => { await api.del(`/cookies?domain=${encodeURIComponent(d)}&name=${encodeURIComponent(c.name)}`); await refresh(); }) }, "✕"))))))))
      : h("div.placeholder", "The cookie jar is empty.", h("br"), "Cookies set by responses are stored here and sent with matching requests."));
  };
  tab.onShow = refresh;
  return h("div.page", { testid: "cookies" }, h("h2", "Cookies"), h("div.sub", "Cookies are stored in the workspace and shared by the GUI and the CLI."),
    h("div.card", h("div.field-row", { style: "margin:0" }, urlInput, cookieInput,
      h("button.btn.primary", { testid: "cookie-add", onClick: () => guard(async () => {
        if (!urlInput.value.trim() || !cookieInput.value.trim()) return toast("Enter a URL and a cookie such as name=value", "error");
        await api.post("/cookies", { url: urlInput.value.trim(), cookie: cookieInput.value.trim() });
        cookieInput.value = "";
        await refresh();
      }) }, "Add cookie"),
      h("button.btn.danger", { testid: "cookie-clear", onClick: async () => {
        if (await confirmDialog("Remove all cookies?", "Every stored cookie will be deleted.", "Remove all")) guard(async () => { await api.del("/cookies"); await refresh(); });
      } }, "Clear all"))),
    table);
};

// ---------------------------------------------------------------- settings

views.settings = (tab) => {
  let s = { ...state.settings };
  const body = h("div");
  const FIELDS = [
    ["timeout_ms", "Request timeout (ms)", "number", "0 waits forever"],
    ["follow_redirects", "Follow redirects", "bool"],
    ["max_redirects", "Maximum redirects", "number"],
    ["verify_ssl", "Verify TLS certificates", "bool"],
    ["proxy", "Proxy URL", "text", "http://host:port or socks5://host:port — empty uses the system proxy"],
    ["no_system_proxy", "Ignore system proxy variables", "bool"],
    ["ca_cert", "CA certificate bundle (PEM path)", "text", ""],
    ["client_cert", "Client certificate + key (PEM path)", "text", ""],
    ["send_default_headers", "Send default headers (User-Agent, Accept)", "bool"],
    ["user_agent", "User-Agent", "text", ""],
    ["cookies", "Store and send cookies", "bool"],
    ["history_limit", "History entries to keep", "number"],
    ["history_body_limit", "Max response bytes stored per history entry", "number"],
    ["history_store_secrets", "Keep passwords, tokens and keys in the history (otherwise stored as <redacted>)", "bool"],
  ];
  const draw = () => clear(body, h("div.form-grid", { style: "grid-template-columns: 300px minmax(0, 460px)" }, FIELDS.flatMap(([key, label, kind, hint]) => {
    const id = "set-" + key;
    const input = kind === "bool"
      ? h("input", { id, type: "checkbox", checked: !!s[key], testid: id, style: "justify-self:start", onChange: (e) => { s[key] = e.target.checked; } })
      : h("input", { id, type: kind === "number" ? "number" : "text", min: kind === "number" ? "0" : undefined, value: s[key], placeholder: hint || "", testid: id, onInput: (e) => { s[key] = kind === "number" ? Math.max(0, parseInt(e.target.value, 10) || 0) : e.target.value; } });
    return [h("label", { for: id }, label), input];
  })));
  tab.save = () => guard(async () => {
    state.settings = await api.put("/settings", s);
    s = { ...state.settings };
    toast("Settings saved", "ok");
  });
  draw();
  return h("div.page", { testid: "settings" }, h("h2", "Settings"), h("div.sub", `Stored in ${state.workspace}/settings.json and shared with the CLI.`),
    h("div.card", body),
    h("div.page-actions", h("button.btn.primary", { testid: "settings-save", onClick: () => tab.save() }, "Save"),
      h("button.btn", { testid: "settings-reset", onClick: async () => {
        if (!(await confirmDialog("Reset settings?", "All settings return to their defaults.", "Reset"))) return;
        await guard(async () => {
          // an empty object deserialises to the defaults on the server
          state.settings = await api.put("/settings", {});
          s = { ...state.settings };
          draw();
          toast("Settings reset", "ok");
        });
      } }, "Reset to defaults")),
    h("div.section-title", "About"),
    h("p.muted", `rsPostman ${state.version} — local-first API client. Workspace: ${state.workspace}`));
};

// ---------------------------------------------------------------- registration

tools.push(
  { id: "runner", label: "Runner", icon: "▶", title: "Run a collection", action: () => openRunner() },
  { id: "mock", label: "Mocks", icon: "◎", title: "Mock servers", action: () => openMocks() },
  { id: "ws", label: "WebSocket", icon: "⇄", title: "New WebSocket connection", action: openWebSocket },
  { id: "cookies", label: "Cookies", icon: "◍", title: "Cookie manager", action: () => openSingleton("cookies", { name: "Cookies" }) },
  { id: "settings", label: "Settings", icon: "⚙", title: "Settings", action: () => openSingleton("settings", { name: "Settings" }) });

collectionActions.push((c, item) => {
  const cid = c.info._postman_id;
  if (item && !item.item) return [];
  const out = [{ label: item ? "Run folder" : "Run collection", action: () => openRunner(cid, item ? item.id : null) }];
  if (!item) {
    out.push(
      { label: "Mock collection", action: () => openMocks(cid) },
      { label: "View documentation", action: () => window.open(`/api/collections/${cid}/docs?format=html`, "_blank", "noopener") },
      { label: "Download documentation (Markdown)", action: () => guard(async () => download(`${c.info.name}.md`, await api.get(`/collections/${cid}/docs?format=md`), "text/markdown")) });
  }
  return out;
});
