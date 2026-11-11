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

test("collection runner: folder run, data file, failures, reports, bail", async ({ page, app }) => {
  const u = ui(page);
  await importDemo(page, app, "Runner demo");
  await u.t("tool-runner").click();
  await u.t("run-collection").selectOption({ label: "Runner demo" });
  await u.t("run-folder").selectOption({ label: "Folder: Auth" });
  await u.t("run-start").click();
  await expect(u.t("run-verdict")).toContainText("Passed");
  await expect(u.t("run-row")).toHaveCount(2);
  await expect(u.t("run-row").first()).toContainText("Auth/Login");
  await expect(u.t("run-row").first()).toContainText("2/2 tests");
  // the run stored the token as a collection variable (Keep variable changes is on)
  expect(app.cli("collection", "show", "Runner demo")).toContain("token=ann-token");

  // whole collection, two data rows
  await u.t("run-folder").selectOption({ index: 0 });
  await u.t("run-data-file").setInputFiles({ name: "users.csv", mimeType: "text/csv", buffer: Buffer.from("user,role\nann,admin\nbob,editor\n") });
  await expect(u.t("run-data-name")).toContainText("users.csv");
  await u.t("run-start").click();
  await expect(u.t("run-verdict")).toContainText("Failed");
  await expect(u.t("run-row")).toHaveCount(14);
  await expect(u.t("run-requests")).toHaveText("14");
  await expect(u.t("run-failed")).toHaveText("2");
  await expect(u.t("runner")).toContainText("Iteration 2");
  const failing = u.t("run-row").filter({ hasText: "Broken/Failing" }).first();
  await expect(failing).toContainText("expected response to have status code 200 but got 500");
  await expect(u.t("run-row").filter({ hasText: "Flow/Skipped" }).first()).toContainText("skipped");

  const href = await u.t("run-report-html").getAttribute("href");
  const report = await page.request.get(app.url + href);
  expect(await report.text()).toContain("Run finished with failures");
  const junit = await page.request.get(app.url + (await u.t("run-report-junit").getAttribute("href")));
  expect(await junit.text()).toContain('<testsuites name="Runner demo"');

  // bail + explicit iterations
  await u.t("run-folder").selectOption({ label: "Folder: Broken" });
  await u.t("run-iterations").fill("3");
  await u.t("run-bail").check();
  await u.t("run-start").click();
  await expect(u.t("run-verdict")).toContainText("bailed after failure");
  await expect(u.t("run-row")).toHaveCount(1);
});

test("collection menu opens the runner; drag and drop moves requests; docs", async ({ page, app, context }) => {
  const u = ui(page);
  await importDemo(page, app, "Menu demo");
  await u.collectionRow("Menu demo").click();
  await u.menuOf(u.folderRow("Flow"), "Run folder");
  await expect(u.t("run-collection")).toHaveValue(/.+/);
  await expect(u.t("run-folder").locator("option:checked")).toHaveText("Folder: Flow");
  await u.t("run-start").click();
  await expect(u.t("run-verdict")).toContainText("Passed");
  await expect(u.t("run-row")).toHaveCount(4); // Loop x3 via setNextRequest + Skipped

  // move "Profile" from Auth into Broken by drag and drop
  await u.folderRow("Auth").click();
  await u.requestRow("Profile").dragTo(u.folderRow("Broken"));
  const folderNames = (folder) => JSON.parse(app.cli("export", "collection", "Menu demo")).item.find((i) => i.name === folder).item.map((i) => i.name);
  await expect.poll(() => folderNames("Broken")).toEqual(["Failing", "Profile"]);
  expect(folderNames("Auth")).toEqual(["Login"]);
  await expect(u.requestRow("Profile")).toBeVisible();

  const popup = context.waitForEvent("page");
  await u.menuOf(u.collectionRow("Menu demo"), "View documentation");
  const docs = await popup;
  await docs.waitForLoadState();
  await expect(docs).toHaveTitle("Menu demo – API documentation");
  await expect(docs.locator("main")).toContainText("Example: logged in");
});

