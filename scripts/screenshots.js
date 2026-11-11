#!/usr/bin/env node
// Regenerates the screenshots used by docs/user-manual.md and the in-app Help Center
// (ui/help/img/*.png). Starts rsPostman and its echo server on a throw-away workspace,
// seeds demo data and drives the interface with Playwright.
//
//   cargo build && node scripts/screenshots.js
//
// Needs Playwright (`npm install` in e2e/, or a global install) and free ports 8686 / 8642.
const path = require("path");
const fs = require("fs");
const os = require("os");
const { spawn, execFileSync } = require("child_process");
const { chromium } = require(require.resolve("playwright", { paths: [__dirname, path.join(__dirname, "..", "e2e"), process.cwd()] }));

const ROOT = path.join(__dirname, "..");
const BIN = process.env.RSPOSTMAN_BIN || path.join(ROOT, "target", "debug", process.platform === "win32" ? "rspostman.exe" : "rspostman");
const OUT = path.join(ROOT, "ui", "help", "img");
const GUI = "http://127.0.0.1:8686", ECHO = "http://127.0.0.1:8642";
const W = 1360, H = 800;

function start(args, pattern, env) {
  return new Promise((resolve, reject) => {
    const proc = spawn(BIN, args, { stdio: ["ignore", "ignore", "pipe"], env: { ...process.env, ...env } });
    let buf = "";
    const timer = setTimeout(() => reject(new Error(`timeout starting ${args.join(" ")}\n${buf}`)), 15000);
    proc.stderr.on("data", (d) => { buf += d; if (pattern.test(buf)) { clearTimeout(timer); resolve(proc); } });
    proc.on("exit", (code) => { clearTimeout(timer); reject(new Error(`${args.join(" ")} exited with ${code}\n${buf}`)); });
  });
}

