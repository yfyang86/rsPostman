// Help Center: the user manual (docs/user-manual.md) and the scripting reference, shown
// in-app with topic navigation, search and screenshots. Works offline – everything is
// served by the local backend.
import { views, emit, openSingleton } from "./state.js";
import { h, clear, toast } from "./util.js";
import { render, slug } from "./md.js";

let docs = null; // { topics: [{ id, title, group, source, text }], anchors: Map(anchor -> topicId) }
let loading = null;

async function fetchText(path) {
  const res = await fetch(path);
  if (!res.ok) throw new Error(`Could not load ${path} (${res.status})`);
  return res.text();
}

function split(manual, scripting) {
  const topics = [];
  const anchors = new Map();
  // the manual: one topic per "## " heading; text before the first one is the introduction
  const parts = manual.split(/^(?=## )/m);
  const intro = parts[0].replace(/^# .*\n/, "").trim();
  if (intro) topics.push({ id: "welcome", title: "Welcome", group: "User manual", source: intro });
  for (const part of parts.slice(1)) {
    const title = part.slice(3, part.indexOf("\n")).trim();
    topics.push({ id: slug(title), title, group: "User manual", source: part });
  }
  // the scripting reference as one topic (its "## " headings become subsections)
  topics.push({ id: "scripting-reference", title: "Scripting reference", group: "Reference", source: scripting.replace(/^# .*\n/, "## Scripting reference\n").replace(/^## (?!Scripting reference)/gm, "### ") });
  for (const t of topics) {
    t.text = t.source.toLowerCase();
    anchors.set(t.id, t.id);
    for (const m of t.source.matchAll(/^#{2,4}\s+(.*)$/gm)) {
      const a = slug(m[1].replace(/[`*]/g, ""));
      if (!anchors.has(a)) anchors.set(a, t.id);
    }
  }
  return { topics, anchors };
}

function load() {
  if (docs) return Promise.resolve(docs);
  if (!loading) {
    loading = Promise.all([fetchText("/help/user-manual.md"), fetchText("/help/scripting.md")])
      .then(([manual, scripting]) => (docs = split(manual, scripting)))
      .finally(() => { loading = null; });
  }
  return loading;
}

/** Open the Help Center, optionally at a heading anchor such as "collection-runner". */
export function openHelp(anchor) {
  const tab = openSingleton("help", { name: "Help" });
  if (anchor && tab.go) tab.go(anchor);
  else if (anchor) tab.initial = anchor;
  return tab;
}

/** Small "?" button that opens the Help Center at a topic. */
export function helpButton(anchor, label = "Help for this page") {
  return h("button.icon-btn.help-btn", { type: "button", title: label, "aria-label": label, testid: "page-help", onClick: () => openHelp(anchor) }, "?");
}

views.help = (tab) => {
  const nav = h("nav.help-nav", { "aria-label": "Help topics", testid: "help-nav" });
  const content = h("article.help-content", { testid: "help-content", tabindex: "-1" });
  const search = h("input", { type: "search", placeholder: "Search the help", "aria-label": "Search the help", testid: "help-search", onInput: () => { query = search.value.trim().toLowerCase(); drawNav(); if (query) drawResults(); else show(current, null); } });
  let current = "welcome";
  let query = "";

  const ctx = {
    image: (src) => src.replace(/^(\.\.\/)?ui\/help\/img\//, "help/img/"),
    id: (text) => "help-" + slug(text),
    link: (href) => {
      if (/^https?:\/\//.test(href)) return { href, external: true };
      if (href.startsWith("#")) return { href, onClick: () => go(href.slice(1)) };
      if (/scripting\.md/i.test(href)) return { href: "#scripting-reference", onClick: () => go(href.includes("#") ? href.split("#")[1] : "scripting-reference") };
      return { href, external: true };
    },
  };

  const drawNav = () => {
    if (!docs) return;
    const matches = (t) => !query || t.text.includes(query);
    const groups = [...new Set(docs.topics.map((t) => t.group))];
    clear(nav, groups.map((g) => {
      const list = docs.topics.filter((t) => t.group === g && matches(t));
      return list.length ? [h("div.help-group", g), list.map((t) => h("button", { class: t.id === current && !query ? "active" : "", "aria-current": t.id === current && !query ? "page" : null, testid: "help-topic", "data-topic": t.id,
        onClick: () => { search.value = ""; query = ""; show(t.id, null); } }, t.title))] : null;
    }));
    if (query && !nav.children.length) nav.append(h("div.muted", { style: "padding:8px 12px" }, "No topic matches."));
  };

  const show = (topicId, anchor) => {
    const topic = docs.topics.find((t) => t.id === topicId) || docs.topics[0];
    current = topic.id;
    const i = docs.topics.indexOf(topic);
    const prev = docs.topics[i - 1], next = docs.topics[i + 1];
    clear(content, render(topic.source, ctx),
      h("div.help-pager",
        prev ? h("button.btn", { testid: "help-prev", onClick: () => show(prev.id, null) }, "← " + prev.title) : h("span"),
        next ? h("button.btn", { testid: "help-next", onClick: () => show(next.id, null) }, next.title + " →") : h("span")));
    drawNav();
    const target = anchor && content.querySelector("#" + CSS.escape("help-" + anchor));
    if (target && target.tagName !== "H2") target.scrollIntoView({ block: "start" });
    else content.scrollTop = 0;
    tab.name = "Help – " + topic.title;
    emit("tabmeta");
  };

  const drawResults = () => {
    const hits = [];
    for (const t of docs.topics) {
      // every paragraph-ish block that contains the query, with the nearest heading above it
      let heading = t.title, anchor = t.id;
      for (const block of t.source.split(/\n\s*\n/)) {
        const hm = /^#{2,4}\s+(.*)$/m.exec(block);
        if (hm) { heading = hm[1].replace(/[`*]/g, ""); anchor = slug(heading); }
        const plain = block.replace(/^#{1,4}\s+.*$/gm, "").replace(/!\[[^\]]*\]\([^)]*\)/g, "").replace(/\[([^\]]+)\]\([^)]*\)/g, "$1").replace(/[`*|>]/g, " ").replace(/\s+/g, " ").trim();
        const at = plain.toLowerCase().indexOf(query);
        if (at === -1 && !(hm && heading.toLowerCase().includes(query))) continue;
        const start = Math.max(0, at - 60);
        hits.push({ topic: t, heading, anchor, snippet: at === -1 ? plain.slice(0, 160) : (start ? "…" : "") + plain.slice(start, at + query.length + 100) + (plain.length > at + query.length + 100 ? "…" : "") });
        if (hits.length >= 40) break;
      }
      if (hits.length >= 40) break;
    }
    clear(content,
      h("h2", `Search results for “${search.value.trim()}”`),
      hits.length
        ? h("div", { testid: "help-results" }, hits.map((x) => h("button.help-hit", { testid: "help-hit", onClick: () => { search.value = ""; query = ""; show(x.topic.id, x.anchor); } },
            h("b", x.topic.title === x.heading ? x.heading : `${x.topic.title} › ${x.heading}`), h("span", x.snippet))))
        : h("p.muted", { testid: "help-no-results" }, "Nothing found. Try a shorter or different word."));
    content.scrollTop = 0;
  };

  const go = (anchor) => {
    if (!docs) { tab.initial = anchor; return; }
    const topicId = docs.anchors.get(anchor);
    if (!topicId) return toast(`No help topic “${anchor}”`, "error");
    search.value = "";
    query = "";
    show(topicId, anchor);
  };
  tab.go = go;

  clear(content, h("p.muted", "Loading the help…"));
  load().then(() => {
    if (tab.initial) go(tab.initial);
    else show(current, null);
    tab.initial = null;
  }).catch((e) => clear(content, h("div.error-box", { role: "alert" }, h("b", "The help could not be loaded"), e.message)));

  return h("div.help", { testid: "help" },
    h("aside.help-side", h("div.help-search", search), nav),
    content);
};
