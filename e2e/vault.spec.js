const { test, expect, ui } = require("./fixtures");
const fs = require("fs");
const path = require("path");

const PASSPHRASE = "correct horse battery";

function workspaceText(dir) {
  let out = "";
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    out += e.isDirectory() ? workspaceText(p) : fs.readFileSync(p, "utf8");
  }
  return out;
}

// One test: the vault is a per-workspace singleton, so the steps build on each other.
test("vault: create, store, use in a request, lock, unlock, change passphrase", async ({ page, app }) => {
  const u = ui(page);
  await expect(u.t("vault-open")).toHaveAttribute("data-state", "none");

  // a request that needs the vault before it exists
  await u.newRequest(`${app.echo}/bearer`);
  await u.t("req-tab-auth").click();
  await u.t("auth-type").selectOption("bearer");
  await u.t("auth-token").fill("{{vault:api_token}}");
  await expect(u.t("auth-token")).toHaveAttribute("type", "text"); // a reference, not a secret
  await u.t("send").click();
  await expect(u.t("response-error")).toContainText("vault secret `api_token` is not available");
  await u.t("error-open-vault").click();

  // create
  await u.t("vault-new-pass").fill(PASSPHRASE);
  await u.t("vault-new-pass2").fill("something else");
  await u.t("vault-create").click();
  await expect(u.t("toast").filter({ hasText: "do not match" }).first()).toBeVisible();
  await u.t("vault-new-pass").fill("short");
  await u.t("vault-new-pass2").fill("short");
  await u.t("vault-create").click();
  await expect(u.t("toast").filter({ hasText: "at least 8 characters" }).first()).toBeVisible();
  await u.t("vault-new-pass").fill(PASSPHRASE);
  await u.t("vault-new-pass2").fill(PASSPHRASE);
  await expect(u.t("vault-new-pass")).toHaveAttribute("type", "password");
  await u.t("vault-create").click();
  await expect(u.t("vault-state")).toHaveText("Unlocked");
  await expect(u.t("vault-open")).toHaveAttribute("data-state", "unlocked");

  // store secrets; values are masked while typing and never listed
  await u.t("vault-add").click();
  await expect(u.t("toast").filter({ hasText: "Enter a name and a value" }).first()).toBeVisible();
  await u.t("vault-name").fill("bad name");
  await u.t("vault-value").fill("x");
  await u.t("vault-add").click();
  await expect(u.t("toast").filter({ hasText: "invalid secret name" }).first()).toBeVisible();
  await u.t("vault-name").fill("api_token");
  await u.t("vault-value").fill("GUI-VAULT-TOKEN-1");
  await expect(u.t("vault-value")).toHaveAttribute("type", "password");
  await u.t("vault-add").click();
  await expect(u.t("vault-row")).toHaveCount(1);
  await expect(u.t("vault-secrets")).toContainText("{{vault:api_token}}");
  await expect(u.t("vault")).not.toContainText("GUI-VAULT-TOKEN-1");
  await u.t("vault-name").fill("other");
  await u.t("vault-value").fill("to-be-deleted");
  await u.t("vault-add").click();
  await expect(u.t("vault-row")).toHaveCount(2);
  await u.t("vault-row").filter({ hasText: "other" }).getByTestId("vault-delete").click();
  await u.t("confirm-ok").click();
  await expect(u.t("vault-row")).toHaveCount(1);

  // the request now works; the secret reaches the server but not the request snapshot
  await page.getByTestId("tab").filter({ hasText: "Untitled request" }).click();
  await u.send("200 OK");
  await expect(u.t("response-body")).toContainText('"token": "GUI-VAULT-TOKEN-1"');
  await u.t("res-tab-request").click();
  await expect(u.t("sent-request")).not.toContainText("GUI-VAULT-TOKEN-1");
  await u.t("sent-request-reveal").click();
  await expect(u.t("sent-request")).toContainText("Authorization: Bearer <redacted>");
  await expect(u.t("sent-request")).not.toContainText("GUI-VAULT-TOKEN-1");

  // nothing on disk in clear: vault, history, anything
  const disk = workspaceText(app.home);
  expect(disk).not.toContain("GUI-VAULT-TOKEN-1");
  expect(disk).not.toContain(PASSPHRASE);
  expect(disk).toContain("{{vault:api_token}}");
  // the CLI shares the vault
  expect(app.cli("vault", "list")).toBe("api_token\n");

  // lock: requests stop, with a way back
  await u.t("vault-open").click();
  await u.t("vault-lock").click();
  await expect(u.t("vault-state")).toHaveText("Locked");
  await expect(u.t("vault-open")).toHaveAttribute("data-state", "locked");
  await expect(u.t("vault-delete")).toHaveCount(0);
  await page.getByTestId("tab").filter({ hasText: "Untitled request" }).click();
  await u.t("send").click();
  await expect(u.t("response-error")).toContainText("the vault is locked or has no secret");
  await u.t("error-open-vault").click();
  await u.t("vault-pass").fill("not the passphrase");
  await u.t("vault-unlock").click();
  await expect(u.t("toast").filter({ hasText: "wrong vault passphrase" }).first()).toBeVisible();
  await u.t("vault-pass").fill(PASSPHRASE);
  await u.t("vault-pass").press("Enter");
  await expect(u.t("vault-state")).toHaveText("Unlocked");

  // the lock state survives a page reload (it lives in the backend), not the secrets in the page
  await page.reload();
  await page.waitForSelector('body[data-ready="1"]');
  await expect(u.t("vault-open")).toHaveAttribute("data-state", "unlocked");

  // change the passphrase
  await u.t("vault-open").click();
  await page.getByText("Change passphrase", { exact: true }).first().click();
  await u.t("vault-cur-pass").fill("wrong current one");
  await u.t("vault-next-pass").fill("a second long passphrase");
  await u.t("vault-passwd").click();
  await expect(u.t("toast").filter({ hasText: "wrong vault passphrase" }).first()).toBeVisible();
  await u.t("vault-cur-pass").fill(PASSPHRASE);
  await u.t("vault-passwd").click();
  await expect(u.t("toast").filter({ hasText: "Passphrase changed" }).first()).toBeVisible();
  await u.t("vault-lock").click();
  await u.t("vault-pass").fill(PASSPHRASE);
  await u.t("vault-unlock").click();
  await expect(u.t("toast").filter({ hasText: "wrong vault passphrase" }).first()).toBeVisible();
  await u.t("vault-pass").fill("a second long passphrase");
  await u.t("vault-unlock").click();
  await expect(u.t("vault-state")).toHaveText("Unlocked");
  await expect(u.t("vault-row")).toHaveCount(1);
});