(async () => {
  if (!fs.existsSync(BIN)) throw new Error(`${BIN} not found – run \`cargo build\` first`);
  fs.mkdirSync(OUT, { recursive: true });
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "rsp-shots-"));
  const cli = (...args) => execFileSync(BIN, ["--workspace", home, ...args], { encoding: "utf8", env: { ...process.env, NO_COLOR: "1" } });
  const procs = [];
  let browser;
  try {
    // ---- demo data
    cli("settings", "set", "no_system_proxy", "true");
    const demo = JSON.parse(fs.readFileSync(path.join(ROOT, "examples", "workflow.postman_collection.json"), "utf8"));
    demo.variable.find((v) => v.key === "base").value = ECHO;
    fs.writeFileSync(path.join(home, "demo.json"), JSON.stringify(demo));
    cli("import", path.join(home, "demo.json"));
    cli("collection", "create", "Shop API", "-d", "Orders and products of the demo shop.");
    cli("c", "set-var", "Shop API", "base", ECHO);
    cli("c", "add-folder", "Shop API", "Orders");
    cli("c", "add-request", "Shop API", "List orders", "{{base}}/delay/12?page=1", "--parent", "Orders");
    cli("c", "add-request", "Shop API", "Get order", "{{base}}/delay/25", "--parent", "Orders");
    cli("c", "add-request", "Shop API", "Create order", "POST", "{{base}}/delay/45", "--json", '{"sku": "A-100", "quantity": 2}', "--parent", "Orders");
    cli("c", "add-request", "Shop API", "List products", "{{base}}/get?category=books");
    cli("c", "add-request", "Shop API", "Health", "{{base}}/delay/4");
    cli("env", "create", "dev");
    cli("env", "set", "dev", "host", ECHO);
    cli("env", "set", "dev", "user", "ann");
    cli("env", "set", "dev", "api_token", "tok_live_4f9a1c", "--secret");
    cli("env", "create", "prod");
    cli("env", "set", "prod", "host", "https://api.example.com");
    cli("env", "use", "dev");

    procs.push(await start(["--workspace", home, "echo"], /listening on/));
    // RSPOSTMAN_UI_DIR: serve the UI (and freshly written screenshots) from the source tree
    procs.push(await start(["--workspace", home, "gui", "--no-open"], /running at/, { RSPOSTMAN_UI_DIR: path.join(ROOT, "ui") }));

    browser = await chromium.launch();
    const context = await browser.newContext({ viewport: { width: W, height: H }, deviceScaleFactor: 2, colorScheme: "light" });
    const page = await context.newPage();
    page.on("pageerror", (e) => { throw e; });
    const t = (id) => page.getByTestId(id);
    const shot = async (name, opts = {}) => {
      await page.evaluate(() => document.querySelectorAll(".toast").forEach((x) => x.remove()));
      await page.waitForTimeout(120);
      await (opts.locator || page).screenshot({ path: path.join(OUT, name + ".png"), clip: opts.clip });
      console.log("  " + name + ".png");
    };
    const size = (h) => page.setViewportSize({ width: W, height: h });
    const row = (kind, name) => page.locator(`[data-testid=${kind}-row][data-name="${name}"]`);
    const closeTabs = async () => {
      while (await t("tab-close").count()) {
        await t("tab-close").first().click();
        if (await t("confirm-ok").count()) await t("confirm-ok").click();
      }
    };
    const fill = async (table, n, key, value) => {
      const r = t(table).locator("tbody tr").nth(n);
      await r.locator("td:nth-child(2) input").fill(key);
      if (value !== undefined) await r.getByLabel("Value", { exact: true }).fill(value);
    };
    const mainClip = (y, h) => ({ x: 300, y, width: W - 300, height: h });

    await page.goto(GUI);
    await page.waitForSelector('body[data-ready="1"]');
    await page.evaluate(() => localStorage.setItem("rsp-theme", "light"));
    await page.reload();
    await page.waitForSelector('body[data-ready="1"]');

    // ---- overview / main window
    await row("collection", "Workflow demo").click();
    await row("folder", "Auth").click();
    await row("request", "Login").click();
    await t("req-tab-body").click();
    await t("send").click();
    await t("response-status").waitFor();
    await shot("overview");
    await t("theme-toggle").click();
    await shot("main-window");
    await t("theme-toggle").click();

    // ---- scripts and test results
    await t("req-tab-tests").click();
    await t("res-tab-tests").click();
    await shot("scripts");
    await t("res-tab-body").click();
    await shot("response-body", { clip: mainClip(H - 330, 306) });

    // ---- request editor sections (a taller window so the whole editor is visible)
    await size(1100);
    await t("tab-new").click();
    await t("url").fill(`${ECHO}/anything/users/:id?page=2&limit=20`);
    await t("path-vars").getByLabel("Value of id").fill("42");
    await fill("params-table", 2, "sort", "name");
    await t("params-table").locator("tbody tr").nth(2).getByLabel("Enabled").uncheck();
    await shot("request-params", { clip: mainClip(44, 500) });

    await page.getByTestId("request-view").last().getByTestId("method").selectOption("POST");
    await t("url").fill(`${ECHO}/post`);
    await t("req-tab-body").click();
    await t("body-mode-raw").check();
    await t("body-raw").fill('{\n  "name": "Ann",\n  "email": "ann@example.com",\n  "roles": ["admin", "editor"]\n}');
    await shot("request-body", { clip: mainClip(44, 420) });

    await t("req-tab-auth").click();
    await t("auth-type").selectOption("basic");
    await t("auth-username").fill("ann");
    await t("auth-password").fill("correct-horse");
    await shot("request-auth", { clip: mainClip(44, 420) });
    await size(H);

    await t("save").click();
    await t("save-name").fill("Create user");
    await t("save-collection").selectOption({ label: "Shop API" });
    await shot("save-request");
    await t("save-confirm").click();

    // ---- code snippet
    await t("codegen").click();
    await t("codegen-lang").selectOption("python");
    await page.waitForFunction(() => document.querySelector('[data-testid=codegen-output]').textContent.includes("import requests"));
    await shot("codegen");
    await page.keyboard.press("Escape");

    // ---- collections: tree menu and editor
    await row("collection", "Shop API").hover();
    await row("collection", "Shop API").locator(".more").click();
    await shot("collections-menu", { clip: { x: 0, y: 44, width: 620, height: 470 } });
    await page.getByRole("menuitem", { name: "Edit collection…" }).click();
    await t("collection-tab-variables").click();
    await fill("collection-variables", 1, "api_version", "v2");
    await shot("collection-editor", { clip: mainClip(44, 400) });
    await closeTabs();

    // ---- environment
    await t("side-environments").click();
    await page.locator('[data-testid=env-row][data-name="dev"]').click();
    await shot("environment");
    await closeTabs();
    await t("side-collections").click();

    // ---- runner
    await size(1010);
    await t("tool-runner").click();
    await t("run-collection").selectOption({ label: "Workflow demo" });
    await t("run-start").click();
    await t("run-verdict").waitFor();
    await shot("runner");
    await closeTabs();

    // ---- mock server
    await size(H);
    await t("tool-mock").click();
    await t("mock-collection").selectOption({ label: "Workflow demo" });
    await t("mock-port").fill("4010");
    await t("mock-start").click();
    await t("mock-card").waitFor();
    for (const [method, p] of [["GET", "/bearer"], ["POST", "/post"], ["GET", "/missing"]]) await fetch(`http://127.0.0.1:4010${p}`, { method }).catch(() => {});
    await t("mock-log").waitFor();
    await page.waitForTimeout(1600);
    await shot("mock");
    await t("mock-stop").click();
    await closeTabs();

    // ---- websocket
    await t("tool-ws").click();
    await t("ws-url").fill("ws://127.0.0.1:8642/ws");
    await t("ws-connect").click();
    await t("ws-state").filter({ hasText: "Connected" }).waitFor();
    for (const m of ['{"type": "subscribe", "channel": "orders"}', "ping"]) {
      await t("ws-message").fill(m);
      await t("ws-send").click();
      await page.waitForTimeout(400);
    }
    await shot("websocket");
    await closeTabs();

    // ---- benchmark (collection with different latencies) and its HTML report
    await size(1340);
    await t("tool-benchmark").click();
    await t("bench-scope").selectOption("collection");
    await t("bench-collection").selectOption({ label: "Shop API" });
    await t("bench-folder").selectOption({ label: "Folder: Orders" });
    await t("bench-iterations").fill("40");
    await t("bench-concurrency").fill("3");
    await t("bench-warmup").fill("1");
    await t("bench-thresholds").fill("p95<200ms\nerror_rate<=1%");
    await t("benchmark-start").click();
    await t("benchmark-verdict").waitFor({ timeout: 60000 });
    await shot("benchmark");
    const href = await t("bench-report-html").getAttribute("href");
    const report = await context.newPage();
    await report.setViewportSize({ width: 1000, height: 1080 });
    await report.goto(GUI + href);
    await report.screenshot({ path: path.join(OUT, "benchmark-report.png") });
    console.log("  benchmark-report.png");
    await report.close();
    await closeTabs();

    // ---- LLM profile
    await size(900);
    await t("tool-profile").click();
    await t("profile-url").fill(`${ECHO}/llm`);
    await t("profile-key").fill("sk-demo-key");
    await t("profile-model").fill("demo-model");
    await t("profile-start").click();
    await t("profile-verdict").waitFor({ timeout: 30000 });
    await shot("profile");
    await closeTabs();

    // ---- vault
    await size(H);
    await t("vault-open").click();
    await t("vault-new-pass").fill("correct horse battery staple");
    await t("vault-new-pass2").fill("correct horse battery staple");
    await t("vault-create").click();
    for (const [n, v] of [["github_token", "ghp_demo"], ["db.password", "demo"]]) {
      await t("vault-name").fill(n);
      await t("vault-value").fill(v);
      await t("vault-add").click();
      await t("vault-row").filter({ hasText: n }).waitFor();
    }
    await shot("vault");
    await closeTabs();

    // ---- cookies and history
    await t("tab-new").click();
    await t("url").fill(`${ECHO}/cookies/set?session=8f3a2c&theme=dark`);
    await t("send").click();
    await t("response-status").waitFor();
    await closeTabs();
    await t("tool-cookies").click();
    await t("cookie-row").first().waitFor();
    await shot("cookies", { clip: { x: 0, y: 0, width: W, height: 420 } });
    await closeTabs();
    await t("side-history").click();
    await t("history-row").first().click();
    await t("response-status").waitFor();
    await shot("history");
    await closeTabs();
    await t("side-collections").click();

    // ---- import dialog
    await t("tool-import").click();
    await t("import-text").fill(`curl -X POST '${ECHO}/post' \\\n  -H 'Content-Type: application/json' \\\n  -u ann:secret \\\n  --data-raw '{"sku": "A-100", "quantity": 2}'`);
    await shot("import");
    await page.keyboard.press("Escape");

    // ---- settings
    await size(840);
    await t("tool-settings").click();
    await shot("settings");
    await closeTabs();

    // ---- help center (last: it shows a screenshot taken above)
    await size(H);
    await t("help-open").click();
    await t("help-topic").filter({ hasText: "Collection runner" }).click();
    await page.locator("[data-testid=help-content] img").first().evaluate((img) => img.complete || new Promise((r) => { img.onload = r; img.onerror = r; }));
    await shot("help-center");

    console.log(`done – ${fs.readdirSync(OUT).filter((f) => f.endsWith(".png")).length} screenshots in ${path.relative(ROOT, OUT)}`);
  } finally {
    if (browser) await browser.close();
    procs.forEach((p) => p.kill());
    fs.rmSync(home, { recursive: true, force: true });
  }
})().catch((e) => { console.error(e); process.exit(1); });
