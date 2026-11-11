// Small dependency-free SVG charts (column and line) with hover/focus tooltips and a table
// view. Used by the GUI and embedded verbatim into the standalone benchmark HTML report,
// so this file must not import anything.

export const CHART_CSS = `
.viz-root { --viz-surface: var(--panel, #fcfcfb); --viz-text: #0b0b0b; --viz-text-2: #52514e; --viz-muted: #898781; --viz-grid: #e1e0d9; --viz-axis: #c3c2b7; --viz-series-1: #2a78d6;
  position: relative; color: var(--viz-text); background: var(--viz-surface); border: 1px solid rgba(11,11,11,0.10); border-radius: 8px; padding: 12px 14px 8px; min-width: 0; }
@media (prefers-color-scheme: dark) {
  :root:where(:not([data-theme="light"])) .viz-root { --viz-surface: var(--panel, #1a1a19); --viz-text: #f0efec; --viz-text-2: #c3c2b7; --viz-grid: #2c2c2a; --viz-axis: #383835; --viz-series-1: #3987e5; border-color: rgba(255,255,255,0.10); }
}
:root[data-theme="dark"] .viz-root { --viz-surface: var(--panel, #1a1a19); --viz-text: #f0efec; --viz-text-2: #c3c2b7; --viz-grid: #2c2c2a; --viz-axis: #383835; --viz-series-1: #3987e5; border-color: rgba(255,255,255,0.10); }
.viz-title { font-weight: 600; font-size: 13px; }
.viz-sub { color: var(--viz-text-2); font-size: 12px; margin-bottom: 6px; }
.viz-root svg { display: block; width: 100%; height: auto; overflow: visible; }
.viz-root svg text { font: 11px system-ui, -apple-system, "Segoe UI", sans-serif; fill: var(--viz-muted); font-variant-numeric: tabular-nums; }
.viz-root svg text.viz-value { fill: var(--viz-text); font-weight: 600; }
.viz-grid { stroke: var(--viz-grid); stroke-width: 1; }
.viz-axis { stroke: var(--viz-axis); stroke-width: 1; }
.viz-bar { fill: var(--viz-series-1); }
.viz-line { fill: none; stroke: var(--viz-series-1); stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
.viz-area { fill: var(--viz-series-1); opacity: 0.1; }
.viz-dot { fill: var(--viz-series-1); stroke: var(--viz-surface); stroke-width: 2; }
.viz-cross { stroke: var(--viz-axis); stroke-width: 1; }
.viz-hit { fill: transparent; outline: none; }
.viz-hit:focus-visible { stroke: var(--viz-text-2); stroke-width: 1; }
.viz-dim .viz-bar:not(.viz-on) { opacity: 0.45; }
.viz-tip { position: absolute; pointer-events: none; background: var(--viz-text); color: var(--viz-surface); padding: 5px 9px; border-radius: 6px; font-size: 12px; white-space: nowrap; transform: translate(-50%, -100%); margin-top: -8px; z-index: 5; }
.viz-tip b { display: block; font-size: 13px; }
.viz-root details { margin-top: 6px; font-size: 12px; color: var(--viz-text-2); }
.viz-root details summary { cursor: pointer; }
.viz-root details table { border-collapse: collapse; margin-top: 6px; }
.viz-root details td, .viz-root details th { padding: 2px 14px 2px 0; text-align: right; font-variant-numeric: tabular-nums; }
.viz-root details th { font-weight: 600; color: var(--viz-text-2); }
.viz-root details td:first-child, .viz-root details th:first-child { text-align: left; }
.viz-empty { color: var(--viz-muted); padding: 18px 0; font-size: 12px; }
`;

const NS = "http://www.w3.org/2000/svg";

function ensureCss() {
  if (document.getElementById("viz-css")) return;
  const style = document.createElement("style");
  style.id = "viz-css";
  style.textContent = CHART_CSS;
  document.head.append(style);
}

function svgEl(tag, attrs, parent) {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs || {})) e.setAttribute(k, v);
  if (parent) parent.append(e);
  return e;
}

function htmlEl(tag, cls, text, parent) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined && text !== null) e.textContent = text;
  if (parent) parent.append(e);
  return e;
}

/** Clean axis ticks from 0 up to at least `max`. */
export function niceTicks(max, count = 4) {
  if (!(max > 0)) return [0, 1];
  const raw = max / count;
  const pow = Math.pow(10, Math.floor(Math.log10(raw)));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * pow).find((s) => s >= raw) || 10 * pow;
  const ticks = [];
  for (let v = 0; v < max + step * 0.999; v += step) ticks.push(Number(v.toPrecision(12)));
  return ticks;
}

