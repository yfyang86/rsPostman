// Application shell: top bar, sidebar, tab bar and workspace.
import { api } from "./api.js";
import { state, on, emit, views, tools, collectionActions, loadAll, loadHistory, collectionById, saveCollection, setActiveEnv, activeTab, openTab, activateTab, closeTab, openNewRequest, openItem, reconcileTabs, requestTab, openSingleton } from "./state.js";
import { h, clear, toast, guard, menu, confirmDialog, promptDialog, methodClass, statusClass, uuid, download, clone } from "./util.js";
import { findItem, duplicateItem, walk, newRequest, normalizeRequest } from "./model.js";
import "./request.js";
import "./editors.js";
import "./tools.js";

const $ = (id) => document.getElementById(id);

// ---------------------------------------------------------------- theme

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  try { localStorage.setItem("rsp-theme", theme); } catch (e) {}
}
function initialTheme() {
  let t = null;
  try { t = localStorage.getItem("rsp-theme"); } catch (e) {}
  return t || (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
}

// ---------------------------------------------------------------- top bar

function renderTopbar() {
  clear($("toolbar"),
    h("button.btn", { testid: "new-request", title: "New request (Ctrl+N)", onClick: () => openNewRequest() }, "＋ ", h("span.txt", "New")),
    tools.map((t) => h("button.btn", { testid: "tool-" + t.id, title: t.title || t.label, onClick: t.action }, t.icon + " ", h("span.txt", t.label))));
  renderEnvbar();
}

function renderEnvbar() {
  const select = h("select", { testid: "env-select", "aria-label": "Active environment", onChange: (e) => guard(() => setActiveEnv(e.target.value || null)) },
    h("option", { value: "" }, "No environment"),
    state.environments.map((e) => h("option", { value: e.id, selected: e.id === state.activeEnv }, e.name)));
  clear($("envbar"),
    select,
    h("button.icon-btn", { title: "Edit active environment", "aria-label": "Edit active environment", testid: "env-edit", onClick: () => {
      if (state.activeEnv) openEnvironment(state.activeEnv);
      else { state.side = "environments"; renderSidebar(); }
    } }, "✎"),
    h("button.icon-btn", { title: "Toggle light / dark theme", "aria-label": "Toggle theme", testid: "theme-toggle", onClick: () => applyTheme(document.documentElement.dataset.theme === "dark" ? "light" : "dark") }, "◐"));
}

// ---------------------------------------------------------------- sidebar

export function openEnvironment(envId) {
  const existing = state.tabs.find((t) => t.kind === "environment" && t.envId === envId);
  if (existing) return activateTab(existing.id);
  const env = envId === "globals" ? state.globals : state.environments.find((e) => e.id === envId);
  if (!env) return;
  openTab({ kind: "environment", envId, name: envId === "globals" ? "Globals" : env.name, draft: clone(env), dirty: false });
}

export function openCollectionEditor(collectionId, itemId = null) {
  const existing = state.tabs.find((t) => t.kind === "collection" && t.collectionId === collectionId && t.itemId === itemId);
  if (existing) return activateTab(existing.id);
  const c = collectionById(collectionId);
  if (!c) return;
  const target = itemId ? findItem(c.item, itemId).item : c;
  openTab({ kind: "collection", collectionId, itemId, name: itemId ? target.name : c.info.name, dirty: false, section: "overview" });
}

async function createCollection() {
  const name = await promptDialog("New collection", "Collection name", "", "Create");
  if (!name) return;
  await guard(async () => {
    const c = await api.post("/collections", { name });
    state.collections.push(c);
    state.expanded.add(c.info._postman_id);
    emit("collections");
    toast(`Collection “${name}” created`, "ok");
  });
}

async function mutate(collectionId, fn) {
  const c = collectionById(collectionId);
  if (!c) return;
  const draft = clone(c);
  if (fn(draft) === false) return;
  await guard(async () => {
    await saveCollection(draft);
    reconcileTabs();
  });
}

function itemMenu(c, it, anchor) {
  const cid = c.info._postman_id;
  const isFolder = !!it.item;
  const extra = collectionActions.flatMap((fn) => fn(c, it));
  menu(anchor, [
    !isFolder && { label: "Open", action: () => openItem(cid, it.id) },
    isFolder && { label: "Add request", action: () => { state.expanded.add(it.id); openNewRequest(cid, it.id); } },
    isFolder && { label: "Add folder", action: () => addFolder(cid, it.id) },
    isFolder && { label: "Edit folder…", action: () => openCollectionEditor(cid, it.id) },
    ...extra,
    "-",
    { label: "Rename…", action: async () => {
      const name = await promptDialog("Rename", "Name", it.name);
      if (name) mutate(cid, (d) => { findItem(d.item, it.id).item.name = name; });
    } },
    { label: "Duplicate", action: () => mutate(cid, (d) => {
      const f = findItem(d.item, it.id);
      const copy = duplicateItem(f.item);
      copy.name += " Copy";
      f.list.splice(f.list.indexOf(f.item) + 1, 0, copy);
    }) },
    "-",
    { label: "Delete", danger: true, action: async () => {
      if (await confirmDialog(`Delete “${it.name}”?`, isFolder ? "The folder and everything inside it will be deleted." : "This request will be deleted from the collection.")) {
        mutate(cid, (d) => { const f = findItem(d.item, it.id); f.list.splice(f.list.indexOf(f.item), 1); });
      }
    } },
  ]);
}

async function addFolder(cid, parentId) {
  const name = await promptDialog("New folder", "Folder name", "", "Create");
  if (!name) return;
  const id = uuid();
  state.expanded.add(cid);
  if (parentId) state.expanded.add(parentId);
  mutate(cid, (d) => {
    const list = parentId ? findItem(d.item, parentId).item.item : d.item;
    list.push({ id, name, item: [] });
  });
}

function collectionMenu(c, anchor) {
  const cid = c.info._postman_id;
  const extra = collectionActions.flatMap((fn) => fn(c, null));
  menu(anchor, [
    { label: "Add request", action: () => { state.expanded.add(cid); openNewRequest(cid, null); } },
    { label: "Add folder", action: () => addFolder(cid, null) },
    { label: "Edit collection…", action: () => openCollectionEditor(cid) },
    ...extra,
    "-",
    { label: "Export", action: () => { window.location.href = `/api/collections/${cid}/export`; } },
    { label: "Duplicate", action: () => guard(async () => {
      const copy = await api.post(`/collections/${cid}/duplicate`);
      state.collections.push(copy);
      emit("collections");
    }) },
    "-",
    { label: "Delete", danger: true, action: async () => {
      if (await confirmDialog(`Delete collection “${c.info.name}”?`, "All requests, folders and examples in it will be deleted. This cannot be undone.")) {
        await guard(async () => {
          await api.del("/collections/" + cid);
          state.collections = state.collections.filter((x) => x !== c);
          emit("collections");
          reconcileTabs();
        });
      }
    } },
  ]);
}

let dragged = null; // {cid, id}

function dropHandlers(row, cid, target /* item or null for the collection root */) {
  const clearMarks = () => row.classList.remove("drop-into", "drop-before");
  row.addEventListener("dragover", (e) => {
    if (!dragged || dragged.cid !== cid || (target && dragged.id === target.id)) return;
    e.preventDefault();
    clearMarks();
    const r = row.getBoundingClientRect();
    const into = !target || (target.item && e.clientY > r.top + r.height / 3);
    row.classList.add(into ? "drop-into" : "drop-before");
  });
  row.addEventListener("dragleave", clearMarks);
  row.addEventListener("drop", (e) => {
    if (!dragged || dragged.cid !== cid) return;
    e.preventDefault();
    const into = row.classList.contains("drop-into");
    clearMarks();
    const moving = dragged.id;
    dragged = null;
    mutate(cid, (d) => {
      const src = findItem(d.item, moving);
      if (!src) return false;
      if (target) {
        // refuse to drop a folder into itself or one of its descendants
        const t = findItem(d.item, target.id);
        if (!t || t.parents.some((p) => p.id === moving) || target.id === moving) return false;
      }
      src.list.splice(src.list.indexOf(src.item), 1);
      if (!target) d.item.push(src.item);
      else {
        const t = findItem(d.item, target.id);
        if (into) { t.item.item.push(src.item); state.expanded.add(target.id); }
        else t.list.splice(t.list.indexOf(t.item), 0, src.item);
      }
    });
  });
}

function renderCollections(body) {
  const filter = state.filter.trim().toLowerCase();
  const list = h("div", { role: "tree", testid: "collection-tree" });
  const active = activeTab();
  for (const c of state.collections) {
    const cid = c.info._postman_id;
    const matches = (it) => !filter || it.name.toLowerCase().includes(filter) || (it.request && (it.request.url.raw || "").toLowerCase().includes(filter)) || (it.item || []).some(matches);
    if (filter && !c.info.name.toLowerCase().includes(filter) && !c.item.some(matches)) continue;
    const open = state.expanded.has(cid) || !!filter;
    const row = h("div.tree-row.collection", { role: "treeitem", tabindex: "0", "aria-expanded": String(open), testid: "collection-row", "data-name": c.info.name, style: "padding-left:6px",
      onClick: () => { open ? state.expanded.delete(cid) : state.expanded.add(cid); renderSidebar(); },
      onKeydown: (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.currentTarget.click(); } } },
      h("span.caret", open ? "▼" : "▶"), h("span.label", { title: c.info.name }, c.info.name),
      h("button.icon-btn.more", { "aria-label": `Actions for ${c.info.name}`, testid: "collection-menu", onClick: (e) => { e.stopPropagation(); collectionMenu(c, e.currentTarget); } }, "⋯"));
    dropHandlers(row, cid, null);
    list.append(row);
    if (!open) continue;
    if (!c.item.length) list.append(h("div.muted", { style: "padding:4px 8px 8px 30px;font-size:12px" }, "Empty collection — add a request from the ⋯ menu."));
    const draw = (items, depth) => {
      for (const it of items) {
        if (filter && !matches(it)) continue;
        const pad = `padding-left:${14 + depth * 14}px`;
        let r;
        if (it.item) {
          const fopen = state.expanded.has(it.id) || !!filter;
          r = h("div.tree-row.folder", { role: "treeitem", tabindex: "0", "aria-expanded": String(fopen), draggable: "true", testid: "folder-row", "data-name": it.name, style: pad,
            onClick: () => { fopen ? state.expanded.delete(it.id) : state.expanded.add(it.id); renderSidebar(); },
            onKeydown: (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.currentTarget.click(); } } },
            h("span.caret", fopen ? "▼" : "▶"), h("span.label", { title: it.name }, it.name),
            h("button.icon-btn.more", { "aria-label": `Actions for ${it.name}`, testid: "item-menu", onClick: (e) => { e.stopPropagation(); itemMenu(c, it, e.currentTarget); } }, "⋯"));
          list.append(r);
          if (fopen) draw(it.item, depth + 1);
        } else {
          const m = (it.request && it.request.method) || "GET";
          r = h("div.tree-row.request" + (active && active.itemId === it.id ? ".active" : ""), { role: "treeitem", tabindex: "0", draggable: "true", testid: "request-row", "data-name": it.name, style: pad,
            onClick: () => openItem(cid, it.id),
            onKeydown: (e) => { if (e.key === "Enter") openItem(cid, it.id); } },
            h("span.method." + methodClass(m), m === "DELETE" ? "DEL" : m.slice(0, 5)), h("span.label", { title: it.name }, it.name),
            h("button.icon-btn.more", { "aria-label": `Actions for ${it.name}`, testid: "item-menu", onClick: (e) => { e.stopPropagation(); itemMenu(c, it, e.currentTarget); } }, "⋯"));
          list.append(r);
        }
        r.addEventListener("dragstart", (e) => { dragged = { cid, id: it.id }; e.dataTransfer.effectAllowed = "move"; e.dataTransfer.setData("text/plain", it.name); });
        r.addEventListener("dragend", () => { dragged = null; });
        dropHandlers(r, cid, it);
      }
    };
    draw(c.item, 1);
  }
  const filterInput = h("input", { type: "search", placeholder: "Filter", value: state.filter, "aria-label": "Filter collections", testid: "tree-filter", onInput: (e) => {
    state.filter = e.target.value;
    renderSidebar();
    const again = document.querySelector('[data-testid="tree-filter"]');
    if (again) { again.focus(); again.setSelectionRange(again.value.length, again.value.length); }
  } });
  clear(body,
    h("div.side-tools", filterInput, h("button.btn.small", { testid: "new-collection", title: "New collection", onClick: createCollection }, "＋ Collection")),
    state.collections.length ? (list.children.length ? list : h("div.empty", "No matches for “" + state.filter + "”."))
      : h("div.empty", { testid: "collections-empty" }, h("b", "No collections yet"), "Create a collection or import a Postman, OpenAPI or HAR file."));
}

