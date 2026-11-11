// Application state, event bus and actions shared by all views.
import { api } from "./api.js";
import { findItem, normalizeRequest, newRequest, scriptOf } from "./model.js";
import { clone, uuid } from "./util.js";

export const state = {
  version: "",
  workspace: "",
  collections: [],
  environments: [],
  globals: { values: [] },
  activeEnv: null,
  settings: {},
  languages: [],
  vault: { exists: false, unlocked: false, names: [] },
  tabs: [],
  activeTabId: null,
  side: "collections",
  history: [],
  expanded: new Set(),
  filter: "",
};

const listeners = {};
export function on(evt, fn) {
  (listeners[evt] = listeners[evt] || []).push(fn);
}
export function emit(evt, ...args) {
  for (const fn of listeners[evt] || []) fn(...args);
}

// Extension points filled by feature modules.
export const views = {}; // kind -> (tab) => HTMLElement
export const tools = []; // toolbar buttons: {id, label, icon, action}
export const collectionActions = []; // (collection, item|null) => menu items
export const requestActions = []; // (tab) => buttons shown next to Save

export async function loadAll() {
  const b = await api.get("/bootstrap");
  Object.assign(state, {
    version: b.version,
    workspace: b.workspace,
    collections: b.collections,
    environments: b.environments,
    globals: b.globals,
    activeEnv: b.state.active_environment || null,
    settings: b.settings,
    languages: b.languages,
    vault: b.vault || state.vault,
  });
  emit("collections");
  emit("environments");
}

/** Reload data that scripts may have changed (variables), without touching open tabs. */
export async function refreshData() {
  const b = await api.get("/bootstrap");
  state.collections = b.collections;
  state.environments = b.environments;
  state.globals = b.globals;
  state.activeEnv = b.state.active_environment || null;
  emit("collections");
  emit("environments");
}

export async function loadHistory() {
  state.history = await api.get("/history?limit=300");
  emit("history");
}

export const collectionById = (id) => state.collections.find((c) => c.info._postman_id === id);
export const envById = (id) => state.environments.find((e) => e.id === id);

export async function saveCollection(c) {
  const saved = await api.put("/collections/" + c.info._postman_id, c);
  const i = state.collections.findIndex((x) => x.info._postman_id === saved.info._postman_id);
  if (i === -1) state.collections.push(saved);
  else state.collections[i] = saved;
  state.collections.sort((a, b) => a.info.name.toLowerCase().localeCompare(b.info.name.toLowerCase()));
  emit("collections");
  return saved;
}

export async function setActiveEnv(id) {
  await api.put("/state", { active_environment: id || null });
  state.activeEnv = id || null;
  emit("environments");
}

// ---------------------------------------------------------------- tabs

export const activeTab = () => state.tabs.find((t) => t.id === state.activeTabId) || null;

export function openTab(tab) {
  tab.id = tab.id || uuid();
  state.tabs.push(tab);
  state.activeTabId = tab.id;
  emit("tabs");
  return tab;
}

export function activateTab(id) {
  if (state.activeTabId === id) return;
  state.activeTabId = id;
  emit("tabs");
}

export function closeTab(id) {
  const i = state.tabs.findIndex((t) => t.id === id);
  if (i === -1) return;
  const [tab] = state.tabs.splice(i, 1);
  if (tab.onClose) tab.onClose();
  if (state.activeTabId === id) {
    const next = state.tabs[i] || state.tabs[i - 1];
    state.activeTabId = next ? next.id : null;
  }
  emit("tabs");
}

/** Open (or focus) a singleton tab such as a tool page. */
export function openSingleton(kind, make) {
  const existing = state.tabs.find((t) => t.kind === kind && (!make.key || t.key === make.key));
  if (existing) return activateTab(existing.id), existing;
  return openTab({ kind, ...make });
}

export function markDirty(tab, dirty = true) {
  if (tab.dirty !== dirty) {
    tab.dirty = dirty;
    emit("tabmeta");
  }
}

export function requestTab({ collectionId = null, itemId = null, name = "Untitled request", request, pre = "", test = "", dirty = false, exchange = null }) {
  return { kind: "request", collectionId, itemId, name, request: normalizeRequest(request || newRequest()), pre, test, dirty, exchange, sending: false, reqTab: "params", resTab: "body", bodyView: "pretty" };
}

export function openNewRequest(collectionId = null, parentId = null) {
  return openTab({ ...requestTab({ collectionId, dirty: false }), parentId });
}

export function openItem(collectionId, itemId) {
  const existing = state.tabs.find((t) => t.kind === "request" && t.itemId === itemId);
  if (existing) return activateTab(existing.id), existing;
  const c = collectionById(collectionId);
  const found = c && findItem(c.item, itemId);
  if (!found || !found.item.request) return null;
  const it = found.item;
  return openTab(requestTab({ collectionId, itemId, name: it.name, request: clone(it.request), pre: scriptOf(it.event, "prerequest"), test: scriptOf(it.event, "test") }));
}

/** Detach tabs whose item (or collection) no longer exists. */
export function reconcileTabs() {
  let changed = false;
  for (const t of [...state.tabs]) {
    if (t.kind === "request" && t.itemId) {
      const c = collectionById(t.collectionId);
      const found = c && findItem(c.item, t.itemId);
      if (!found) {
        t.itemId = null;
        t.parentId = null;
        t.collectionId = c ? t.collectionId : null;
        t.dirty = true;
        changed = true;
        if (t.onShow) t.onShow(); // refresh breadcrumbs: the tab is now an unsaved draft
      } else if (!t.dirty && found.item.name !== t.name) {
        t.name = found.item.name;
        changed = true;
        if (t.onRename) t.onRename();
      } else if (t.onShow) {
        t.onShow(); // parents may have been renamed or moved
      }
    }
    if (t.kind === "collection") {
      const c = collectionById(t.collectionId);
      if (!c || (t.itemId && !findItem(c.item, t.itemId))) closeTab(t.id);
    }
    if (t.kind === "environment" && t.envId !== "globals" && !envById(t.envId)) closeTab(t.id);
  }
  if (changed) emit("tabmeta");
}