export const compact = (v) => {
  const a = Math.abs(v);
  if (a >= 1e6) return (v / 1e6).toFixed(a >= 1e7 ? 0 : 1).replace(/\.0$/, "") + "M";
  if (a >= 1e4) return (v / 1e3).toFixed(a >= 1e5 ? 0 : 1).replace(/\.0$/, "") + "K";
  if (a >= 100 || Number.isInteger(v)) return String(Math.round(v)).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return a >= 10 ? v.toFixed(1).replace(/\.0$/, "") : v.toFixed(2).replace(/\.?0+$/, "");
};

function frame(el, opts) {
  ensureCss();
  el.replaceChildren();
  el.classList.add("viz-root");
  if (opts.testid) el.setAttribute("data-testid", opts.testid);
  htmlEl("div", "viz-title", opts.title, el);
  if (opts.subtitle) htmlEl("div", "viz-sub", opts.subtitle, el);
  return el;
}

function tooltip(el) {
  const tip = htmlEl("div", "viz-tip", null, el);
  tip.hidden = true;
  tip.setAttribute("role", "status");
  return {
    show(x, y, value, label) {
      tip.replaceChildren();
      htmlEl("b", null, value, tip);
      tip.append(document.createTextNode(label));
      tip.style.left = x + "px";
      tip.style.top = y + "px";
      tip.hidden = false;
    },
    hide() {
      tip.hidden = true;
    },
  };
}

function tableView(el, columns, rows) {
  const details = htmlEl("details", null, null, el);
  htmlEl("summary", null, "Show data", details);
  const table = htmlEl("table", null, null, details);
  const head = htmlEl("tr", null, null, htmlEl("thead", null, null, table));
  columns.forEach((c) => htmlEl("th", null, c, head));
  const body = htmlEl("tbody", null, null, table);
  for (const r of rows) {
    const tr = htmlEl("tr", null, null, body);
    r.forEach((c) => htmlEl("td", null, c, tr));
  }
}

function axes(svg, m, w, h, ticks, yMax, yFormat) {
  const y = (v) => m.top + h - (v / yMax) * h;
  for (const t of ticks) {
    if (t > 0) svgEl("line", { class: "viz-grid", x1: m.left, x2: m.left + w, y1: y(t), y2: y(t) }, svg);
    const label = svgEl("text", { x: m.left - 8, y: y(t) + 4, "text-anchor": "end" }, svg);
    label.textContent = yFormat(t);
  }
  svgEl("line", { class: "viz-axis", x1: m.left, x2: m.left + w, y1: y(0), y2: y(0) }, svg);
  return y;
}

/**
 * Column chart for one series.
 * opts: { title, subtitle, bars: [{ label, value, detail? }], valueLabel, categoryLabel, yFormat?, testid? }
 */
