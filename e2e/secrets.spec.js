const { test, expect, ui } = require("./fixtures");
const fs = require("fs");
const path = require("path");

test("auth secrets are masked, revealable, and references stay readable", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/basic-auth/bob/pw-visible-nowhere`);
  await u.t("req-tab-auth").click();
  await u.t("auth-type").selectOption("basic");
  await u.t("auth-username").fill("bob");
  await expect(u.t("auth-username")).toHaveAttribute("type", "text");
  const pw = u.t("auth-password");
  await pw.fill("pw-visible-nowhere");
  await expect(pw).toHaveAttribute("type", "password");
  const reveal = pw.locator("xpath=..").getByRole("button", { name: "Show value" });
  await reveal.click();
  await expect(pw).toHaveAttribute("type", "text");
  await pw.locator("xpath=..").getByRole("button", { name: "Hide value" }).click();
  await expect(pw).toHaveAttribute("type", "password");
  // a {{variable}} reference is not a secret and is shown as typed
  await pw.fill("{{password}}");
  await expect(pw).toHaveAttribute("type", "text");
  await pw.fill("pw-visible-nowhere");
  await u.send("200 OK");

  // the sent request hides the Authorization value until asked
  await u.t("res-tab-request").click();
  await expect(u.t("sent-request")).toContainText("Authorization: Basic ••••••••");
  await expect(u.t("sent-request")).not.toContainText("Ym9iOnB3");
  await u.t("sent-request-reveal").click();
  await expect(u.t("sent-request")).toContainText("Authorization: Basic Ym9iOnB3");
});

test("sensitive header and form values are masked in tables", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/post`, "POST");
  await u.t("req-tab-headers").click();
  await u.fillRow("headers-table", 0, "X-Api-Key", "k-secret");
  await u.fillRow("headers-table", 1, "X-Trace", "plain");
  await u.fillRow("headers-table", 2, "Authorization", "{{auth}}");
  const value = (n) => u.row("headers-table", n).getByLabel("Value", { exact: true });
  await expect(value(0)).toHaveAttribute("type", "password");
  await expect(value(1)).toHaveAttribute("type", "text");
  await expect(value(2)).toHaveAttribute("type", "text");
  // renaming the key updates the masking
  await u.row("headers-table", 1).locator("td:nth-child(2) input").fill("X-Auth-Token");
  await expect(value(1)).toHaveAttribute("type", "password");
  await u.row("headers-table", 2).getByLabel("Enabled").uncheck();

  await u.t("req-tab-body").click();
  await u.t("body-mode-urlencoded").check();
  await u.fillRow("body-urlencoded", 0, "username", "bob");
  await u.fillRow("body-urlencoded", 1, "password", "form-pw");
  await expect(u.row("body-urlencoded", 0).getByLabel("Value", { exact: true })).toHaveAttribute("type", "text");
  await expect(u.row("body-urlencoded", 1).getByLabel("Value", { exact: true })).toHaveAttribute("type", "password");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"x-api-key": "k-secret"');
  await expect(u.t("response-body")).toContainText('"password": "form-pw"');
});

test("history does not keep credentials; reopening an entry says so", async ({ page, app }) => {
  const u = ui(page);
  await u.newRequest(`${app.echo}/bearer?api_key=QUERY-KEY-77`);
  await u.t("req-tab-auth").click();
  await u.t("auth-type").selectOption("bearer");
  await u.t("auth-token").fill("BEARER-TOKEN-77");
  await u.t("req-tab-headers").click();
  await u.fillRow("headers-table", 0, "X-Api-Key", "HEADER-KEY-77");
  await u.send("200 OK");

  const file = path.join(app.home, "history.jsonl");
  await expect.poll(() => fs.existsSync(file) && fs.readFileSync(file, "utf8").includes("/bearer")).toBe(true);
  const stored = fs.readFileSync(file, "utf8");
  for (const secret of ["BEARER-TOKEN-77", "HEADER-KEY-77", "QUERY-KEY-77"]) expect(stored).not.toContain(secret);
  expect(stored).toContain("<redacted>");
  if (process.platform !== "win32") expect(fs.statSync(file).mode & 0o777).toBe(0o600);

  await u.t("side-history").click();
  await u.t("history-row").first().click();
  await expect(u.t("toast").filter({ hasText: "Credentials are not stored in the history" }).first()).toBeVisible();
  await expect(u.t("url")).toHaveValue(`${app.echo}/bearer?api_key=<redacted>`);
  await page.getByTestId("request-view").last().getByTestId("req-tab-auth").click();
  await expect(page.getByTestId("request-view").last().getByTestId("auth-token")).toHaveValue("<redacted>");
});

test("secret variables: auto-marked, masked, stored with type secret, hidden by the CLI", async ({ page, app }) => {
  const u = ui(page);
  await u.t("side-environments").click();
  await u.t("new-environment").click();
  await u.prompt("secrets-env");
  await u.fillRow("environment-variables", 0, "host", app.echo);
  await u.fillRow("environment-variables", 1, "api_token", "tok-e2e-1");
  const row = (n) => u.row("environment-variables", n);
  await expect(row(0).getByLabel("Secret")).not.toBeChecked();
  await expect(row(1).getByLabel("Secret")).toBeChecked();
  await expect(row(1).getByLabel("Value", { exact: true })).toHaveAttribute("type", "password");
  // any variable can be marked by hand, and the automatic choice can be undone
  await row(0).getByLabel("Secret").check();
  await expect(row(0).getByLabel("Value", { exact: true })).toHaveAttribute("type", "password");
  await row(0).getByLabel("Secret").uncheck();
  await expect(row(0).getByLabel("Value", { exact: true })).toHaveAttribute("type", "text");
  await u.t("environment-save").click();
  await expect(u.t("toast").filter({ hasText: "Saved" }).first()).toBeVisible();

  const shown = app.cli("env", "show", "secrets-env");
  expect(shown).toContain("********");
  expect(shown).not.toContain("tok-e2e-1");
  expect(app.cli("env", "show", "secrets-env", "--show-secrets")).toContain("tok-e2e-1");
  const exported = JSON.parse(app.cli("export", "env", "secrets-env"));
  expect(exported.values.find((v) => v.key === "api_token").type).toBe("secret");
  expect(exported.values.find((v) => v.key === "host").type).toBeUndefined();

  // still usable in requests
  await u.t("env-select").selectOption({ label: "secrets-env" });
  await u.newRequest("{{host}}/bearer");
  await u.t("req-tab-auth").click();
  await u.t("auth-type").selectOption("bearer");
  await u.t("auth-token").fill("{{api_token}}");
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"token": "tok-e2e-1"');
  await u.t("env-select").selectOption({ label: "No environment" });

  // the marker survives a reload
  await page.reload();
  await page.waitForSelector('body[data-ready="1"]');
  await u.t("side-environments").click();
  await page.locator('[data-testid=env-row][data-name="secrets-env"]').click();
  await expect(u.row("environment-variables", 1).getByLabel("Secret")).toBeChecked();
  await expect(u.row("environment-variables", 1).getByLabel("Value", { exact: true })).toHaveAttribute("type", "password");
});

test("settings expose the history opt-in", async ({ page, app }) => {
  const u = ui(page);
  await u.t("tool-settings").click();
  await expect(u.t("set-history_store_secrets")).not.toBeChecked();
  expect(JSON.parse(app.cli("settings", "show")).history_store_secrets).toBe(false);
});
