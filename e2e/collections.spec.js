const { test, expect, ui } = require("./fixtures");
const fs = require("fs");
const path = require("path");

test("save a request, reload, edit and save again", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/get?saved=1`);
  await expect(u.t("unsaved-hint")).toBeVisible();
  await u.saveAs("Saved GET", { newCollection: "Persistence" });
  await expect(u.requestRow("Saved GET")).toBeVisible();
  await expect(u.t("unsaved-hint")).toHaveCount(0);
  await expect(u.t("tab-dirty")).toHaveCount(0);

  await page.reload();
  await page.waitForSelector('body[data-ready="1"]');
  if (!(await u.requestRow("Saved GET").isVisible())) await u.collectionRow("Persistence").click();
  await u.requestRow("Saved GET").click();
  await expect(u.t("url")).toHaveValue(`${app.echo}/get?saved=1`);

  await u.t("url").fill(`${app.echo}/get?saved=2`);
  await expect(u.t("tab-dirty")).toBeVisible();
  await page.keyboard.press("Control+s");
  await expect(u.t("tab-dirty")).toHaveCount(0);
  expect(app.cli("collection", "show", "Persistence")).toContain("/get?saved=2");

  // closing a dirty saved tab asks for confirmation
  await u.t("url").fill(`${app.echo}/get?saved=3`);
  await page.getByTestId("tab-close").first().click();
  await expect(u.t("confirm")).toContainText("unsaved changes");
  await u.t("confirm-ok").click();
  await expect(u.t("tab")).toHaveCount(0);
});

test("tree operations: folders, rename, duplicate, delete, filter", async ({ page, app }) => {
  const u = ui(page);
  await u.createCollection("Tree ops");
  await u.menuOf(u.collectionRow("Tree ops"), "Add folder");
  await u.prompt("Users");
  await expect(u.folderRow("Users")).toBeVisible();

  await u.menuOf(u.folderRow("Users"), "Add request");
  await u.t("url").fill(`${app.echo}/get?who=list`);
  await u.saveAs("List users");
  await expect(u.requestRow("List users")).toBeVisible();
  expect(app.cli("collection", "show", "Tree ops")).toMatch(/Users\n\s+GET\s+List users/);

  await u.menuOf(u.requestRow("List users"), "Duplicate");
  await expect(u.requestRow("List users Copy")).toBeVisible();
  await u.menuOf(u.requestRow("List users Copy"), "Rename…");
  await u.prompt("Get user");
  await expect(u.requestRow("Get user")).toBeVisible();
  // the open tab of the renamed original keeps working
  await u.requestRow("List users").click();
  await u.send("200 OK");

  await u.t("tree-filter").fill("get us");
  await expect(u.requestRow("Get user")).toBeVisible();
  await expect(u.requestRow("List users")).toHaveCount(0);
  await u.t("tree-filter").fill("");

  await u.menuOf(u.requestRow("Get user"), "Delete");
  await u.t("confirm-ok").click();
  await expect(u.requestRow("Get user")).toHaveCount(0);

  // deleting the folder detaches the open tab instead of losing the work
  await u.menuOf(u.folderRow("Users"), "Delete");
  await u.t("confirm-ok").click();
  await expect(u.folderRow("Users")).toHaveCount(0);
  await expect(u.t("unsaved-hint")).toBeVisible();

  await u.menuOf(u.collectionRow("Tree ops"), "Duplicate");
  await expect(u.collectionRow("Tree ops Copy")).toBeVisible();
  await u.menuOf(u.collectionRow("Tree ops Copy"), "Delete");
  await u.t("confirm-ok").click();
  await expect(u.collectionRow("Tree ops Copy")).toHaveCount(0);
  expect(app.cli("collection", "list")).not.toContain("Tree ops Copy");
});

test("collection variables, inherited auth and collection-level scripts", async ({ page, app }) => {
  const u = ui(page);
  await u.createCollection("Inherit");
  await u.menuOf(u.collectionRow("Inherit"), "Edit collection…");
  await u.t("collection-tab-variables").click();
  await u.fillRow("collection-variables", 0, "base", app.echo);
  await u.fillRow("collection-variables", 1, "token", "col-token");
  await u.t("collection-tab-auth").click();
  await u.t("auth-type").selectOption("bearer");
  await u.t("auth-token").fill("{{token}}");
  await u.t("collection-tab-tests").click();
  await u.t("collection-script-test").fill('pm.test("from the collection", () => pm.response.to.be.success);');
  await u.t("collection-save").click();
  await expect(u.t("toast").filter({ hasText: "Saved" }).first()).toBeVisible();

  await u.menuOf(u.collectionRow("Inherit"), "Add request");
  await u.t("url").fill("{{base}}/bearer");
  await u.saveAs("Who am I");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"token": "col-token"');
  await u.t("res-tab-tests").click();
  await expect(u.t("test-row")).toContainText("from the collection");

  // request-level override
  await u.t("req-tab-auth").click();
  await u.t("auth-type").selectOption("noauth");
  await u.send("401 Unauthorized");
  await u.t("res-tab-tests").click();
  await expect(u.t("test-row").first()).toContainText("FAIL");
});

test("import dialog: collection JSON, curl, invalid input; export", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-import").click();
  await u.t("import-text").fill("this is not importable");
  await u.t("import-confirm").click();
  await expect(u.t("toast").filter({ hasText: "unrecognised import format" }).first()).toBeVisible();
  await expect(u.t("import-dialog")).toBeVisible();

  const workflow = fs.readFileSync(path.join(__dirname, "..", "examples", "workflow.postman_collection.json"), "utf8");
  await u.t("import-text").fill(workflow);
  await u.t("import-confirm").click();
  await expect(u.collectionRow("Workflow demo")).toBeVisible();
  await u.folderRow("Auth").click();
  await expect(u.requestRow("Login")).toBeVisible();

  await u.t("tool-import").click();
  await u.t("import-text").fill(`curl -X PUT '${app.echo}/put?v=1' -H 'X-From: curl' -u me:pw --data-raw '{"k": true}'`);
  await u.t("import-confirm").click();
  await expect(u.t("url")).toHaveValue(`${app.echo}/put?v=1`);
  await expect(page.getByTestId("request-view").getByTestId("method")).toHaveValue("PUT");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"x-from": "curl"');
  await expect(u.t("response-body")).toContainText('"authorization": "Basic bWU6cHc="');
  await expect(u.t("response-body")).toContainText('"k": true');

  // pasting curl straight into the URL bar
  await u.t("tab-new").click();
  await u.t("url").focus();
  await page.evaluate((text) => {
    const dt = new DataTransfer();
    dt.setData("text", text);
    document.activeElement.dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
  }, `curl ${app.echo}/get -H 'X-Pasted: 1'`);
  await expect(u.t("url")).toHaveValue(`${app.echo}/get`);
  await u.t("req-tab-headers").click();
  await expect(u.row("headers-table", 0).getByLabel("Key")).toHaveValue("X-Pasted");

  // export downloads Postman v2.1 JSON
  const download = page.waitForEvent("download");
  await u.menuOf(u.collectionRow("Workflow demo"), "Export");
  const file = await (await download).path();
  const exported = JSON.parse(fs.readFileSync(file, "utf8"));
  expect(exported.info.schema).toContain("v2.1.0");
  expect(exported.item.map((i) => i.name)).toEqual(["Auth", "Flow", "Broken"]);
});

test("examples can be saved and removed", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/json`);
  await u.send("200 OK");
  await u.t("save-example").click();
  await expect(u.t("toast").filter({ hasText: "Save the request to a collection first" }).first()).toBeVisible();
  await u.saveAs("Slides", { newCollection: "Examples" });
  await u.t("save-example").click();
  await u.prompt("happy path");
  await expect(u.t("res-tab-examples")).toContainText("(1)");
  await u.t("res-tab-examples").click();
  await expect(u.t("examples-table")).toContainText("happy path");
  expect(app.cli("docs", "Examples")).toContain("**Example: happy path** — `200 OK`");
  await u.t("example-delete").click();
  await u.t("confirm-ok").click();
  await expect(u.t("res-tab-examples")).not.toContainText("(1)");
});
