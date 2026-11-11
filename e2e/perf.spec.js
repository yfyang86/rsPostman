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
