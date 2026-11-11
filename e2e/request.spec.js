const { test, expect, ui } = require("./fixtures");

test("welcome screen, then a GET with params kept in sync with the URL", async ({ page, app }) => {
  const u = ui(page);
  await expect(u.t("welcome")).toBeVisible();
  await u.t("welcome-new").click();
  await expect(u.t("response-empty")).toBeVisible();
  await u.t("url").fill(`${app.echo}/get?x=1`);

  // URL -> params
  await expect(u.row("params-table", 0).getByLabel("Key")).toHaveValue("x");
  // params -> URL
  await u.fillRow("params-table", 1, "y", "two");
  await expect(u.t("url")).toHaveValue(`${app.echo}/get?x=1&y=two`);
  await u.row("params-table", 0).getByLabel("Enabled").uncheck();
  await expect(u.t("url")).toHaveValue(`${app.echo}/get?y=two`);
  await expect(u.t("req-tab-params")).toContainText("(1)");

  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"y": "two"');
  await expect(u.t("response-body")).not.toContainText('"x"');
  await expect(u.t("response-meta")).toContainText(/ms|s/);

  // response sub tabs
  await u.t("res-tab-headers").click();
  await expect(u.t("response-headers")).toContainText("content-type");
  await u.t("res-tab-request").click();
  await expect(u.t("sent-request")).toContainText(`GET ${app.echo}/get?y=two`);
  await u.t("res-tab-body").click();
  await u.t("body-view-raw").click();
  await expect(u.t("response-body")).toContainText('"y": "two"');
});

test("headers, path variables and Enter-to-send", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/anything/:kind/:id`);
  await expect(u.t("path-vars").locator("tbody tr")).toHaveCount(2);
  await u.t("path-vars").getByLabel("Value of kind").fill("users");
  await u.t("path-vars").getByLabel("Value of id").fill("42");
  await u.t("req-tab-headers").click();
  await u.fillRow("headers-table", 0, "X-Test", "abc");
  await u.fillRow("headers-table", 1, "X-Off", "nope");
  await u.row("headers-table", 1).getByLabel("Enabled").uncheck();
  await expect(u.t("req-tab-headers")).toContainText("(1)");
  await u.t("url").press("Enter");
  await expect(u.t("response-status")).toHaveText("200 OK");
  await expect(u.t("response-body")).toContainText('"path": "/anything/users/42"');
  await expect(u.t("response-body")).toContainText('"x-test": "abc"');
  await expect(u.t("response-body")).not.toContainText("x-off");
});

test("request bodies: raw JSON, urlencoded, form-data with file upload, GraphQL", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/post`, "POST");
  await u.t("req-tab-body").click();

  await u.t("body-mode-raw").check();
  await u.t("body-raw").fill('{"name":"Ann","tags":[1,2]}');
  await u.t("body-beautify").click();
  await expect(u.t("body-raw")).toHaveValue('{\n  "name": "Ann",\n  "tags": [\n    1,\n    2\n  ]\n}');
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"content-type": "application/json"');
  await expect(u.t("response-body")).toContainText('"name": "Ann"');

  await u.t("body-raw").fill("{not json");
  await u.t("body-beautify").click();
  await expect(u.t("toast").filter({ hasText: "not valid JSON" }).first()).toBeVisible();

  await u.t("body-mode-urlencoded").check();
  await u.fillRow("body-urlencoded", 0, "city", "São Paulo & co");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"city": "São Paulo & co"');
  await expect(u.t("response-body")).toContainText("application/x-www-form-urlencoded");

  await u.t("body-mode-formdata").check();
  await u.fillRow("body-formdata", 0, "note", "hello");
  const fileRow = u.row("body-formdata", 1);
  await fileRow.locator("td:nth-child(2) input").fill("attachment");
  await fileRow.getByLabel("Type").selectOption("file");
  await fileRow.locator("input[type=file]").setInputFiles({ name: "hello.txt", mimeType: "text/plain", buffer: Buffer.from("file-body-123") });
  await expect(fileRow.getByLabel("File path")).toHaveValue(/hello\.txt$/);
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"note": "hello"');
  await expect(u.t("response-body")).toContainText('"text": "file-body-123"');

  await u.t("body-mode-graphql").check();
  await u.t("body-graphql-query").fill("query Q($id: ID!) { user(id: $id) { name } }");
  await u.t("body-graphql-vars").fill('{"id": 5}');
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"id": 5');

  await u.t("body-mode-none").check();
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"data": ""');
});

