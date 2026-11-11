const { test, expect, ui } = require("./fixtures");

test("help center: topics, screenshots, navigation and search", async ({ page }) => {
  const u = ui(page);
  await u.t("welcome-help").click();
  await expect(u.t("help-content").locator("h2, p").first()).toBeVisible();
  await expect(u.t("help-content")).toContainText("runs entirely on your computer");
  const topics = u.t("help-topic");
  expect(await topics.count()).toBeGreaterThan(20);
  await expect(topics.first()).toHaveText("Welcome");
  await expect(u.t("help-nav")).toContainText("Scripting reference");

  // a chapter with a table, a screenshot that really loads, and code
  await topics.filter({ hasText: "Sending requests" }).click();
  await expect(u.t("help-content").locator("h2")).toHaveText("Sending requests");
  await expect(page.getByTestId("tab").filter({ hasText: "Help – Sending requests" })).toBeVisible();
  const images = u.t("help-content").locator("img");
  expect(await images.count()).toBeGreaterThanOrEqual(3);
  for (const img of await images.all()) {
    await img.scrollIntoViewIfNeeded();
    await expect.poll(() => img.evaluate((i) => i.complete && i.naturalWidth)).toBeGreaterThan(400);
    expect(await img.getAttribute("alt")).not.toBe("");
  }
  await expect(u.t("help-content").locator("table").first()).toContainText("form-data");

  // previous / next
  await u.t("help-next").click();
  await expect(u.t("help-content").locator("h2")).toHaveText("Reading the response");
  await u.t("help-prev").click();
  await expect(u.t("help-content").locator("h2")).toHaveText("Sending requests");

  // links inside the text move between chapters, including into the scripting reference
  await u.t("help-content").getByRole("link", { name: "Variables and environments" }).first().click();
  await expect(u.t("help-content").locator("h2")).toHaveText("Variables and environments");
  await topics.filter({ hasText: "Scripts and tests" }).click();
  await expect(u.t("help-content").locator("pre.code").first()).toContainText("pm.test(");
  await u.t("help-content").getByRole("link", { name: "scripting reference" }).click();
  await expect(u.t("help-content").locator("h2")).toHaveText("Scripting reference");
  await expect(u.t("help-content")).toContainText("pm.sendRequest");

  // search: results with context, click goes to the place, no-result message
  await u.t("help-search").fill("x-mock-response-code");
  await expect(u.t("help-hit").first()).toContainText("Mock servers");
  await u.t("help-hit").first().click();
  await expect(u.t("help-content").locator("h2")).toHaveText("Mock servers");
  await expect(u.t("help-search")).toHaveValue("");
  await u.t("help-search").fill("p95");
  expect(await u.t("help-hit").count()).toBeGreaterThan(1);
  await expect(u.t("help-nav")).toContainText("Benchmark");
  await expect(u.t("help-nav")).not.toContainText("WebSocket");
  await u.t("help-search").fill("zzzz-not-in-the-manual");
  await expect(u.t("help-no-results")).toBeVisible();
  await u.t("help-search").fill("");
  await expect(u.t("help-content").locator("h2")).toHaveText("Mock servers");
});

test("help center: F1, the ? button and page help open the right chapter", async ({ page }) => {
  const u = ui(page);
  await page.keyboard.press("F1");
  await expect(u.t("help")).toBeVisible();
  await page.getByTestId("tab-close").first().click();

  await u.t("help-open").click();
  await expect(u.t("help")).toBeVisible();
  await expect(page.getByTestId("tab")).toHaveCount(1);

  // context help: each tool page links to its chapter and reuses the one help tab
  for (const [tool, title] of [["runner", "Collection runner"], ["mock", "Mock servers"], ["benchmark", "Benchmark"], ["settings", "Settings"], ["cookies", "Cookies"], ["profile", "Profile an LLM provider"], ["ws", "WebSocket"]]) {
    await u.t("tool-" + tool).click();
    await page.locator(".page").last().getByTestId("page-help").click();
    await expect(u.t("help-content").locator("h2")).toHaveText(title);
  }
  await u.t("vault-open").click();
  await page.locator(".page").last().getByTestId("page-help").click();
  await expect(u.t("help-content").locator("h2")).toHaveText("Vault and credentials");
  await expect(page.getByTestId("tab").filter({ hasText: "Help –" })).toHaveCount(1);
});

test("help center: document text cannot inject markup", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const { render, slug } = await import("/js/md.js");
    const ctx = { image: (s) => s, id: (t) => "x-" + slug(t), link: (href) => ({ href, external: true }) };
    const host = document.createElement("div");
    host.append(render('## A <img src=x onerror="window.pwned=1"> heading\n\nText with <script>window.pwned=2</script> and `<b>code</b>` and [a link](javascript:alert(1)).\n\n| a | b |\n|---|---|\n| <i>x</i> | **bold** |\n\n- item <u>one</u>\n  - nested\n\n```html\n<script>window.pwned=3</script>\n```\n\n> note\n', ctx));
    document.body.append(host);
    await new Promise((r) => setTimeout(r, 50));
    return {
      pwned: window.pwned,
      injected: host.querySelectorAll("script, i, u, b, img").length,
      links: host.querySelectorAll("a").length,
      linkText: host.querySelector("p").textContent.includes("a link"),
      heading: host.querySelector("h2").textContent,
      nested: host.querySelectorAll("ul ul li").length,
      strong: host.querySelectorAll("td strong").length,
      note: host.querySelectorAll(".note").length,
      code: host.querySelector("pre code").textContent,
      slug: slug("Import, export & `code` snippets"),
    };
  });
  expect(result).toEqual({ pwned: undefined, injected: 0, links: 0, linkText: true, heading: 'A <img src=x onerror="window.pwned=1"> heading', nested: 1, strong: 1, note: 1, code: "<script>window.pwned=3</script>", slug: "import-export-code-snippets" });
});
