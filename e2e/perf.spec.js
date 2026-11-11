const { test, expect, ui } = require("./fixtures");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

/** Import a private copy of the example collection through the CLI and reload the page. */
async function importDemo(page, app, name) {
  const src = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "examples", "workflow.postman_collection.json"), "utf8"));
  src.info.name = name;
  src.info._postman_id = crypto.randomUUID();
  src.variable.find((v) => v.key === "base").value = app.echo;
  const file = path.join(app.home, `${name.replace(/\W/g, "_")}.json`);
  fs.writeFileSync(file, JSON.stringify(src));
  app.cli("import", file);
  await page.reload();
  await page.waitForSelector('body[data-ready="1"]');
}

test("profile LLM provider: healthy verdict, TTFT and token rate", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-profile").click();
  await u.t("profile-url").fill(`${app.echo}/llm`);
  await u.t("profile-model").fill("test-model");
  await u.t("profile-iterations").fill("2");
  await u.t("profile-start").click();
  await expect(u.t("profile-verdict")).toHaveText("healthy", { timeout: 15000 });
  await expect(u.t("profile-stats")).toContainText("test-model");
  // both runs listed, and the aggregate table carries the percentile columns
  await expect(u.t("profile-run")).toHaveCount(2);
  await expect(u.t("stats-table")).toContainText("P10");
  await expect(u.t("stats-table")).toContainText("P99");
  // the streamed completion text is captured
  await expect(u.t("profile-run").first()).toContainText("streamed test reply");
});

test("profile LLM provider: unhealthy verdict on HTTP errors", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-profile").click();
  await u.t("profile-url").fill(`${app.echo}/status/500`);
  await u.t("profile-iterations").fill("1");
  await u.t("profile-start").click();
  await expect(u.t("profile-verdict")).toHaveText("unhealthy", { timeout: 15000 });
  await expect(u.t("profile-run").first()).toContainText("500");
});

test("profile LLM provider: API key is masked and sent as a bearer token", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-profile").click();
  // /bearer answers 401 without a token, 200 (plain JSON, no stream) with one
  await u.t("profile-url").fill(`${app.echo}/bearer`);
  await u.t("profile-iterations").fill("1");
  await u.t("profile-start").click();
  await expect(u.t("profile-verdict")).toHaveText("unhealthy", { timeout: 15000 });
  await u.t("profile-key").fill("sk-e2e-key");
  await expect(u.t("profile-key")).toHaveAttribute("type", "password");
  await u.t("profile-start").click();
  await expect(u.t("profile-verdict")).toHaveText("degraded", { timeout: 15000 });
  await expect(u.t("profile-results")).toContainText("not an event stream");
  await expect(u.t("profile-run").first()).toContainText("200");
});

test("benchmark: HTTP errors are counted separately and zero latencies are shown", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-benchmark").click();
  await u.t("bench-url").fill(`${app.echo}/status/500`);
  await u.t("bench-iterations").fill("4");
  await u.t("bench-concurrency").fill("2");
  await u.t("benchmark-start").click();
  await expect(u.t("benchmark-requests")).toHaveText("4", { timeout: 15000 });
  await expect(u.t("benchmark-errors")).toHaveText("0");
  await expect(u.t("benchmark-http-errors")).toHaveText("4");
  // the "all requests" row is listed even when the fastest request took 0 ms
  await expect(u.t("stats-table").locator("tbody tr")).toHaveCount(2);
});

test("benchmark: ad-hoc URL with latency percentiles", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-benchmark").click();
  await u.t("bench-url").fill(`${app.echo}/get`);
  await u.t("bench-iterations").fill("8");
  await u.t("benchmark-start").click();
  await expect(u.t("benchmark-requests")).toHaveText("8", { timeout: 15000 });
  await expect(u.t("benchmark-errors")).toHaveText("0");
  const stats = u.t("benchmark-stats");
  await expect(stats).toContainText("Latency p50");
  await expect(stats).toContainText("Requests/s");
  const table = u.t("stats-table");
  for (const p of ["MIN", "P10", "P50", "P90", "P95", "P99", "MAX", "MEAN"]) {
    await expect(table).toContainText(p);
  }
});

test("benchmark: whole collection with per-request rows", async ({ page, app }) => {
  const u = ui(page);
  await importDemo(page, app, "Bench demo");
  await u.t("tool-benchmark").click();
  await u.t("bench-scope").selectOption("collection");
  await u.t("bench-collection").selectOption({ label: "Bench demo" });
  await u.t("bench-iterations").fill("2");
  await u.t("benchmark-start").click();
  // 5 requests × 2 iterations
  await expect(u.t("benchmark-requests")).toHaveText("10", { timeout: 20000 });
  await expect(u.t("benchmark-errors")).toHaveText("0");
  const table = u.t("stats-table");
  for (const name of ["Auth/Login", "Auth/Profile", "Flow/Loop", "Broken/Failing"]) {
    await expect(table).toContainText(name);
  }
});

