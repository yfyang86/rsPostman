// Helpers for the Postman-format request model used on both sides of the API.
import { uuid } from "./util.js";

export const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function newRequest(method = "GET", raw = "") {
  return { method, header: [], url: { raw, query: [], variable: [] }, body: null, auth: null, description: undefined };
}

/** Make sure optional parts exist so editors can bind to them. */
export function normalizeRequest(r) {
  r.method = r.method || "GET";
  r.header = r.header || [];
  if (typeof r.url === "string") r.url = { raw: r.url };
  r.url = r.url || { raw: "" };
  r.url.raw = r.url.raw || "";
  r.url.query = r.url.query || parseQuery(r.url.raw);
  r.url.variable = r.url.variable || [];
  return r;
}

export function parseQuery(raw) {
  const noHash = raw.split("#")[0];
  const i = noHash.indexOf("?");
  if (i === -1) return [];
  return noHash.slice(i + 1).split("&").filter(Boolean).map((p) => {
    const j = p.indexOf("=");
    return j === -1 ? { key: p, value: "" } : { key: p.slice(0, j), value: p.slice(j + 1) };
  });
}

export const urlBase = (raw) => raw.split("#")[0].split("?")[0];

/** The URL field changed: rebuild query rows (keeping disabled ones) and path variables. */
export function syncFromRaw(url) {
  const disabled = (url.query || []).filter((q) => q.disabled && (q.key || q.value));
  const old = url.query || [];
  const parsed = parseQuery(url.raw).map((q) => {
    const prev = old.find((o) => !o.disabled && o.key === q.key && o.description);
    return prev ? { ...q, description: prev.description } : q;
  });
  url.query = parsed.concat(disabled);
  syncPathVariables(url);
}

/** The params table changed: rebuild the raw URL from the enabled rows. */
export function syncFromQuery(url) {
  const q = (url.query || []).filter((k) => !k.disabled && (k.key || k.value)).map((k) => `${k.key}=${k.value}`);
  url.raw = urlBase(url.raw) + (q.length ? "?" + q.join("&") : "");
}

export function syncPathVariables(url) {
  const base = urlBase(url.raw);
  const afterScheme = base.includes("://") ? base.slice(base.indexOf("://") + 3) : base;
  const names = afterScheme.split("/").slice(1).filter((s) => s.startsWith(":") && s.length > 1).map((s) => s.slice(1));
  const old = url.variable || [];
  url.variable = names.map((n) => old.find((o) => o.key === n) || { key: n, value: "" });
}

// ---- auth: { type, [type]: [{key, value, type: "string"}] }  <->  plain map

export function authParams(auth) {
  const out = {};
  if (!auth) return out;
  const list = auth[auth.type];
  if (Array.isArray(list)) for (const e of list) out[e.key] = e.value === undefined || e.value === null ? "" : String(e.value);
  else if (list && typeof list === "object") for (const k of Object.keys(list)) out[k] = String(list[k]);
  return out;
}

export function makeAuth(type, params) {
  if (type === "inherit") return null;
  const auth = { type };
  const entries = Object.entries(params || {}).filter(([, v]) => v !== "" && v !== undefined && v !== null);
  if (entries.length) auth[type] = entries.map(([key, value]) => ({ key, value: String(value), type: "string" }));
  return auth;
}

// ---- scripts: event[] <-> text

export function scriptOf(events, listen) {
  return (events || []).filter((e) => e.listen === listen).map((e) => (Array.isArray(e.script && e.script.exec) ? e.script.exec.join("\n") : (e.script && e.script.exec) || "")).join("\n");
}

export function setScript(holder, listen, code) {
  holder.event = (holder.event || []).filter((e) => e.listen !== listen);
  if (code && code.trim()) holder.event.push({ listen, script: { type: "text/javascript", exec: code.split("\n") } });
  if (!holder.event.length) delete holder.event;
}

// ---- collection tree

export function findItem(items, id, parents = []) {
  for (const it of items || []) {
    if (it.id === id) return { item: it, parents, list: items };
    if (it.item) {
      const f = findItem(it.item, id, parents.concat(it));
      if (f) return f;
    }
  }
  return null;
}

export function walk(items, fn, depth = 0, parents = []) {
  for (const it of items || []) {
    fn(it, depth, parents);
    if (it.item) walk(it.item, fn, depth + 1, parents.concat(it));
  }
}

export function countRequests(items) {
  let n = 0;
  walk(items, (it) => { if (!it.item) n++; });
  return n;
}

/** Deep copy with fresh ids. */
export function duplicateItem(item) {
  const copy = JSON.parse(JSON.stringify(item));
  const renew = (it) => {
    it.id = uuid();
    (it.response || []).forEach((r) => (r.id = uuid()));
    (it.item || []).forEach(renew);
  };
  renew(copy);
  return copy;
}

export const folders = (collection) => {
  const out = [];
  walk(collection.item, (it, depth, parents) => { if (it.item) out.push({ id: it.id, label: parents.map((p) => p.name).concat(it.name).join(" / "), depth }); });
  return out;
};
