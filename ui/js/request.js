// Request tab: editor (params, auth, headers, body, scripts) and response viewer.
import { api } from "./api.js";
import { state, views, requestActions, emit, collectionById, saveCollection, markDirty, refreshData, loadHistory } from "./state.js";
import { h, clear, toast, guard, modal, kvEditor, cleanRows, tabStrip, highlightJson, prettyJson, prettyXml, humanSize, humanTime, statusClass, download, b64ToBytes, copyText, promptDialog, confirmDialog, uuid, clone, esc } from "./util.js";
import { METHODS, syncFromRaw, syncFromQuery, findItem, folders, setScript } from "./model.js";
import { authEditor } from "./authui.js";

const SNIPPETS = {
  pre: [
    ["Set an environment variable", 'pm.environment.set("variable_key", "variable_value");'],
    ["Set a collection variable", 'pm.collectionVariables.set("variable_key", "variable_value");'],
    ["Get a variable", 'const value = pm.variables.get("variable_key");'],
    ["Add a header", 'pm.request.headers.upsert({ key: "X-Request-Id", value: pm.variables.replaceIn("{{$guid}}") });'],
    ["Timestamp variable", 'pm.variables.set("now", new Date().toISOString());'],
    ["Send a request", 'pm.sendRequest("https://example.com/token", (err, res) => {\n  if (err) return console.error(err);\n  pm.environment.set("token", res.json().token);\n});'],
    ["Skip this request", "pm.execution.skipRequest();"],
  ],
  test: [
    ["Status code is 200", 'pm.test("Status code is 200", () => {\n  pm.response.to.have.status(200);\n});'],
    ["Response time below 500 ms", 'pm.test("Response time is below 500 ms", () => {\n  pm.expect(pm.response.responseTime).to.be.below(500);\n});'],
    ["JSON value check", 'pm.test("Body has the expected value", () => {\n  const json = pm.response.json();\n  pm.expect(json.value).to.eql(100);\n});'],
    ["Body contains string", 'pm.test("Body contains string", () => {\n  pm.expect(pm.response.text()).to.include("string_you_want_to_search");\n});'],
    ["Header is present", 'pm.test("Content-Type header is present", () => {\n  pm.response.to.have.header("Content-Type");\n});'],
    ["Validate JSON schema", 'const schema = { type: "object", required: ["id"], properties: { id: { type: "integer" } } };\npm.test("Schema is valid", () => {\n  pm.response.to.have.jsonSchema(schema);\n});'],
    ["Save a value for later requests", 'pm.environment.set("id", pm.response.json().id);'],
    ["Run another request next", 'pm.execution.setNextRequest("Request name");'],
  ],
};

const LANGS = [["json", "JSON"], ["text", "Text"], ["xml", "XML"], ["html", "HTML"], ["javascript", "JavaScript"]];

/** The request as sent to the server: blank editor rows removed, unused body parts dropped. */
export function cleanRequest(req) {
  const r = clone(req);
  r.header = cleanRows(r.header);
  r.url.query = cleanRows(r.url.query);
  r.url.variable = (r.url.variable || []).filter((v) => v.key);
  if (r.body) {
    const b = r.body;
    const mode = b.mode || "none";
    if (mode === "none") r.body = null;
    else if (mode === "raw") r.body = { mode, raw: b.raw || "", options: { raw: { language: (b.options && b.options.raw && b.options.raw.language) || "text" } } };
    else if (mode === "urlencoded") r.body = { mode, urlencoded: cleanRows(b.urlencoded) };
    else if (mode === "formdata") r.body = { mode, formdata: cleanRows(b.formdata).map((f) => (f.type === "file" ? { key: f.key, type: "file", src: f.src || "", disabled: f.disabled } : { key: f.key, value: f.value || "", type: "text", disabled: f.disabled })) };
    else if (mode === "file") r.body = { mode, file: { src: (b.file && b.file.src) || "" } };
    else if (mode === "graphql") r.body = { mode, graphql: { query: (b.graphql && b.graphql.query) || "", variables: (b.graphql && b.graphql.variables) || "" } };
  }
  if (!r.auth) delete r.auth;
  if (!r.description) delete r.description;
  return r;
}

