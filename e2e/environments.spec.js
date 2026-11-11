const { test, expect, ui } = require("./fixtures");

test("environments: create, activate, resolve variables, switch off", async ({ page, app }) => {
  const u = ui(page);
  await u.t("side-environments").click();
  await u.t("new-environment").click();
  await u.prompt("dev");
  await expect(u.t("environment-editor")).toBeVisible();
  await u.fillRow("environment-variables", 0, "host", app.echo);
  await u.fillRow("environment-variables", 1, "stage", "development");
  await u.fillRow("environment-variables", 2, "disabled_one", "x");
  await u.row("environment-variables", 2).getByLabel("Enabled").uncheck();
  await u.t("environment-save").click();
  await expect(u.t("toast").filter({ hasText: "Saved" }).first()).toBeVisible();

  await u.t("env-select").selectOption({ label: "dev" });
  await expect(u.t("status-env")).toHaveText("Environment: dev");
  await u.newRequest("{{host}}/get?stage={{stage}}&off={{disabled_one}}");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"stage": "development"');
  await expect(u.t("response-body")).toContainText('"off": "{{disabled_one}}"');

  // the choice survives a reload and is shared with the CLI
  await page.reload();
  await page.waitForSelector('body[data-ready="1"]');
  await expect(u.t("env-select")).toHaveValue(/.+/);
  expect(app.cli("env", "list")).toMatch(/\*\s+dev/);

  await u.t("env-select").selectOption({ label: "No environment" });
  await u.newRequest("{{host}}/get");
  await u.t("send").click();
  await expect(u.t("response-error")).toBeVisible();

  // rename through the editor, delete through the menu
  await u.t("side-environments").click();
  await page.locator('[data-testid=env-row][data-name="dev"]').click();
  await u.t("environment-name").fill("development");
  await u.t("environment-save").click();
  const row = page.locator('[data-testid=env-row][data-name="development"]');
  await expect(row).toBeVisible();
  await u.menuOf(row, "Delete");
  await u.t("confirm-ok").click();
  await expect(row).toHaveCount(0);
  await expect(u.t("environment-editor")).toHaveCount(0);
});

test("scripts: pre-request mutation, tests, console, variables written back", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/get?v=7`);
  await u.t("req-tab-pre").click();
  await u.t("script-pre").fill('pm.request.headers.add({ key: "X-Pre", value: "yes" });\npm.variables.set("local", "L");');
  await u.t("req-tab-tests").click();
  await u.t("script-test").fill(
    [
      'const args = pm.response.json().args;',
      'pm.test("v is 7", () => pm.expect(args.v).to.equal("7"));',
      'pm.test("this one fails", () => pm.expect(1).to.equal(2));',
      'pm.test.skip("skipped", () => {});',
      'pm.globals.set("seen_v", args.v);',
      'console.log("hello from test", { v: args.v });',
      'console.warn("careful");',
    ].join("\n")
  );
  await u.send("200 OK");
  // failures bring the test tab forward
  await expect(u.t("test-results")).toBeVisible();
  await expect(u.t("res-tab-tests")).toContainText("(2/3)");
  const rows = u.t("test-row");
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(0)).toContainText("PASS");
  await expect(rows.nth(1)).toContainText("FAIL");
  await expect(rows.nth(1)).toContainText("expected 1 to equal 2");
  await expect(rows.nth(2)).toContainText("SKIP");
  await u.t("res-tab-console").click();
  await expect(u.t("console")).toContainText('hello from test {"v":"7"}');
  await expect(u.t("console")).toContainText("careful");
  await u.t("res-tab-body").click();
  await expect(u.t("response-body")).toContainText('"x-pre": "yes"');

  // the script wrote a global: visible in the Globals editor
  await u.t("side-environments").click();
  await page.locator('[data-testid=env-row][data-name="Globals"]').click();
  await expect(u.row("environment-variables", 0).getByLabel("Variable")).toHaveValue("seen_v");
  await expect(u.row("environment-variables", 0).getByLabel("Value", { exact: true })).toHaveValue("7");
});

test("script errors are reported; snippets insert code", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/get`);
  await u.t("req-tab-tests").click();
  await page.getByRole("button", { name: "Status code is 200" }).click();
  await expect(u.t("script-test")).toHaveValue(/pm\.response\.to\.have\.status\(200\)/);
  await u.send("200 OK");
  await expect(u.t("res-tab-tests")).toContainText("(1/1)");

  await u.t("script-test").fill("this is not javascript (");
  await u.t("send").click();
  await expect(u.t("test-results")).toContainText("Script failed");
  await expect(u.t("test-results")).toContainText("SyntaxError");

  await u.t("req-tab-pre").click();
  await u.t("script-pre").fill("throw new Error('stop here');");
  await u.t("send").click();
  await expect(u.t("response-error")).toContainText("pre-request script error: Error: stop here");
});

test("history lists sent requests and reopens them with their response", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/get?history=first`);
  await u.send("200 OK");
  await u.t("url").fill(`${app.echo}/status/404`);
  await u.send("404 Not Found");
  await u.t("side-history").click();
  const rows = u.t("history-row");
  await expect(rows.first()).toContainText("/status/404");
  await expect(rows.first()).toContainText("404");
  await rows.filter({ hasText: "history=first" }).first().click();
  await expect(u.t("url")).toHaveValue(`${app.echo}/get?history=first`);
  await expect(u.t("response-status")).toHaveText("200 OK");
  await expect(u.t("response-body")).toContainText('"history": "first"');

  await u.t("history-clear").click();
  await u.t("confirm-ok").click();
  await expect(u.t("history-row")).toHaveCount(0);
  expect(app.cli("history", "list")).toContain("History is empty");
});

test("theme toggle and keyboard shortcuts", async ({ page, app }) => {
  const u = ui(page);
  const before = await page.evaluate(() => document.documentElement.dataset.theme);
  await u.t("theme-toggle").click();
  const after = await page.evaluate(() => document.documentElement.dataset.theme);
  expect(after).not.toBe(before);
  await page.reload();
  await page.waitForSelector('body[data-ready="1"]');
  expect(await page.evaluate(() => document.documentElement.dataset.theme)).toBe(after);

  await page.keyboard.press("Control+n");
  await expect(u.t("request-view")).toBeVisible();
  await u.t("url").fill(`${app.echo}/get?kbd=1`);
  await u.t("req-tab-headers").click();
  await page.keyboard.press("Control+Enter");
  await expect(u.t("response-status")).toHaveText("200 OK");
});
