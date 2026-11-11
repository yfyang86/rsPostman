// Minimal Markdown renderer for the Help Center. Builds DOM nodes directly (no innerHTML),
// so the text of a document can never inject markup. Supports what the manual uses:
// headings, paragraphs, fenced code, lists (one nested level), tables, block quotes,
// images, links, inline code, bold and italic.

export const slug = (text) => text.toLowerCase().replace(/`/g, "").replace(/[^\p{L}\p{N}\s-]/gu, "").trim().replace(/\s+/g, "-");

const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

/** Inline markup -> nodes appended to `parent`. `ctx` = { link(href) -> {href, onClick?, external}, image(src) -> src } */
export function inline(parent, text, ctx) {
  const re = /(`[^`]+`)|(!\[([^\]]*)\]\(([^)\s]+)\))|(\[([^\]]+)\]\(([^)\s]+)\))|(\*\*([^*]+)\*\*)|(\*([^*\s][^*]*)\*)/g;
  let last = 0, m;
  while ((m = re.exec(text))) {
    if (m.index > last) parent.append(document.createTextNode(text.slice(last, m.index)));
    if (m[1]) parent.append(el("code", null, m[1].slice(1, -1)));
    else if (m[2]) {
      const img = el("img");
      img.alt = m[3];
      img.src = ctx.image(m[4]);
      img.loading = "lazy";
      parent.append(img);
    } else if (m[5] && /^\s*(javascript|data|vbscript):/i.test(m[7])) {
      // never produce a link that runs script; show the text only
      inline(parent, m[6], ctx);
    } else if (m[5]) {
      const a = el("a");
      inline(a, m[6], ctx);
      const target = ctx.link(m[7]);
      a.href = target.href;
      if (target.onClick) a.addEventListener("click", (e) => { e.preventDefault(); target.onClick(); });
      if (target.external) { a.target = "_blank"; a.rel = "noopener noreferrer"; }
      parent.append(a);
    } else if (m[8]) {
      const b = el("strong");
      inline(b, m[9], ctx);
      parent.append(b);
    } else if (m[10]) {
      const i = el("em");
      inline(i, m[11], ctx);
      parent.append(i);
    }
    last = re.lastIndex;
  }
  if (last < text.length) parent.append(document.createTextNode(text.slice(last)));
  return parent;
}

const isListItem = (line) => /^(\s*)([-*]|\d+\.)\s+/.exec(line);
const cells = (line) => line.trim().replace(/^\||\|$/g, "").split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, "|"));

/** Render Markdown `source` into a DocumentFragment. Headings get ids from `ctx.id(text, level)`. */
export function render(source, ctx) {
  const out = document.createDocumentFragment();
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) { i++; continue; }

    const fence = /^```(\w*)\s*$/.exec(line);
    if (fence) {
      const body = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i])) body.push(lines[i++]);
      i++;
      const pre = el("pre", "code");
      pre.append(el("code", null, body.join("\n")));
      if (fence[1]) pre.dataset.lang = fence[1];
      out.append(pre);
      continue;
    }

    const heading = /^(#{1,4})\s+(.*)$/.exec(line);
    if (heading) {
      const level = heading[1].length;
      const h = el("h" + level);
      inline(h, heading[2], ctx);
      h.id = ctx.id(heading[2].replace(/[`*]/g, ""), level);
      out.append(h);
      i++;
      continue;
    }

    if (/^---+\s*$/.test(line)) { out.append(el("hr")); i++; continue; }

    if (line.trim().startsWith("|") && i + 1 < lines.length && /^\s*\|?\s*:?-{2,}/.test(lines[i + 1])) {
      const table = el("table", "data");
      const head = el("tr");
      cells(line).forEach((c) => inline(head.appendChild(el("th")), c, ctx));
      table.appendChild(el("thead")).append(head);
      const body = table.appendChild(el("tbody"));
      i += 2;
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        const tr = body.appendChild(el("tr"));
        cells(lines[i]).forEach((c) => inline(tr.appendChild(el("td")), c, ctx));
        i++;
      }
      const wrap = el("div", "table-wrap");
      wrap.append(table);
      out.append(wrap);
      continue;
    }

    if (line.startsWith(">")) {
      const body = [];
      while (i < lines.length && lines[i].startsWith(">")) body.push(lines[i++].replace(/^>\s?/, ""));
      const note = el("div", "note");
      note.append(render(body.join("\n"), ctx));
      out.append(note);
      continue;
    }

    const item = isListItem(line);
    if (item) {
      const indent = item[1].length;
      const list = el(/\d/.test(item[2]) ? "ol" : "ul");
      while (i < lines.length) {
        const m = isListItem(lines[i]);
        if (!m || m[1].length !== indent) break;
        const li = list.appendChild(el("li"));
        const first = lines[i].slice(m[0].length);
        const rest = [];
        i++;
        // continuation lines and nested blocks: anything indented deeper than the marker
        while (i < lines.length && (/^\s+\S/.test(lines[i]) && lines[i].search(/\S/) > indent || (!lines[i].trim() && i + 1 < lines.length && lines[i + 1].search(/\S/) > indent))) {
          rest.push(lines[i++]);
        }
        const nestedStart = rest.findIndex((r) => isListItem(r) || /^\s*```/.test(r) || !r.trim());
        const textLines = nestedStart === -1 ? rest : rest.slice(0, nestedStart);
        inline(li, [first, ...textLines.map((t) => t.trim())].join(" "), ctx);
        if (nestedStart !== -1) {
          const block = rest.slice(nestedStart);
          const cut = Math.min(...block.filter((b) => b.trim()).map((b) => b.search(/\S/)));
          li.append(render(block.map((b) => b.slice(cut)).join("\n"), ctx));
        }
      }
      out.append(list);
      continue;
    }

    const image = /^!\[([^\]]*)\]\(([^)\s]+)\)\s*$/.exec(line.trim());
    if (image) {
      const fig = el("figure");
      const img = fig.appendChild(el("img"));
      img.alt = image[1];
      img.src = ctx.image(image[2]);
      img.loading = "lazy";
      if (image[1]) fig.append(el("figcaption", null, image[1]));
      out.append(fig);
      i++;
      continue;
    }

    const para = [];
    while (i < lines.length && lines[i].trim() && !/^(#{1,4}\s|```|>|\s*\|)/.test(lines[i]) && !isListItem(lines[i]) && !/^!\[[^\]]*\]\([^)]+\)\s*$/.test(lines[i].trim())) {
      para.push(lines[i++].trim());
    }
    out.append(inline(el("p"), para.join(" "), ctx));
  }
  return out;
}