function saveAsDialog(tab) {
  return new Promise((resolve) => {
    const name = h("input", { type: "text", value: tab.name === "Untitled request" ? "" : tab.name, placeholder: "Request name", testid: "save-name", autofocus: true });
    const NEW = "__new__";
    const collection = h("select", { testid: "save-collection" },
      state.collections.map((c) => h("option", { value: c.info._postman_id, selected: c.info._postman_id === tab.collectionId }, c.info.name)),
      h("option", { value: NEW }, "＋ New collection…"));
    const newName = h("input", { type: "text", placeholder: "New collection name", testid: "save-new-collection" });
    const folder = h("select", { testid: "save-folder" });
    const newWrap = h("div", h("label.block", "New collection name"), newName);
    const folderWrap = h("div", h("label.block", "Folder"), folder);
    const sync = () => {
      const isNew = collection.value === NEW || !state.collections.length;
      newWrap.classList.toggle("hidden", !isNew);
      folderWrap.classList.toggle("hidden", isNew);
      if (!isNew) {
        const c = collectionById(collection.value);
        clear(folder, h("option", { value: "" }, "(collection root)"), folders(c).map((f) => h("option", { value: f.id, selected: f.id === tab.parentId }, f.label)));
      }
    };
    if (!state.collections.length) collection.value = NEW;
    collection.addEventListener("change", sync);
    sync();
    modal({
      title: "Save request",
      testid: "save-dialog",
      body: [h("label.block", "Request name"), name, h("label.block", "Collection"), collection, newWrap, folderWrap],
      onClose: (v) => resolve(v || null),
      buttons: [
        { label: "Cancel", value: null },
        { label: "Save", primary: true, testid: "save-confirm", action: () => {
          if (!name.value.trim()) return name.focus(), false;
          const isNew = collection.value === NEW;
          if (isNew && !newName.value.trim()) return newName.focus(), false;
          return { name: name.value.trim(), collectionId: isNew ? null : collection.value, newCollection: isNew ? newName.value.trim() : null, folderId: isNew ? null : folder.value || null };
        } },
      ],
    });
  });
}