test("authorization helpers", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/basic-auth/bob/s3cret`);
  await u.send("401 Unauthorized");
  await u.t("req-tab-auth").click();
  await u.t("auth-type").selectOption("basic");
  await u.t("auth-username").fill("bob");
  await u.t("auth-password").fill("s3cret");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"authenticated": true');

  // switching the type keeps shared fields, digest answers the challenge
  await u.t("url").fill(`${app.echo}/digest-auth/bob/s3cret`);
  await u.t("auth-type").selectOption("digest");
  await expect(u.t("auth-username")).toHaveValue("bob");
  await u.send("200 OK");

  await u.t("url").fill(`${app.echo}/headers`);
  await u.t("auth-type").selectOption("apikey");
  await u.t("auth-key").fill("X-Api-Key");
  await u.t("auth-value").fill("k-123");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"x-api-key": "k-123"');

  await u.t("auth-type").selectOption("bearer");
  await u.t("auth-token").fill("tok");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"authorization": "Bearer tok"');

  await u.t("auth-type").selectOption("jwt");
  await u.t("auth-secret").fill("secret");
  await u.t("auth-payload").fill('{"sub":"1"}');
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText(/"authorization": "Bearer eyJ[\w-]+\.eyJ[\w-]+\.[\w-]+"/);
});

test("errors are shown, not swallowed", async ({ page }) => {
  const u = ui(page);
  await u.t("tab-new").click();
  await u.t("send").click();
  await expect(u.t("toast").filter({ hasText: "Enter a URL first" }).first()).toBeVisible();

  await u.t("url").fill("http://127.0.0.1:1/unreachable");
  await u.t("send").click();
  await expect(u.t("response-error")).toContainText("Could not get a response");
  await expect(u.t("response-status")).toHaveText("Error");

  await u.t("url").fill("{{undefined_host}}/x");
  await u.t("send").click();
  await expect(u.t("response-error")).toBeVisible();
});

test("response viewer: html preview, binary, cookies, redirects", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/html`);
  await u.send("200 OK");
  await u.t("body-view-preview").click();
  await expect(page.frameLocator("[data-testid=response-preview]").locator("h1")).toHaveText("Hello from rsPostman");
  await u.t("body-view-pretty").click();
  await expect(u.t("response-body")).toContainText("<h1>Hello from rsPostman</h1>");

  await u.t("url").fill(`${app.echo}/bytes/2048`);
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText("Binary response (2.00 KB");
  const download = page.waitForEvent("download");
  await u.t("response-download").click();
  expect((await download).suggestedFilename()).toBe("response.octet-stream");

  await u.t("url").fill(`${app.echo}/cookies/set?flavour=choc`);
  await u.send("200 OK");
  await expect(u.t("response-meta")).toContainText("1 redirect");
  await expect(u.t("response-body")).toContainText('"flavour": "choc"');
  await u.t("res-tab-cookies").click();
  await expect(u.t("response-cookies")).toContainText("flavour");

  await u.t("url").fill(`${app.echo}/status/503`);
  await u.send("503 Service Unavailable");
  await u.t("url").fill(`${app.echo}/xml`);
  await u.send("200 OK");
  await u.t("res-tab-body").click();
  await expect(u.t("response-body")).toContainText('<slide type="all">');
});