export function columnChart(el, opts) {
  frame(el, opts);
  const bars = opts.bars || [];
  if (!bars.length) return htmlEl("div", "viz-empty", "No data yet.", el);
  const yFormat = opts.yFormat || compact;
  const width = Math.max(320, Math.round(el.clientWidth || 640) - 30);
  const m = { top: 14, right: 12, bottom: 34, left: 46 };
  const w = width - m.left - m.right, h = 150;
  const svg = svgEl("svg", { viewBox: `0 0 ${width} ${h + m.top + m.bottom}`, role: "img", "aria-label": `${opts.title}. ${bars.map((b) => `${b.label}: ${yFormat(b.value)}`).join(", ")}` }, el);
  const ticks = niceTicks(Math.max(...bars.map((b) => b.value)));
  const yMax = ticks[ticks.length - 1];
  const y = axes(svg, m, w, h, ticks, yMax, yFormat);
  const band = w / bars.length;
  const bw = Math.max(2, Math.min(24, band - 2)); // capped width, 2px surface gap between neighbours
  const tip = tooltip(el);
  const every = Math.ceil(bars.length / Math.max(1, Math.floor(w / 64)));
  const peak = bars.reduce((best, b, i) => (b.value > bars[best].value ? i : best), 0);
  bars.forEach((b, i) => {
    const cx = m.left + band * i + band / 2;
    const top = y(b.value), base = y(0);
    const r = Math.min(4, bw / 2, Math.max(0, base - top));
    let mark = null;
    if (b.value > 0) {
      // rounded data end, square at the baseline
      mark = svgEl("path", { class: "viz-bar", d: `M${cx - bw / 2},${base} V${top + r} Q${cx - bw / 2},${top} ${cx - bw / 2 + r},${top} H${cx + bw / 2 - r} Q${cx + bw / 2},${top} ${cx + bw / 2},${top + r} V${base} Z` }, svg);
    }
    if (i % every === 0) {
      const t = svgEl("text", { x: cx, y: base + 15, "text-anchor": "middle" }, svg);
      t.textContent = b.label;
    }
    if (i === peak && b.value > 0) {
      // one selective direct label: the tallest column
      const t = svgEl("text", { class: "viz-value", x: cx, y: top - 5, "text-anchor": "middle" }, svg);
      t.textContent = yFormat(b.value);
    }
    const hit = svgEl("rect", { class: "viz-hit", x: cx - band / 2, y: m.top, width: band, height: h, tabindex: "0", role: "img", "aria-label": `${b.label}: ${yFormat(b.value)} ${opts.valueLabel || ""}` }, svg);
    const on = () => {
      svg.classList.add("viz-dim");
      if (mark) mark.classList.add("viz-on");
      const box = svg.getBoundingClientRect(), host = el.getBoundingClientRect();
      const scale = box.width / width;
      tip.show(box.left - host.left + cx * scale, box.top - host.top + Math.min(top, base - 4) * scale, `${yFormat(b.value)} ${opts.valueLabel || ""}`.trim(), b.detail || b.label);
    };
    const off = () => {
      svg.classList.remove("viz-dim");
      if (mark) mark.classList.remove("viz-on");
      tip.hide();
    };
    hit.addEventListener("pointerenter", on);
    hit.addEventListener("pointerleave", off);
    hit.addEventListener("focus", on);
    hit.addEventListener("blur", off);
  });
  if (opts.categoryLabel) {
    const t = svgEl("text", { x: m.left + w / 2, y: h + m.top + 31, "text-anchor": "middle" }, svg);
    t.textContent = opts.categoryLabel;
  }
  tableView(el, [opts.categoryLabel || "Category", opts.valueLabel || "Value"], bars.map((b) => [b.detail || b.label, yFormat(b.value)]));
  return svg;
}

/**
 * Line chart for one series over an ordered x.
 * opts: { title, subtitle, points: [{ x, y, label }], valueLabel, xLabel, yFormat?, testid? }
 * `label` is the tooltip text of a point (e.g. "second 12").
 */