function renderHistory(body) {
  const list = h("div", { testid: "history-list" });
  let day = "";
  for (const e of state.history) {
    const d = new Date(e.timestamp);
    const label = d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
    if (label !== day) { day = label; list.append(h("div.hist-day", label)); }
    const m = e.method || "GET";
    list.append(h("div.tree-row.hist-row", { tabindex: "0", testid: "history-row", style: "padding-left:8px", title: `${e.url || ""}\n${d.toLocaleTimeString()}`,
      onClick: () => openHistory(e.id), onKeydown: (ev) => { if (ev.key === "Enter") openHistory(e.id); } },
      h("span.method." + methodClass(m), m === "DELETE" ? "DEL" : m.slice(0, 5)),
      h("span.label", (e.url || "").replace(/^https?:\/\//, "")),
      h("span.status", { style: `color:var(--${e.error ? "danger" : e.status >= 400 ? "warn" : "ok"})` }, e.error ? "ERR" : e.status || ""),
      h("button.icon-btn.more", { "aria-label": "Delete history entry", onClick: (ev) => { ev.stopPropagation(); guard(async () => { await api.del("/history/" + e.id); await loadHistory(); }); } }, "✕")));
  }
  clear(body,
    h("div.side-tools", h("span.muted", { style: "flex:1;align-self:center" }, `${state.history.length} request${state.history.length === 1 ? "" : "s"}`),
      h("button.btn.small", { testid: "history-clear", disabled: !state.history.length, onClick: async () => {
        if (await confirmDialog("Clear history?", "All recorded requests will be removed.", "Clear")) guard(async () => { await api.del("/history"); await loadHistory(); });
      } }, "Clear all")),
    state.history.length ? list : h("div.empty", h("b", "No history yet"), "Requests you send show up here."));
}

async function openHistory(id) {
  await guard(async () => {
    const e = await api.get("/history/" + id);
    const source = e.source || (e.exchange.request ? { ...newRequest(e.exchange.request.method, e.exchange.request.url), header: e.exchange.request.headers.map(([key, value]) => ({ key, value })) } : newRequest());
    const tab = requestTab({ name: e.name || `${source.method} ${(source.url.raw || source.url || "").toString().replace(/^https?:\/\//, "").slice(0, 40)}`, request: normalizeRequest(clone(source)), exchange: e.exchange, dirty: true });
    openTab(tab);
  });
}

function renderEnvironments(body) {
  const row = (id, name, count) => h("div.tree-row", { tabindex: "0", testid: "env-row", "data-name": name, style: "padding-left:10px",
    onClick: () => openEnvironment(id), onKeydown: (e) => { if (e.key === "Enter") openEnvironment(id); } },
    h("span.label", name, id === state.activeEnv ? h("span.muted", "  · active") : null), h("span.muted", { style: "font-size:11px" }, count),
    id === "globals" ? null : h("button.icon-btn.more", { "aria-label": `Actions for ${name}`, testid: "env-menu", onClick: (e) => {
      e.stopPropagation();
      menu(e.currentTarget, [
        { label: id === state.activeEnv ? "Deactivate" : "Set active", action: () => guard(() => setActiveEnv(id === state.activeEnv ? null : id)) },
        { label: "Export", action: () => { window.location.href = `/api/environments/${id}/export`; } },
        { label: "Duplicate", action: () => guard(async () => {
          const src = state.environments.find((x) => x.id === id);
          const copy = await api.post("/environments", { name: src.name + " Copy", values: src.values });
          state.environments.push(copy);
          emit("environments");
        }) },
        "-",
        { label: "Delete", danger: true, action: async () => {
          if (await confirmDialog(`Delete environment “${name}”?`, "Its variables will be removed.")) {
            await guard(async () => {
              await api.del("/environments/" + id);
              state.environments = state.environments.filter((x) => x.id !== id);
              if (state.activeEnv === id) state.activeEnv = null;
              emit("environments");
              reconcileTabs();
            });
          }
        } },
      ]);
    } }, "⋯"));
  clear(body,
    h("div.side-tools", h("span", { style: "flex:1" }), h("button.btn.small", { testid: "new-environment", onClick: async () => {
      const name = await promptDialog("New environment", "Environment name", "", "Create");
      if (!name) return;
      await guard(async () => {
        const e = await api.post("/environments", { name });
        state.environments.push(e);
        emit("environments");
        openEnvironment(e.id);
      });
    } }, "＋ Environment")),
    row("globals", "Globals", `${state.globals.values.length} vars`),
    state.environments.map((e) => row(e.id, e.name, `${e.values.length} vars`)),
    state.environments.length ? null : h("div.empty", "Environments hold sets of variables such as hosts and tokens, e.g. “dev” and “prod”."));
}

function renderSidebar() {
  const tabs = [["collections", "Collections"], ["history", "History"], ["environments", "Environments"]];
  clear($("side-tabs"), tabs.map(([id, label]) => h("button", { role: "tab", "aria-selected": String(state.side === id), testid: "side-" + id, onClick: () => {
    state.side = id;
    if (id === "history") guard(loadHistory);
    renderSidebar();
  } }, label)));
  const body = $("side-body");
  if (state.side === "collections") renderCollections(body);
  else if (state.side === "history") renderHistory(body);
  else renderEnvironments(body);
}

// ---------------------------------------------------------------- tabs + workspace

async function requestClose(tab) {
  if (tab.dirty && tab.kind !== "request") {
    if (!(await confirmDialog("Discard changes?", `“${tab.name}” has unsaved changes.`, "Discard"))) return;
  } else if (tab.dirty && tab.kind === "request" && tab.itemId) {
    if (!(await confirmDialog("Discard changes?", `“${tab.name}” has unsaved changes.`, "Discard"))) return;
  }
  closeTab(tab.id);
}

function tabIcon(t) {
  if (t.kind === "request") {
    const m = t.request.method;
    return h("span.method." + methodClass(m), { style: "width:auto" }, m === "DELETE" ? "DEL" : m.slice(0, 5));
  }
  return h("span.muted", { style: "font-size:11px" }, { collection: "▣", environment: "◈", runner: "▶", ws: "⇄", mock: "◎", cookies: "◍", settings: "⚙" }[t.kind] || "•");
}

function renderTabbar() {
  clear($("tabbar"),
    state.tabs.map((t) => h("div.tab" + (t.id === state.activeTabId ? ".active" : ""), { role: "tab", "aria-selected": String(t.id === state.activeTabId), testid: "tab", title: t.name,
      onClick: () => activateTab(t.id), onAuxclick: (e) => { if (e.button === 1) requestClose(t); } },
      tabIcon(t), h("span.title", t.name), t.dirty ? h("span.dot", { title: "Unsaved changes", testid: "tab-dirty" }) : null,
      h("button.icon-btn", { "aria-label": `Close ${t.name}`, testid: "tab-close", onClick: (e) => { e.stopPropagation(); requestClose(t); } }, "✕"))),
    h("button.icon-btn.new-tab", { title: "New request", "aria-label": "New request", testid: "tab-new", onClick: () => openNewRequest() }, "＋"));
}

function renderWorkspace() {
  const ws = $("workspace");
  const tab = activeTab();
  if (!tab) {
    clear(ws, h("div.welcome", { testid: "welcome" },
      h("img", { src: "icon.svg", width: 56, height: 56, alt: "" }),
      h("h2", "Send your first request"),
      h("p", "rsPostman keeps everything on this machine: collections, environments, history and cookies live in your workspace folder."),
      h("div.actions",
        h("button.btn.primary", { testid: "welcome-new", onClick: () => openNewRequest() }, "New request"),
        h("button.btn", { onClick: createCollection }, "New collection"),
        tools.filter((t) => t.id === "import").map((t) => h("button.btn", { onClick: t.action }, "Import…"))),
      h("p", { style: "margin-top:18px;font-size:12px" }, h("kbd", "Ctrl"), " + ", h("kbd", "Enter"), " send · ", h("kbd", "Ctrl"), " + ", h("kbd", "S"), " save · ", h("kbd", "Ctrl"), " + ", h("kbd", "N"), " new request")));
    return;
  }
  if (!tab.el) {
    const make = views[tab.kind];
    tab.el = make ? make(tab) : h("div.placeholder", `No view for “${tab.kind}”`);
  }
  if (ws.firstChild !== tab.el || ws.childNodes.length !== 1) ws.replaceChildren(tab.el);
  if (tab.onShow) tab.onShow();
}

function renderStatus() {
  const env = state.environments.find((e) => e.id === state.activeEnv);
  clear($("statusbar"),
    h("span", `rsPostman ${state.version}`),
    h("span.path", { title: "Workspace folder" }, `Workspace: ${state.workspace}`),
    h("span.spacer"),
    h("span", { testid: "status-env" }, env ? `Environment: ${env.name}` : "No environment"),
    h("span", `${state.collections.length} collection${state.collections.length === 1 ? "" : "s"}`));
}

on("collections", () => { renderSidebar(); renderStatus(); });
on("environments", () => { renderEnvbar(); renderStatus(); if (state.side === "environments") renderSidebar(); });
on("history", () => { if (state.side === "history") renderSidebar(); });
on("tabs", () => { renderTabbar(); renderWorkspace(); if (state.side === "collections") renderSidebar(); });
on("tabmeta", renderTabbar);

// ---------------------------------------------------------------- global keys, resizer

document.addEventListener("keydown", (e) => {
  const mod = e.ctrlKey || e.metaKey;
  if (!mod) return;
  const tab = activeTab();
  const key = e.key.toLowerCase();
  if (key === "n" && !e.shiftKey) { e.preventDefault(); openNewRequest(); }
  else if (key === "s" && tab && tab.save) { e.preventDefault(); tab.save(); }
  else if (e.key === "Enter" && tab && tab.send) { e.preventDefault(); tab.send(); }
  else if (key === "w" && tab && e.altKey) { e.preventDefault(); requestClose(tab); }
});

(function resizer() {
  const bar = $("resizer"), side = $("sidebar");
  try { const w = parseInt(localStorage.getItem("rsp-side"), 10); if (w) side.style.width = w + "px"; } catch (e) {}
  bar.addEventListener("mousedown", (e) => {
    e.preventDefault();
    const move = (ev) => { side.style.width = Math.max(180, Math.min(ev.clientX, window.innerWidth * 0.6)) + "px"; };
    const up = () => {
      document.removeEventListener("mousemove", move);
      document.removeEventListener("mouseup", up);
      try { localStorage.setItem("rsp-side", parseInt(side.style.width, 10)); } catch (e) {}
    };
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", up);
  });
})();

window.addEventListener("beforeunload", (e) => {
  if (state.tabs.some((t) => t.dirty && (t.kind !== "request" || t.itemId))) { e.preventDefault(); e.returnValue = ""; }
});

// ---------------------------------------------------------------- boot

applyTheme(initialTheme());
renderTopbar();
renderSidebar();
renderTabbar();
renderWorkspace();
loadAll().then(() => {
  if (state.collections.length === 1) state.expanded.add(state.collections[0].info._postman_id);
  renderTopbar();
  renderSidebar();
  renderWorkspace();
  document.body.dataset.ready = "1";
}).catch((e) => {
  clear($("workspace"), h("div.error-box", h("b", "Could not load the workspace"), e.message));
});

export { renderSidebar, openHistory, walk, download };
