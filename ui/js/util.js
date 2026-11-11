// DOM helpers and small reusable widgets.

/** Create an element: h("div.cls#id", {attrs / onX handlers}, ...children). */
export function h(spec, attrs, ...children) {
  const [tagAndId, ...classes] = spec.split(".");
  const [tag, id] = tagAndId.split("#");
  const el = document.createElement(tag || "div");
  if (id) el.id = id;
  if (classes.length) el.className = classes.join(" ");
  if (attrs && (typeof attrs !== "object" || attrs instanceof Node || Array.isArray(attrs))) {
    children.unshift(attrs);
    attrs = null;
  }
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === "class") el.className += (el.className ? " " : "") + v;
    else if (k === "value" || k === "checked" || k === "disabled" || k === "selected" || k === "open") el[k] = v;
    else if (k === "testid") el.setAttribute("data-testid", v);
    else if (k === "html") el.innerHTML = v;
    else el.setAttribute(k, v === true ? "" : v);
  }
  append(el, children);
  return el;
}

function append(el, children) {
  for (const c of children) {
    if (c === undefined || c === null || c === false) continue;
    if (Array.isArray(c)) append(el, c);
    else el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
}

export function clear(el, ...children) {
  el.replaceChildren();
  append(el, children);
  return el;
}

export const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

export function toast(message, kind = "") {
  const t = h("div.toast" + (kind ? "." + kind : ""), { role: kind === "error" ? "alert" : "status", testid: "toast" }, message);
  document.getElementById("toasts").append(t);
  setTimeout(() => t.remove(), kind === "error" ? 6000 : 2600);
}

/** Run an async action and surface failures as a toast. */
export async function guard(fn) {
  try {
    return await fn();
  } catch (e) {
    console.warn(e);
    toast(e.message || String(e), "error");
    return undefined;
  }
}

// ---------------------------------------------------------------- overlays

export function modal({ title, body, buttons = [], wide = false, testid = "modal", onClose }) {
  const root = document.getElementById("overlay-root");
  const previous = document.activeElement;
  let closed = false;
  const close = (value) => {
    if (closed) return;
    closed = true;
    backdrop.remove();
    document.removeEventListener("keydown", onKey, true);
    if (previous && previous.focus) previous.focus();
    if (onClose) onClose(value);
  };
  const onKey = (e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      close(undefined);
    }
  };
  const footer = h("footer");
  for (const b of buttons) {
    footer.append(
      h("button.btn" + (b.primary ? ".primary" : "") + (b.danger ? ".danger" : ""), {
        testid: b.testid,
        onClick: async () => {
          const result = b.action ? await b.action() : b.value;
          if (result !== false) close(result);
        },
      }, b.label)
    );
  }
  const box = h("div.modal" + (wide ? ".wide" : ""), { role: "dialog", "aria-modal": "true", "aria-label": title, testid },
    h("header", h("h3", title), h("button.icon-btn", { "aria-label": "Close", onClick: () => close(undefined) }, "✕")),
    h("div.body", body),
    buttons.length ? footer : null);
  const backdrop = h("div.backdrop", { onMousedown: (e) => e.target === backdrop && close(undefined) }, box);
  root.append(backdrop);
  document.addEventListener("keydown", onKey, true);
  const focusable = box.querySelector("[autofocus], .body input, .body textarea, .body select") || box.querySelector("footer .primary") || box.querySelector("button");
  if (focusable) focusable.focus();
  return { close, box };
}

export function confirmDialog(title, message, okLabel = "Delete") {
  return new Promise((resolve) => {
    modal({
      title,
      body: h("p", { style: "margin:6px 0" }, message),
      testid: "confirm",
      onClose: (v) => resolve(v === true),
      buttons: [{ label: "Cancel", value: false }, { label: okLabel, danger: true, value: true, testid: "confirm-ok" }],
    });
  });
}

export function promptDialog(title, label, value = "", okLabel = "Save") {
  return new Promise((resolve) => {
    const input = h("input", { type: "text", value, testid: "prompt-input", autofocus: true });
    const m = modal({
      title,
      body: [h("label.block", label), input],
      testid: "prompt",
      onClose: (v) => resolve(v === true ? input.value.trim() : null),
      buttons: [{ label: "Cancel", value: false }, { label: okLabel, primary: true, testid: "prompt-ok", action: () => (input.value.trim() ? true : (input.focus(), false)) }],
    });
    input.select();
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && input.value.trim()) m.close(true);
    });
  });
}