export function lineChart(el, opts) {
  frame(el, opts);
  const pts = opts.points || [];
  if (pts.length < 2) return htmlEl("div", "viz-empty", pts.length ? "Not enough data for a trend yet." : "No data yet.", el);
  const yFormat = opts.yFormat || compact;
  const width = Math.max(320, Math.round(el.clientWidth || 640) - 30);
  const m = { top: 14, right: 46, bottom: 34, left: 46 };
  const w = width - m.left - m.right, h = 150;
  const svg = svgEl("svg", { viewBox: `0 0 ${width} ${h + m.top + m.bottom}`, role: "img", "aria-label": `${opts.title}. ${pts.length} points, from ${yFormat(pts[0].y)} to ${yFormat(pts[pts.length - 1].y)} ${opts.valueLabel || ""}` }, el);
  const ticks = niceTicks(Math.max(...pts.map((p) => p.y)));
  const yMax = ticks[ticks.length - 1];
  const y = axes(svg, m, w, h, ticks, yMax, yFormat);
  const x0 = pts[0].x, x1 = pts[pts.length - 1].x;
  const x = (v) => m.left + (x1 === x0 ? w / 2 : ((v - x0) / (x1 - x0)) * w);
  const path = pts.map((p, i) => `${i ? "L" : "M"}${x(p.x).toFixed(1)},${y(p.y).toFixed(1)}`).join(" ");
  svgEl("path", { class: "viz-area", d: `${path} L${x(x1).toFixed(1)},${y(0)} L${x(x0).toFixed(1)},${y(0)} Z` }, svg);
  svgEl("path", { class: "viz-line", d: path }, svg);
  // x ticks: a handful of clean positions
  const step = Math.max(1, Math.ceil(pts.length / Math.max(1, Math.floor(w / 70))));
  pts.forEach((p, i) => {
    if (i % step === 0 || i === pts.length - 1) {
      if (i !== pts.length - 1 && pts.length - 1 - i < step / 2) return;
      const t = svgEl("text", { x: x(p.x), y: y(0) + 15, "text-anchor": "middle" }, svg);
      t.textContent = String(p.x);
    }
  });
  if (opts.xLabel) {
    const t = svgEl("text", { x: m.left + w / 2, y: h + m.top + 31, "text-anchor": "middle" }, svg);
    t.textContent = opts.xLabel;
  }
  // end marker with its value: the one direct label
  const last = pts[pts.length - 1];
  svgEl("circle", { class: "viz-dot", cx: x(last.x), cy: y(last.y), r: 4 }, svg);
  const endLabel = svgEl("text", { class: "viz-value", x: x(last.x) + 8, y: y(last.y) + 4 }, svg);
  endLabel.textContent = yFormat(last.y);

  const cross = svgEl("line", { class: "viz-cross", y1: m.top, y2: m.top + h, visibility: "hidden" }, svg);
  const dot = svgEl("circle", { class: "viz-dot", r: 4, visibility: "hidden" }, svg);
  const tip = tooltip(el);
  const hit = svgEl("rect", { class: "viz-hit", x: m.left, y: m.top, width: w, height: h, tabindex: "0", "aria-label": `${opts.title}: use the arrow keys to read values` }, svg);
  let current = pts.length - 1;
  const show = (i) => {
    current = Math.max(0, Math.min(pts.length - 1, i));
    const p = pts[current];
    cross.setAttribute("x1", x(p.x));
    cross.setAttribute("x2", x(p.x));
    cross.setAttribute("visibility", "visible");
    dot.setAttribute("cx", x(p.x));
    dot.setAttribute("cy", y(p.y));
    dot.setAttribute("visibility", "visible");
    const box = svg.getBoundingClientRect(), host = el.getBoundingClientRect();
    const scale = box.width / width;
    tip.show(box.left - host.left + x(p.x) * scale, box.top - host.top + y(p.y) * scale, `${yFormat(p.y)} ${opts.valueLabel || ""}`.trim(), p.label || String(p.x));
  };
  const hide = () => {
    cross.setAttribute("visibility", "hidden");
    dot.setAttribute("visibility", "hidden");
    tip.hide();
  };
  hit.addEventListener("pointermove", (e) => {
    const box = svg.getBoundingClientRect();
    const px = ((e.clientX - box.left) / box.width) * width;
    let best = 0;
    pts.forEach((p, i) => { if (Math.abs(x(p.x) - px) < Math.abs(x(pts[best].x) - px)) best = i; });
    show(best);
  });
  hit.addEventListener("pointerleave", hide);
  hit.addEventListener("focus", () => show(current));
  hit.addEventListener("blur", hide);
  hit.addEventListener("keydown", (e) => {
    if (e.key === "ArrowLeft") { e.preventDefault(); show(current - 1); }
    if (e.key === "ArrowRight") { e.preventDefault(); show(current + 1); }
  });
  tableView(el, [opts.xLabel || "x", opts.valueLabel || "Value"], pts.map((p) => [p.label || String(p.x), yFormat(p.y)]));
  return svg;
}

/** Histogram bucket label, e.g. "≤1", "5–10", ">30,000" (milliseconds). */
export const bucketLabel = (b) => (b.to_ms === null || b.to_ms === undefined ? `>${compact(b.from_ms)}` : b.from_ms === 0 ? `≤${compact(b.to_ms)}` : `${compact(b.from_ms)}–${compact(b.to_ms)}`);

/** Draw the two standard benchmark charts into `histEl` and `timeEl` from a report. */
export function benchCharts(histEl, timeEl, report) {
  columnChart(histEl, {
    testid: "chart-histogram",
    title: "Latency distribution",
    subtitle: "Answered requests per latency range",
    valueLabel: "requests",
    categoryLabel: "Latency (ms)",
    bars: (report.histogram || []).map((b) => ({ label: bucketLabel(b), value: b.count, detail: `${bucketLabel(b)} ms` })),
  });
  const ticks = report.timeline || [];
  lineChart(timeEl, {
    testid: "chart-timeline",
    title: "Mean latency over time",
    subtitle: "Mean latency of the requests completed in each second (ms)",
    valueLabel: "ms",
    xLabel: "Seconds since start",
    points: ticks.filter((t) => t.requests > t.failures || t.mean_ms > 0).map((t) => ({ x: t.second, y: t.mean_ms, label: `second ${t.second} · ${t.requests} request${t.requests === 1 ? "" : "s"}${t.failures ? `, ${t.failures} failed` : ""}` })),
  });
}