views.request = (tab) => {
  const req = tab.request;
  const dirty = () => markDirty(tab, true);

  // ------------------------------------------------------------ header
  const nameInput = h("input.name", { type: "text", value: tab.name, "aria-label": "Request name", testid: "request-name", onInput: (e) => { tab.name = e.target.value || "Untitled request"; tab.dirty = true; emit("tabmeta"); } });
  const crumbs = h("div.crumbs");
  const drawCrumbs = () => {
    const c = tab.collectionId && collectionById(tab.collectionId);
    const parts = [];
    if (c) {
      parts.push(c.info.name);
      const f = tab.itemId && findItem(c.item, tab.itemId);
      const parents = f ? f.parents : tab.parentId ? ((p) => (p ? p.parents.concat(p.item) : []))(findItem(c.item, tab.parentId)) : [];
      parts.push(...parents.map((p) => p.name));
    }
    clear(crumbs, parts.flatMap((p) => [h("span", p), h("span", "/")]), nameInput, !tab.itemId ? h("span.muted", { testid: "unsaved-hint" }, "· not saved to a collection") : null);
  };

  const methodSel = h("select", { "aria-label": "HTTP method", testid: "method", onChange: (e) => { req.method = e.target.value; dirty(); emit("tabmeta"); } },
    [...new Set(METHODS.concat(req.method))].map((m) => h("option", { value: m, selected: m === req.method }, m)));
  const urlInput = h("input", { type: "text", value: req.url.raw, placeholder: "Enter URL or paste a curl command — {{variables}} are supported", "aria-label": "Request URL", spellcheck: "false", autocomplete: "off", testid: "url",
    onInput: (e) => { req.url.raw = e.target.value; syncFromRaw(req.url); dirty(); drawSubtabs(); if (tab.reqTab === "params") drawEditor(); },
    onKeydown: (e) => { if (e.key === "Enter") { e.preventDefault(); tab.send(); } },
    onPaste: (e) => {
      const text = (e.clipboardData || window.clipboardData).getData("text");
      if (/^\s*curl(\.exe)?\s/.test(text)) {
        e.preventDefault();
        guard(async () => {
          const r = await api.post("/import", { text });
          if (r.kind !== "request") return;
          Object.assign(req, { header: [], body: null, auth: null }, r.request);
          req.url.query = req.url.query || [];
          req.url.variable = req.url.variable || [];
          urlInput.value = req.url.raw;
          methodSel.value = req.method;
          if (!METHODS.includes(req.method)) methodSel.append(h("option", { value: req.method, selected: true }, req.method));
          dirty(); emit("tabmeta"); drawSubtabs(); drawEditor();
          toast("curl command imported", "ok");
        });
      }
    } });
  const sendBtn = h("button.btn.primary", { testid: "send", onClick: () => tab.send() }, "Send");
  const saveBtn = h("button.btn", { testid: "save", title: "Save (Ctrl+S)", onClick: () => tab.save() }, "Save");

  const subtabs = h("div");
  const editor = h("div.req-body", { testid: "request-editor" });
  const resPane = h("div.res-pane", { testid: "response" });

  const enabledCount = (rows) => (rows || []).filter((r) => !r.disabled && (r.key || r.value)).length;
  const drawSubtabs = () => {
    const mode = (req.body && req.body.mode) || "none";
    clear(subtabs, tabStrip([
      { id: "params", label: "Params", count: enabledCount(req.url.query) },
      { id: "auth", label: "Authorization", badge: req.auth && req.auth.type !== "noauth" ? { cls: "", text: "●" } : null },
      { id: "headers", label: "Headers", count: enabledCount(req.header) },
      { id: "body", label: "Body", badge: mode !== "none" ? { cls: "", text: "●" } : null },
      { id: "pre", label: "Pre-request", badge: tab.pre.trim() ? { cls: "", text: "●" } : null },
      { id: "tests", label: "Tests", badge: tab.test.trim() ? { cls: "", text: "●" } : null },
      { id: "docs", label: "Docs" },
    ], tab.reqTab, (id) => { tab.reqTab = id; drawSubtabs(); drawEditor(); }, "req-tab"));
  };

  // ------------------------------------------------------------ editors
  const upload = async (file) => {
    const r = await guard(() => api.upload(file));
    return r ? r.path : null;
  };

  const scriptEditor = (key, placeholder, snippets) => {
    const ta = h("textarea", { spellcheck: "false", placeholder, testid: "script-" + key, "aria-label": placeholder, onInput: (e) => { tab[key] = e.target.value; dirty(); drawSubtabs(); },
      onKeydown: (e) => {
        if (e.key === "Tab" && !e.shiftKey && !e.ctrlKey && !e.altKey) {
          e.preventDefault();
          const s = ta.selectionStart;
          ta.setRangeText("  ", s, ta.selectionEnd, "end");
          ta.dispatchEvent(new Event("input"));
        }
      } });
    ta.value = tab[key];
    return h("div.script-wrap", ta, h("div.snippets", h("div.section-title", { style: "margin-top:0" }, "Snippets"),
      snippets.map(([label, code]) => h("button", { type: "button", onClick: () => {
        ta.value = (ta.value.trimEnd() ? ta.value.trimEnd() + "\n\n" : "") + code + "\n";
        ta.dispatchEvent(new Event("input"));
        ta.focus();
      } }, label))));
  };

  const bodyEditor = () => {
    if (!req.body) req.body = { mode: "none" };
    const b = req.body;
    b.mode = b.mode || "none";
    const content = h("div");
    const drawBody = () => {
      if (b.mode === "none") return clear(content, h("p.hint", "This request does not have a body."));
      if (b.mode === "urlencoded") { b.urlencoded = b.urlencoded || []; return clear(content, kvEditor(b.urlencoded, { onChange: dirty, description: true, testid: "body-urlencoded" })); }
      if (b.mode === "formdata") { b.formdata = b.formdata || []; return clear(content, kvEditor(b.formdata, { onChange: dirty, fileType: true, onUpload: upload, testid: "body-formdata" })); }
      if (b.mode === "file") {
        b.file = b.file || { src: "" };
        const path = h("input", { type: "text", value: b.file.src || "", placeholder: "/path/to/file", style: "flex:1", "aria-label": "File path", testid: "body-file", onInput: (e) => { b.file.src = e.target.value; dirty(); } });
        const pick = h("input", { type: "file", class: "hidden", onChange: async (e) => { const p = e.target.files[0] && (await upload(e.target.files[0])); if (p) { b.file.src = p; path.value = p; dirty(); } } });
        return clear(content, h("div.field-row", path, pick, h("button.btn", { onClick: () => pick.click() }, "Choose file…")), h("p.hint", "The file is read from this machine when the request is sent."));
      }
      if (b.mode === "graphql") {
        b.graphql = b.graphql || { query: "", variables: "" };
        const q = h("textarea", { spellcheck: "false", style: "width:100%;min-height:150px", placeholder: "query { ... }", testid: "body-graphql-query", "aria-label": "GraphQL query", onInput: (e) => { b.graphql.query = e.target.value; dirty(); } });
        q.value = b.graphql.query || "";
        const v = h("textarea", { spellcheck: "false", style: "width:100%;min-height:80px", placeholder: '{ "id": 1 }', testid: "body-graphql-vars", "aria-label": "GraphQL variables", onInput: (e) => { b.graphql.variables = e.target.value; dirty(); } });
        v.value = b.graphql.variables || "";
        return clear(content, h("div.section-title", { style: "margin-top:0" }, "Query"), q, h("div.section-title", "Variables (JSON)"), v);
      }
      // raw
      b.options = b.options || { raw: { language: "json" } };
      b.options.raw = b.options.raw || { language: "json" };
      const ta = h("textarea.editor", { spellcheck: "false", placeholder: "Request body", testid: "body-raw", "aria-label": "Raw body", onInput: (e) => { b.raw = e.target.value; dirty(); },
        onKeydown: (e) => { if (e.key === "Tab" && !e.shiftKey) { e.preventDefault(); ta.setRangeText("  ", ta.selectionStart, ta.selectionEnd, "end"); ta.dispatchEvent(new Event("input")); } } });
      ta.value = b.raw || "";
      clear(content,
        h("div.field-row",
          h("select", { "aria-label": "Body language", testid: "body-language", onChange: (e) => { b.options.raw.language = e.target.value; dirty(); } }, LANGS.map(([v, l]) => h("option", { value: v, selected: v === b.options.raw.language }, l))),
          h("button.btn.small", { testid: "body-beautify", onClick: () => {
            const p = b.options.raw.language === "xml" || b.options.raw.language === "html" ? prettyXml(ta.value) : prettyJson(ta.value);
            if (p === null) return toast("The body is not valid JSON", "error");
            ta.value = p; b.raw = p; dirty();
          } }, "Beautify")),
        ta);
    };
    drawBody();
    const modes = [["none", "none"], ["formdata", "form-data"], ["urlencoded", "x-www-form-urlencoded"], ["raw", "raw"], ["file", "binary"], ["graphql", "GraphQL"]];
    return h("div", { style: "height:100%" },
      h("div.radio-row", { role: "radiogroup", "aria-label": "Body type" }, modes.map(([v, l]) => h("label", h("input", { type: "radio", name: "body-mode-" + tab.id, value: v, checked: b.mode === v, testid: "body-mode-" + v,
        onChange: () => { b.mode = v; dirty(); drawSubtabs(); drawBody(); } }), l))),
      content);
  };

  const drawEditor = () => {
    const t = tab.reqTab;
    if (t === "params") {
      const parts = [h("div.section-title", { style: "margin-top:0" }, "Query params"), kvEditor(req.url.query, { description: true, testid: "params-table", onChange: () => { syncFromQuery(req.url); urlInput.value = req.url.raw; dirty(); drawSubtabs(); } })];
      if (req.url.variable.length) {
        parts.push(h("div.section-title", "Path variables"),
          h("table.kv", { testid: "path-vars" }, h("thead", h("tr", h("th", "Key"), h("th", "Value"), h("th", "Description"))),
            h("tbody", req.url.variable.map((v) => h("tr",
              h("td", h("input", { type: "text", value: v.key, readonly: true, "aria-label": "Path variable name" })),
              h("td", h("input", { type: "text", value: v.value || "", placeholder: "Value", "aria-label": `Value of ${v.key}`, onInput: (e) => { v.value = e.target.value; dirty(); } })),
              h("td", h("input", { type: "text", value: v.description || "", placeholder: "Description", "aria-label": "Description", onInput: (e) => { v.description = e.target.value || undefined; dirty(); } })))))));
      }
      clear(editor, parts);
    } else if (t === "auth") {
      clear(editor, authEditor(req, { allowInherit: true, collectionId: tab.collectionId, onChange: () => { dirty(); drawSubtabs(); } }));
    } else if (t === "headers") {
      clear(editor, kvEditor(req.header, { description: true, testid: "headers-table", onChange: () => { dirty(); drawSubtabs(); } }),
        h("p.hint", "User-Agent, Accept, Host, Content-Length and a matching Content-Type are added automatically."));
    } else if (t === "body") {
      clear(editor, bodyEditor());
    } else if (t === "pre") {
      clear(editor, scriptEditor("pre", "JavaScript that runs before the request is sent", SNIPPETS.pre));
    } else if (t === "tests") {
      clear(editor, scriptEditor("test", "JavaScript that runs after the response arrives", SNIPPETS.test));
    } else {
      const ta = h("textarea", { style: "width:100%;min-height:160px;font-family:inherit;font-size:13px", placeholder: "Describe what this request does (shown in generated documentation).", testid: "request-description", "aria-label": "Description", onInput: (e) => { req.description = e.target.value || undefined; dirty(); } });
      ta.value = req.description || "";
      clear(editor, ta);
    }
  };

  // ------------------------------------------------------------ response
  const currentItem = () => {
    const c = tab.collectionId && collectionById(tab.collectionId);
    const f = c && tab.itemId && findItem(c.item, tab.itemId);
    return f ? f.item : null;
  };

  const bodyView = (r) => {
    const wrap = h("div", { style: "height:100%;display:flex;flex-direction:column" });
    const content = h("div", { style: "flex:1;min-height:0;overflow:auto", testid: "response-body" });
    const ct = r.content_type || "";
    const kind = ct.includes("json") ? "json" : ct.includes("xml") ? "xml" : ct.includes("html") ? "html" : ct.startsWith("image/") ? "image" : "text";
    const draw = () => {
      if (!r.is_text && kind !== "image") {
        return clear(content, h("div.placeholder", h("div.big", "⬚"), `Binary response (${humanSize(r.size)}, ${ct || "unknown type"}).`, h("br"), "Use Download to save it."));
      }
      if (tab.bodyView === "preview") {
        if (kind === "image") return clear(content, h("div", { style: "padding:14px" }, h("img", { src: `data:${ct};base64,${r.body}`, alt: "Response image", style: "max-width:100%" })));
        if (kind === "html") return clear(content, h("iframe.preview-frame", { sandbox: "", srcdoc: r.body_text, title: "Response preview", testid: "response-preview" }));
        return clear(content, h("pre.code", r.body_text));
      }
      if (kind === "image") return clear(content, h("div.placeholder", "Image response — switch to Preview to see it."));
      if (!r.body_text) return clear(content, h("div.placeholder", "The response has an empty body."));
      if (tab.bodyView === "pretty" && r.body_text.length < 1500000) {
        const p = kind === "json" || /^\s*[\[{]/.test(r.body_text) ? prettyJson(r.body_text) : null;
        if (p !== null) return clear(content, h("pre.code", { html: highlightJson(p) }));
        if (kind === "xml" || kind === "html") return clear(content, h("pre.code", prettyXml(r.body_text)));
      }
      clear(content, h("pre.code", r.body_text));
    };
    const seg = h("div.seg", { role: "group", "aria-label": "Body view" }, ["pretty", "raw", "preview"].map((v) => h("button", { "aria-pressed": String(tab.bodyView === v), testid: "body-view-" + v, onClick: (e) => {
      tab.bodyView = v;
      seg.querySelectorAll("button").forEach((b) => b.setAttribute("aria-pressed", String(b === e.currentTarget)));
      draw();
    } }, v[0].toUpperCase() + v.slice(1))));
    draw();
    wrap.append(h("div.res-tools", seg, h("span.spacer"),
      h("button.btn.small", { disabled: !r.is_text, onClick: () => copyText(r.body_text) }, "Copy"),
      h("button.btn.small", { testid: "response-download", onClick: () => {
        const ext = r.is_text ? { json: "json", xml: "xml", html: "html", text: "txt" }[kind] || "txt" : (ct.split("/")[1] || "bin").split(/[;+]/)[0];
        download("response." + ext, new Blob([b64ToBytes(r.body)], { type: ct || "application/octet-stream" }));
      } }, "Download"),
      h("button.btn.small", { testid: "save-example", onClick: saveExample }, "Save as example")), content);
    return wrap;
  };

  async function saveExample() {
    const ex = tab.exchange;
    if (!tab.itemId) return toast("Save the request to a collection first, then save examples for it.", "error");
    if (!ex || !ex.response) return;
    const name = await promptDialog("Save as example", "Example name", `${ex.response.status} ${ex.response.status_text}`.trim());
    if (!name) return;
    await guard(async () => {
      const c = clone(collectionById(tab.collectionId));
      const it = findItem(c.item, tab.itemId).item;
      it.response = it.response || [];
      const ct = ex.response.content_type || "";
      it.response.push({
        id: uuid(), name, originalRequest: cleanRequest(req), status: ex.response.status_text, code: ex.response.status,
        header: ex.response.headers.filter(([k]) => !["content-length", "transfer-encoding", "content-encoding", "connection", "date"].includes(k.toLowerCase())).map(([key, value]) => ({ key, value })),
        body: ex.response.body_text || "", _postman_previewlanguage: ct.includes("json") ? "json" : ct.includes("xml") ? "xml" : ct.includes("html") ? "html" : "text",
      });
      await saveCollection(c);
      toast(`Example “${name}” saved`, "ok");
      drawResponse();
    });
  }

  const examplesView = () => {
    const it = currentItem();
    const list = (it && it.response) || [];
    if (!list.length) return h("div.placeholder", "No saved examples.", h("br"), "Examples document a request and are what the mock server replies with.");
    return h("table.data", { testid: "examples-table" }, h("thead", h("tr", h("th", "Name"), h("th", "Status"), h("th", ""))),
      h("tbody", list.map((e) => h("tr", h("td", { style: "font-family:inherit" }, e.name), h("td", `${e.code} ${e.status || ""}`),
        h("td", { style: "text-align:right;white-space:nowrap" },
          h("button.btn.small", { onClick: () => modal({ title: `Example: ${e.name}`, wide: true, body: h("pre.code", { style: "padding:0", html: prettyJson(e.body || "") !== null ? highlightJson(prettyJson(e.body)) : esc(e.body || "(empty body)") }), buttons: [{ label: "Close", primary: true }] }) }, "View"), " ",
          h("button.btn.small.danger", { testid: "example-delete", onClick: async () => {
            if (!(await confirmDialog(`Delete example “${e.name}”?`, "The mock server will no longer serve it."))) return;
            await guard(async () => {
              const c = clone(collectionById(tab.collectionId));
              const item = findItem(c.item, tab.itemId).item;
              item.response = item.response.filter((x) => x.id !== e.id);
              await saveCollection(c);
              drawResponse();
            });
          } }, "Delete"))))));
  };

  const drawResponse = () => {
    const ex = tab.exchange;
    if (tab.sending) return clear(resPane, h("div.placeholder", { testid: "response-loading" }, h("span.spinner"), "  Sending request…"));
    const it = currentItem();
    const exampleCount = it && it.response ? it.response.length : 0;
    if (!ex) {
      if (exampleCount) {
        tab.resTab = "examples";
        return clear(resPane, h("div.res-bar", tabStrip([{ id: "examples", label: "Examples", count: exampleCount }], "examples", () => {}, "res-tab")), h("div.res-body", examplesView()));
      }
      return clear(resPane, h("div.placeholder", { testid: "response-empty" }, h("div.big", "➤"), "Enter a URL and press ", h("kbd", "Send"), " to get a response."));
    }
    const r = ex.response;
    const tests = ex.tests || [];
    const failed = tests.filter((t) => !t.passed).length + (ex.script_error ? 1 : 0);
    const logs = ex.console || [];
    const tabsList = [
      { id: "body", label: "Body" },
      { id: "headers", label: "Headers", count: r ? r.headers.length : 0 },
      { id: "cookies", label: "Cookies", count: r ? r.cookies.length : 0 },
      { id: "tests", label: "Test results", badge: tests.length || ex.script_error ? { cls: failed ? "badge-bad" : "badge-ok", text: `(${tests.length - tests.filter((t) => !t.passed).length}/${tests.length})` } : null },
      { id: "console", label: "Console", count: logs.length },
      { id: "request", label: "Request" },
      { id: "examples", label: "Examples", count: exampleCount },
    ];
    if (!tabsList.some((t) => t.id === tab.resTab)) tab.resTab = "body";
    const meta = r
      ? h("div.res-meta", { testid: "response-meta" },
          h("span.status-pill." + statusClass(r.status), { testid: "response-status" }, `${r.status} ${r.status_text}`),
          h("span", { title: "Response time" }, h("b", humanTime(r.time_ms))),
          h("span", { title: `Body ${humanSize(r.size)}, headers ${humanSize(r.headers_size || 0)}` }, h("b", humanSize(r.size))),
          r.redirects && r.redirects.length ? h("span", { title: r.redirects.join("\n") }, `${r.redirects.length} redirect${r.redirects.length > 1 ? "s" : ""}`) : null)
      : h("div.res-meta", h("span.status-pill.err", { testid: "response-status" }, ex.skipped ? "Skipped" : "Error"));
    let body;
    const t = tab.resTab;
    if (t === "body") {
      if (ex.error) body = h("div.error-box", { testid: "response-error", role: "alert" }, h("b", "Could not get a response"), ex.error);
      else if (ex.skipped) body = h("div.placeholder", "The request was skipped by pm.execution.skipRequest().");
      else body = bodyView(r);
    } else if (t === "headers") {
      body = r ? h("table.data", { testid: "response-headers" }, h("thead", h("tr", h("th", "Name"), h("th", "Value"))), h("tbody", r.headers.map(([k, v]) => h("tr", h("td.k", k), h("td", v))))) : h("div.placeholder", "No response.");
    } else if (t === "cookies") {
      body = r && r.cookies.length
        ? h("table.data", { testid: "response-cookies" }, h("thead", h("tr", ["Name", "Value", "Domain", "Path", "Expires", "Flags"].map((x) => h("th", x)))),
            h("tbody", r.cookies.map((c) => h("tr", h("td.k", c.name), h("td", c.value), h("td", c.domain), h("td", c.path), h("td", c.expires ? new Date(c.expires).toLocaleString() : "Session"), h("td", [c.secure && "Secure", c.http_only && "HttpOnly"].filter(Boolean).join(", "))))))
        : h("div.placeholder", "The response did not set any cookies.");
    } else if (t === "tests") {
      body = tests.length || ex.script_error
        ? h("div", { testid: "test-results" },
            ex.script_error ? h("div.test-row.fail", h("span.tag", "ERROR"), h("div", "Script failed", h("div.err", ex.script_error))) : null,
            tests.map((x) => h("div.test-row." + (x.skipped ? "skip" : x.passed ? "pass" : "fail"), { testid: "test-row" }, h("span.tag", x.skipped ? "SKIP" : x.passed ? "PASS" : "FAIL"), h("div", x.name, x.error ? h("div.err", x.error) : null))))
        : h("div.placeholder", "No tests ran for this request.", h("br"), "Add assertions in the Tests tab, e.g. ", h("code", "pm.test(...)"), ".");
    } else if (t === "console") {
      body = logs.length ? h("div", { testid: "console" }, logs.map((l) => h("div.log-row." + l.level, h("span.lvl", l.level), h("span", l.message)))) : h("div.placeholder", "Nothing was logged. Use console.log() in scripts.");
    } else if (t === "request") {
      const p = ex.request;
      body = p
        ? h("div", { testid: "sent-request" }, h("pre.code", { style: "padding-top:12px" }, `${p.method} ${p.url}\n` + p.headers.map(([k, v]) => `${k}: ${v}`).join("\n") + (p.body_preview ? `\n\n${p.body_preview}` : "")))
        : h("div.placeholder", "The request was not sent.");
    } else {
      body = examplesView();
    }
    clear(resPane, h("div.res-bar", tabStrip(tabsList, tab.resTab, (id) => { tab.resTab = id; drawResponse(); }, "res-tab"), meta), h("div.res-body", body));
  };

  // ------------------------------------------------------------ actions
  tab.send = async () => {
    if (tab.sending) return;
    if (!req.url.raw.trim()) { toast("Enter a URL first", "error"); urlInput.focus(); return; }
    tab.sending = true;
    sendBtn.disabled = true;
    sendBtn.textContent = "Sending…";
    drawResponse();
    try {
      tab.exchange = await api.post("/send", { collection_id: tab.collectionId, item_id: tab.itemId, name: tab.name, request: cleanRequest(req), prerequest: tab.pre, test: tab.test });
      const ex = tab.exchange;
      if ((ex.tests || []).some((t) => !t.passed) || ex.script_error) tab.resTab = "tests";
      else if (!["body", "headers", "cookies", "tests", "console", "request"].includes(tab.resTab)) tab.resTab = "body";
      if (ex.error) tab.resTab = "body";
    } catch (e) {
      tab.exchange = { error: e.message, tests: [], console: [] };
      tab.resTab = "body";
    }
    tab.sending = false;
    sendBtn.disabled = false;
    sendBtn.textContent = "Send";
    drawResponse();
    guard(refreshData);
    if (state.side === "history") guard(loadHistory);
  };

  tab.save = async () => {
    await guard(async () => {
      let c, item;
      if (tab.itemId && collectionById(tab.collectionId) && findItem(collectionById(tab.collectionId).item, tab.itemId)) {
        c = clone(collectionById(tab.collectionId));
        item = findItem(c.item, tab.itemId).item;
      } else {
        const choice = await saveAsDialog(tab);
        if (!choice) return;
        if (choice.newCollection) c = await api.post("/collections", { name: choice.newCollection });
        else c = clone(collectionById(choice.collectionId));
        item = { id: uuid(), name: choice.name };
        const parent = choice.folderId && findItem(c.item, choice.folderId);
        (parent ? parent.item.item : c.item).push(item);
        tab.name = choice.name;
        tab.collectionId = c.info._postman_id;
        tab.itemId = item.id;
        tab.parentId = choice.folderId;
        state.expanded.add(tab.collectionId);
        if (choice.folderId) state.expanded.add(choice.folderId);
        nameInput.value = tab.name;
      }
      item.name = tab.name;
      item.request = cleanRequest(req);
      setScript(item, "prerequest", tab.pre);
      setScript(item, "test", tab.test);
      tab.dirty = false;
      await saveCollection(c);
      emit("tabmeta");
      emit("tabs");
      drawCrumbs();
      toast("Saved", "ok");
    });
  };

  tab.onShow = () => { drawCrumbs(); drawResponse(); };
  tab.onRename = () => { nameInput.value = tab.name; drawCrumbs(); };

  drawCrumbs();
  drawSubtabs();
  drawEditor();
  drawResponse();
  const extra = h("span", { style: "display:contents" }, requestActions.flatMap((fn) => fn(tab, () => cleanRequest(req))));
  return h("div.req-view", { testid: "request-view" },
    h("div.req-head", crumbs, h("div.url-row", h("div.url-box", methodSel, urlInput), sendBtn, saveBtn, extra)),
    subtabs,
    h("div.split", editor, resPane));
};