let openMenu = null;
export function closeMenu() {
  if (openMenu) openMenu.remove();
  openMenu = null;
}
/** Context menu anchored to an element. items: [{label, action, danger}] or "-" for a divider. */
export function menu(anchor, items) {
  closeMenu();
  const m = h("div.menu", { role: "menu", testid: "menu" });
  for (const it of items) {
    if (!it) continue;
    if (it === "-") m.append(h("hr"));
    else m.append(h("button", { role: "menuitem", class: it.danger ? "danger" : "", onClick: () => { closeMenu(); it.action(); } }, it.label));
  }
  document.body.append(m);
  const r = anchor.getBoundingClientRect();
  const w = m.offsetWidth, hgt = m.offsetHeight;
  m.style.left = Math.max(4, Math.min(r.left, window.innerWidth - w - 8)) + "px";
  m.style.top = (r.bottom + hgt + 8 > window.innerHeight ? Math.max(4, r.top - hgt - 2) : r.bottom + 2) + "px";
  openMenu = m;
  const first = m.querySelector("button");
  if (first) first.focus();
  m.addEventListener("keydown", (e) => {
    const btns = [...m.querySelectorAll("button")];
    const i = btns.indexOf(document.activeElement);
    if (e.key === "ArrowDown") { e.preventDefault(); btns[(i + 1) % btns.length].focus(); }
    if (e.key === "ArrowUp") { e.preventDefault(); btns[(i - 1 + btns.length) % btns.length].focus(); }
    if (e.key === "Escape") { closeMenu(); anchor.focus && anchor.focus(); }
  });
}
document.addEventListener("mousedown", (e) => {
  if (openMenu && !openMenu.contains(e.target)) closeMenu();
});
window.addEventListener("blur", closeMenu);

/** Tab strip. tabs: [{id, label, count, badge}] */
export function tabStrip(tabs, active, onSelect, testid) {
  return h("div.subtabs", { role: "tablist", testid },
    tabs.map((t) =>
      h("button", { role: "tab", "aria-selected": String(t.id === active), testid: `${testid}-${t.id}`, onClick: () => onSelect(t.id) },
        t.label, t.count ? h("span.count", `(${t.count})`) : null, t.badge ? h("span.count." + t.badge.cls, t.badge.text) : null)));
}

// ---------------------------------------------------------------- key/value editor

const isBlank = (r) => !r.key && !r.value && !(r.src && String(r.src).length);

/**
 * Editable key/value table bound to `rows` (mutated in place). A blank row is always kept at
 * the end; use `cleanRows` before saving.
 * opts: { onChange, description, fileType, onUpload, keyLabel, valueLabel, testid, enabledKey }
 */
export function kvEditor(rows, opts = {}) {
  const enabledOf = (r) => (opts.enabledKey ? r[opts.enabledKey] !== false : !r.disabled);
  const setEnabled = (r, on) => {
    if (opts.enabledKey) r[opts.enabledKey] = on;
    else if (on) delete r.disabled;
    else r.disabled = true;
  };
  const changed = () => opts.onChange && opts.onChange();
  const tbody = h("tbody");
  const ensureBlank = () => {
    if (!rows.length || !isBlank(rows[rows.length - 1])) {
      const r = { key: "", value: "" };
      if (opts.fileType) r.type = "text";
      rows.push(r);
      tbody.append(rowEl(r));
    }
  };
  const rowEl = (r) => {
    const tr = h("tr", { class: enabledOf(r) ? "" : "off" });
    const valueCell = h("td");
    const drawValue = () => {
      if (opts.fileType && r.type === "file") {
        const path = h("input", { type: "text", placeholder: "File path", value: Array.isArray(r.src) ? r.src[0] || "" : r.src || "", "aria-label": "File path", onInput: (e) => { r.src = e.target.value; changed(); ensureBlank(); } });
        const pick = h("input", { type: "file", class: "hidden", onChange: async (e) => {
          const f = e.target.files[0];
          if (!f || !opts.onUpload) return;
          const p = await opts.onUpload(f);
          if (p) { r.src = p; path.value = p; changed(); ensureBlank(); }
        } });
        clear(valueCell, h("div.file-cell", path, pick, h("button.btn.small", { type: "button", onClick: () => pick.click() }, "Choose…")));
      } else {
        clear(valueCell, h("input", { type: "text", placeholder: opts.valueLabel || "Value", value: r.value || "", "aria-label": opts.valueLabel || "Value", onInput: (e) => { r.value = e.target.value; changed(); ensureBlank(); } }));
      }
    };
    drawValue();
    append(tr, [
      h("td.c", h("input", { type: "checkbox", checked: enabledOf(r), "aria-label": "Enabled", onChange: (e) => { setEnabled(r, e.target.checked); tr.className = e.target.checked ? "" : "off"; changed(); } })),
      h("td", h("input", { type: "text", placeholder: opts.keyLabel || "Key", value: r.key || "", "aria-label": opts.keyLabel || "Key", onInput: (e) => { r.key = e.target.value; changed(); ensureBlank(); } })),
      opts.fileType ? h("td.t", h("select", { "aria-label": "Type", onChange: (e) => { r.type = e.target.value; if (r.type === "file") r.value = ""; else delete r.src; drawValue(); changed(); } },
        h("option", { value: "text", selected: r.type !== "file" }, "Text"), h("option", { value: "file", selected: r.type === "file" }, "File"))) : null,
      valueCell,
      opts.description ? h("td", h("input", { type: "text", placeholder: "Description", value: r.description || "", "aria-label": "Description", onInput: (e) => { r.description = e.target.value || undefined; changed(); } })) : null,
      h("td.x", h("button.icon-btn", { type: "button", title: "Remove row", "aria-label": "Remove row", onClick: () => {
        const i = rows.indexOf(r);
        if (i !== -1) rows.splice(i, 1);
        tr.remove();
        ensureBlank();
        changed();
      } }, "✕"))]);
    return tr;
  };
  for (const r of rows) tbody.append(rowEl(r));
  ensureBlank();
  return h("table.kv", { testid: opts.testid },
    h("thead", h("tr", h("th.c"), h("th", opts.keyLabel || "Key"), opts.fileType ? h("th.t", "Type") : null, h("th", opts.valueLabel || "Value"), opts.description ? h("th", "Description") : null, h("th.x"))),
    tbody);
}