test("benchmark: fixed time at a fixed rate, thresholds, charts, reports and baseline comparison", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-benchmark").click();
  await u.t("bench-url").fill(`${app.echo}/delay/15`);

  // a bad threshold is rejected before anything runs
  await u.t("bench-thresholds").fill("p96<5");
  await u.t("benchmark-start").click();
  await expect(u.t("toast").filter({ hasText: "unknown threshold metric" }).first()).toBeVisible();
  await expect(u.t("benchmark-stats")).toHaveCount(0);

  await u.t("bench-mode").selectOption("duration");
  await u.t("bench-duration").fill("2");
  await u.t("bench-concurrency").fill("3");
  await u.t("bench-rate").fill("30");
  await u.t("bench-warmup").fill("2");
  await u.t("bench-thresholds").fill("p95<2s\nerror_rate<=0%\nrps>=1000");
  await u.t("benchmark-start").click();
  // interim results appear while it runs
  await expect(u.t("benchmark-stop")).toBeVisible();
  await expect(u.t("benchmark-stats")).toContainText("Requests so far");
  await expect(u.t("benchmark-verdict")).toHaveText("Thresholds failed", { timeout: 20000 });

  // about 30 req/s for 2 s
  const total = parseInt(await u.t("benchmark-requests").textContent(), 10);
  expect(total).toBeGreaterThanOrEqual(45);
  expect(total).toBeLessThanOrEqual(62);
  const rows = u.t("threshold-row");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText("PASS");
  await expect(rows.nth(1)).toContainText("PASS");
  await expect(rows.nth(2)).toContainText("FAIL");
  await expect(rows.nth(2)).toContainText("measured");
  await expect(u.t("status-table")).toContainText("200");
  await expect(u.t("stats-table")).toContainText("first byte");
  await expect(u.t("stats-table")).toContainText("P99.9");
  await expect(u.t("benchmark-results")).toContainText("2 warm-up request(s)");

  // charts: drawn, with a tooltip on hover and a table view of the same numbers
  const hist = u.t("chart-histogram");
  await expect(hist.locator("path.viz-bar").first()).toBeVisible();
  await hist.locator(".viz-hit").first().hover();
  await expect(hist.locator(".viz-tip")).toContainText("requests");
  await hist.getByText("Show data").click();
  await expect(hist.locator("details table")).toContainText("Latency (ms)");
  const line = u.t("chart-timeline");
  await expect(line.locator("path.viz-line")).toBeVisible();
  await line.locator(".viz-hit").focus();
  await page.keyboard.press("ArrowLeft");
  await expect(line.locator(".viz-tip")).toContainText("second");

  // reports
  const html = await (await page.request.get(app.url + (await u.t("bench-report-html").getAttribute("href")))).text();
  expect(html).toContain("Latency distribution");
  expect(html).toContain("Thresholds failed");
  const csv = await (await page.request.get(app.url + (await u.t("bench-report-csv").getAttribute("href")))).text();
  expect(csv.split("\n")[0]).toBe("iteration,name,started_ms,time_ms,ttfb_ms,status,size,error");
  expect(csv.trim().split("\n").length).toBe(total + 1);
  const json = await (await page.request.get(app.url + (await u.t("bench-report-json").getAttribute("href")))).text();
  expect(JSON.parse(json).total_requests).toBe(total);

  // compare with a baseline: garbage is refused, the run itself is "no regression"
  await u.t("compare-file").setInputFiles({ name: "junk.json", mimeType: "application/json", buffer: Buffer.from("{\"hello\": 1}") });
  await expect(u.t("toast").filter({ hasText: "not a benchmark JSON report" }).first()).toBeVisible();
  await u.t("compare-file").setInputFiles({ name: "baseline.json", mimeType: "application/json", buffer: Buffer.from(json) });
  await expect(u.t("compare-verdict")).toHaveText("No regression");
  await expect(u.t("compare-table")).toContainText("p95 (ms)");
  // a baseline that was five times faster flags this run
  const faster = JSON.parse(json);
  for (const k of ["min", "p10", "p50", "p75", "p90", "p95", "p99", "p999", "max", "mean"]) faster.overall[k] /= 5;
  await u.t("compare-file").setInputFiles({ name: "faster.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(faster)) });
  await expect(u.t("compare-verdict")).toHaveText("Regression");
  const withCompare = await (await page.request.get(app.url + (await u.t("bench-report-html").getAttribute("href")))).text();
  expect(withCompare).toContain("This run is worse than the baseline.");
});

test("benchmark: a run can be stopped", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-benchmark").click();
  await u.t("bench-url").fill(`${app.echo}/delay/20`);
  await u.t("bench-mode").selectOption("duration");
  await u.t("bench-duration").fill("60");
  await u.t("benchmark-start").click();
  await expect(u.t("benchmark-stats")).toContainText("Requests so far");
  await u.t("benchmark-stop").click();
  await expect(u.t("benchmark-start")).toBeVisible({ timeout: 10000 });
  await expect(u.t("benchmark-results")).toContainText("cancelled");
  await expect(u.t("bench-report-json")).toBeVisible();
});
