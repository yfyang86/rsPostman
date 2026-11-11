// Each worker gets its own throw-away workspace, GUI backend and echo server.
const base = require("./pw");
const { spawn, execFileSync } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const BIN = process.env.RSPOSTMAN_BIN || path.join(__dirname, "..", "target", "debug", process.platform === "win32" ? "rspostman.exe" : "rspostman");

function start(args, pattern) {
  return new Promise((resolve, reject) => {
    const proc = spawn(BIN, args, { stdio: ["ignore", "ignore", "pipe"], env: process.env });
    let buf = "";
    if (process.env.RSPOSTMAN_E2E_LOG) proc.stderr.on("data", (d) => fs.appendFileSync(process.env.RSPOSTMAN_E2E_LOG + "." + proc.pid, d));
    const timer = setTimeout(() => reject(new Error(`timeout starting rspostman ${args.join(" ")}\n${buf}`)), 15000);
    proc.stderr.on("data", (d) => {
      buf += d.toString();
      const m = pattern.exec(buf);
      if (m) {
        clearTimeout(timer);
        resolve({ proc, url: m[1] });
      }
    });
    proc.on("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`rspostman ${args.join(" ")} exited with ${code}\n${buf}`));
    });
  });
}

const test = base.test.extend({
  app: [
    async ({}, use) => {
      if (!fs.existsSync(BIN)) throw new Error(`rspostman binary not found at ${BIN}; run \`cargo build\` first or set RSPOSTMAN_BIN`);
      const home = fs.mkdtempSync(path.join(os.tmpdir(), "rsp-e2e-"));
      const cli = (...args) => execFileSync(BIN, ["--workspace", home, ...args], { encoding: "utf8", env: { ...process.env, NO_COLOR: "1" } });
      cli("settings", "set", "no_system_proxy", "true");
      const echo = await start(["--workspace", home, "echo", "--port", "0"], /listening on (http:\/\/\S+)/);
      const gui = await start(["--workspace", home, "gui", "--port", "0", "--no-open"], /running at (http:\/\/\S+)/);
      await use({ url: gui.url, echo: echo.url, echoPort: new URL(echo.url).port, home, cli });
      gui.proc.kill();
      echo.proc.kill();
      fs.rmSync(home, { recursive: true, force: true });
    },
    { scope: "worker" },
  ],

  // Opens the app and fails the test on any uncaught page error or unexpected console error.
  page: async ({ page, app }, use) => {
    const problems = [];
    page.on("pageerror", (e) => problems.push("pageerror: " + e.message));
    page.on("console", (m) => {
      if (m.type() === "error" && !/Failed to load resource/.test(m.text())) problems.push("console: " + m.text());
    });
    await page.goto(app.url);
    await page.waitForSelector('body[data-ready="1"]');
    await use(page);
    base.expect(problems, "browser errors during the test").toEqual([]);
  },
});

/** Helpers bound to a page. */
function ui(page) {
  const t = (id) => page.getByTestId(id);
  const api = {
    t,
    async newRequest(url, method) {
      await t("tab-new").click();
      if (method) await page.getByTestId("request-view").last().getByTestId("method").selectOption(method);
      if (url !== undefined) await t("url").fill(url);
    },
    async send(expectedStatus) {
      await t("send").click();
      if (expectedStatus) await base.expect(t("response-status")).toHaveText(expectedStatus);
    },
    row(table, n) {
      return t(table).locator("tbody tr").nth(n);
    },
    async fillRow(table, n, key, value) {
      const row = api.row(table, n);
      await row.locator("td:nth-child(2) input").fill(key);
      if (value !== undefined) await row.getByLabel("Value", { exact: true }).or(row.getByLabel("Variable value")).first().fill(value);
    },
    async prompt(value) {
      await t("prompt-input").fill(value);
      await t("prompt-ok").click();
    },
    async saveAs(name, { collection, newCollection, folder } = {}) {
      await t("save").click();
      await t("save-name").fill(name);
      if (newCollection) {
        await t("save-collection").selectOption({ label: "＋ New collection…" });
        await t("save-new-collection").fill(newCollection);
      } else if (collection) {
        await t("save-collection").selectOption({ label: collection });
        if (folder) await t("save-folder").selectOption({ label: folder });
      }
      await t("save-confirm").click();
      await base.expect(t("toast").filter({ hasText: "Saved" }).first()).toBeVisible();
    },
    collectionRow: (name) => page.locator(`[data-testid=collection-row][data-name="${name}"]`),
    requestRow: (name) => page.locator(`[data-testid=request-row][data-name="${name}"]`),
    folderRow: (name) => page.locator(`[data-testid=folder-row][data-name="${name}"]`),
    async menuOf(row, item) {
      await row.hover();
      await row.locator(".more").click();
      await page.getByRole("menuitem", { name: item, exact: true }).click();
    },
    async createCollection(name) {
      await t("side-collections").click();
      await t("new-collection").click();
      await api.prompt(name);
      await base.expect(api.collectionRow(name)).toBeVisible();
    },
  };
  return api;
}

module.exports = { test, expect: base.expect, ui };