/** Drop the trailing blank rows an editor adds. */
export function cleanRows(rows) {
  return (rows || []).filter((r) => !isBlank(r)).map((r) => {
    const o = { ...r };
    if (o.description === "" || o.description === undefined) delete o.description;
    return o;
  });
}

// ---------------------------------------------------------------- formatting

export function highlightJson(text) {
  return esc(text).replace(/(&quot;(?:\\.|(?!&quot;)[^\\])*&quot;)(\s*:)?|\b(true|false|null)\b|-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/g, (m, str, colon, lit) => {
    if (str) return colon ? `<span class="j-key">${str}</span>${colon}` : `<span class="j-str">${str}</span>`;
    if (lit) return `<span class="j-lit">${m}</span>`;
    return `<span class="j-num">${m}</span>`;
  });
}

export function prettyJson(text) {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch (e) {
    return null;
  }
}

/** Indent XML/HTML markup for display. */
export function prettyXml(text) {
  const tokens = text.replace(/>\s+</g, "><").split(/(?=<)|(?<=>)/).filter((t) => t.trim() !== "");
  let depth = 0;
  const out = [];
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (/^<\//.test(t)) depth = Math.max(0, depth - 1);
    // keep <a>text</a> on one line
    if (/^<[^!?\/][^>]*[^\/]>$/.test(t) && tokens[i + 1] && !tokens[i + 1].startsWith("<") && tokens[i + 2] && /^<\//.test(tokens[i + 2])) {
      out.push("  ".repeat(depth) + t + tokens[i + 1] + tokens[i + 2]);
      i += 2;
      continue;
    }
    out.push("  ".repeat(depth) + t);
    if (/^<[^!?\/][^>]*[^\/]>$/.test(t) && !/^<(br|hr|img|input|meta|link)\b/i.test(t)) depth++;
  }
  return out.join("\n");
}

export const humanSize = (n) => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(2)} KB` : `${(n / 1048576).toFixed(2)} MB`);
export const humanTime = (ms) => (ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`);
export const methodClass = (m) => (["GET", "POST", "PUT", "PATCH", "DELETE"].includes(m) ? m : "other");
export const statusClass = (s) => "s" + String(s).charAt(0);

export function download(filename, content, type = "application/octet-stream") {
  const blob = content instanceof Blob ? content : new Blob([content], { type });
  const a = h("a", { href: URL.createObjectURL(blob), download: filename });
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

export function b64ToBytes(b64) {
  const bin = atob(b64 || "");
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch (e) {
    const ta = h("textarea", { style: "position:fixed;opacity:0" });
    ta.value = text;
    document.body.append(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
  toast("Copied to clipboard");
}

export const uuid = () => (crypto.randomUUID ? crypto.randomUUID() : "id-" + Math.random().toString(16).slice(2) + Date.now().toString(16));
export const clone = (o) => JSON.parse(JSON.stringify(o));