test("code snippets", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/post?q=1`, "POST");
  await u.t("req-tab-headers").click();
  await u.fillRow("headers-table", 0, "X-Snip", "it's");
  await u.t("req-tab-body").click();
  await u.t("body-mode-raw").check();
  await u.t("body-raw").fill('{"a": 1}');
  await u.t("codegen").click();
  await u.t("codegen-lang").selectOption("curl");
  await expect(u.t("codegen-output")).toContainText(`curl --location --request POST '${app.echo}/post?q=1'`);
  await expect(u.t("codegen-output")).toContainText("--header 'X-Snip: it'\\''s'");
  await u.t("codegen-lang").selectOption("python");
  await expect(u.t("codegen-output")).toContainText("import requests");
  await u.t("codegen-lang").selectOption("rust");
  await expect(u.t("codegen-output")).toContainText("reqwest::blocking::Client::new()");
});

test("mock server serves saved examples", async ({ page, app }) => {
  const u = ui(page);
  await importDemo(page, app, "Mock demo");
  await u.collectionRow("Mock demo").click();
  await u.menuOf(u.collectionRow("Mock demo"), "Mock collection");
  await expect(u.t("mock-collection").locator("option:checked")).toHaveText("Mock demo");
  await u.t("mock-start").click();
  const card = u.t("mock-card");
  await expect(card).toHaveCount(1);
  const mockUrl = (await u.t("mock-url").textContent()).trim();
  expect(mockUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
  await expect(card).toContainText("/bearer");

  await u.newRequest(`${mockUrl}/bearer`);
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"plan": "pro"');
  await u.t("req-tab-headers").click();
  await u.fillRow("headers-table", 0, "x-mock-response-code", "401");
  await u.send("401 Unauthorized");

  await page.getByTestId("tab").filter({ hasText: "Mock servers" }).click();
  await expect(u.t("mock-log")).toContainText("GET /bearer -> 200");
  await expect(u.t("mock-log")).toContainText("GET /bearer -> 401");
  await u.t("mock-stop").click();
  await expect(u.t("mock-card")).toHaveCount(0);

  // a collection without examples cannot be mocked
  await u.createCollection("No examples");
  await page.getByTestId("tab").filter({ hasText: "Mock servers" }).click();
  await u.t("mock-collection").selectOption({ label: "No examples" });
  await u.t("mock-start").click();
  await expect(u.t("toast").filter({ hasText: "no saved examples" }).first()).toBeVisible();
});

test("websocket client", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-ws").click();
  await u.t("ws-url").fill("ws://127.0.0.1:1/nothing");
  await u.t("ws-connect").click();
  await expect(u.t("toast").filter({ hasText: "WebSocket connection failed" }).first()).toBeVisible();
  await expect(u.t("ws-state")).toHaveText("Disconnected");

  await u.t("ws-url").fill(`ws://127.0.0.1:${app.echoPort}/ws`);
  await u.t("ws-connect").click();
  await expect(u.t("ws-state")).toHaveText("Connected");
  await expect(u.t("ws-log")).toContainText('"event":"connected"');
  await u.t("ws-message").fill('{"hello": "socket"}');
  await u.t("ws-send").click();
  await expect(u.t("ws-event").filter({ hasText: "↓" }).filter({ hasText: '{"hello": "socket"}' })).toHaveCount(1);
  await expect(u.t("ws-event").filter({ hasText: "↑" })).toHaveCount(1);
  await u.t("ws-disconnect").click();
  await expect(u.t("ws-state")).toHaveText("Disconnected");
  await expect(u.t("ws-send")).toBeDisabled();
});

test("cookie manager", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-cookies").click();
  await u.t("cookie-clear").click();
  await u.t("confirm-ok").click();
  await u.t("cookie-add").click();
  await expect(u.t("toast").filter({ hasText: "Enter a URL and a cookie" }).first()).toBeVisible();
  await u.t("cookie-url").fill(app.echo);
  await u.t("cookie-value").fill("manual=yes; Path=/");
  await u.t("cookie-add").click();
  await expect(u.t("cookie-row")).toHaveCount(1);

  await u.newRequest(`${app.echo}/cookies/set?fromserver=1`);
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"manual": "yes"');
  await expect(u.t("response-body")).toContainText('"fromserver": "1"');

  await page.getByTestId("tab").filter({ hasText: "Cookies" }).click();
  await expect(u.t("cookie-row")).toHaveCount(2);
  await u.t("cookie-row").filter({ hasText: "manual" }).getByTestId("cookie-delete").click();
  await expect(u.t("cookie-row")).toHaveCount(1);
  expect(app.cli("cookies", "list")).toContain("fromserver");
  await u.t("cookie-clear").click();
  await u.t("confirm-ok").click();
  await expect(u.t("cookie-row")).toHaveCount(0);
});

test("settings are applied to requests and shared with the CLI", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-settings").click();
  await u.t("set-follow_redirects").uncheck();
  await u.t("set-timeout_ms").fill("400");
  await u.t("settings-save").click();
  await expect(u.t("toast").filter({ hasText: "Settings saved" }).first()).toBeVisible();
  expect(JSON.parse(app.cli("settings", "show")).follow_redirects).toBe(false);

  await u.newRequest(`${app.echo}/redirect/1`);
  await u.send("302 Found");
  await u.t("url").fill(`${app.echo}/delay/1500`);
  await u.t("send").click();
  await expect(u.t("response-error")).toBeVisible();

  await page.getByTestId("tab").filter({ hasText: "Settings" }).click();
  await u.t("settings-reset").click();
  await u.t("confirm-ok").click();
  await expect(u.t("set-follow_redirects")).toBeChecked();
  // keep the test workspace independent of proxy variables
  await u.t("set-no_system_proxy").check();
  await u.t("settings-save").click();
  await expect(u.t("toast").filter({ hasText: "Settings saved" }).first()).toBeVisible();
  await page.getByTestId("tab").filter({ hasText: "Untitled request" }).click();
  await u.t("url").fill(`${app.echo}/redirect/1`);
  await u.send("200 OK");
});
