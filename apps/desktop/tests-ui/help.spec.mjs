// The help: search, navigation, links into the app, the unlock-screen
// dialog — and every topic rendering cleanly.

import { test, expect } from "./harness.mjs";

const openHelp = async (page) => {
  await page.goto("/");
  await page.locator("nav").getByRole("button", { name: "Help", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Help", level: 1 })).toBeVisible();
};

test("searching finds the topic and highlights the words", async ({ page }) => {
  await openHelp(page);
  const search = page.getByRole("searchbox", { name: "Search help" });
  await search.fill("forgot password");
  const first = page.locator(".help-result").first();
  await expect(first).toContainText("If you forget your passphrase");
  await expect(first.locator("mark").first()).toBeVisible();

  await search.press("Enter");
  const article = page.locator(".help-article");
  await expect(article.getByRole("heading", { level: 1 })).toHaveText("If you forget your passphrase");
  await expect(article.locator("mark.help-hit").first()).toBeVisible();
  await article.getByRole("button", { name: /Back to results/ }).click();
  await expect(page.locator(".help-result")).not.toHaveCount(0);

  await search.fill("zzzzqqq");
  await expect(page.getByText("Nothing found for “zzzzqqq”")).toBeVisible();
  await search.press("Escape");
  await expect(page.locator(".help-start")).toBeVisible();
});

test("“?” opens the help with the search ready", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await page.keyboard.press("?");
  await expect(page.getByRole("searchbox", { name: "Search help" })).toBeFocused();
});

test("links lead to other topics and into the app", async ({ page }) => {
  await openHelp(page);
  await page.locator(".help-toc").getByRole("button", { name: "Finding your way around" }).click();
  const article = page.locator(".help-article");
  await article.getByRole("button", { name: "Holdings", exact: true }).first().click();
  await expect(page.getByRole("heading", { name: "Holdings", level: 1 })).toBeVisible();

  await openHelp(page);
  await page.locator(".help-toc").getByRole("button", { name: "Backups" }).click();
  await page.locator(".help-article").getByRole("button", { name: "Verifying a backup" }).first().click();
  await expect(page.locator(".help-article h1")).toHaveText("Verifying a backup");
  await page.locator(".help-pager").getByRole("button", { name: /Next/ }).click();
  await expect(page.locator(".help-article h1")).toHaveText("Restoring a backup or moving computers");
});

test("every topic renders, with no markup left showing", async ({ page }) => {
  await openHelp(page);
  const links = page.locator(".help-toc-link");
  const n = await links.count();
  expect(n).toBeGreaterThan(60);
  const leftovers = [];
  for (let i = 0; i < n; i++) {
    const title = (await links.nth(i).textContent()).trim();
    await links.nth(i).click();
    const article = page.locator(".help-article");
    await expect(article.locator("h1")).toHaveText(title);
    const text = await article.locator(".help-body").innerText();
    if (/«|»|`|\[\[|\]\]|\{\{|\}\}|\*\*|^#|^> |^! /m.test(text)) leftovers.push(title);
  }
  expect(leftovers).toEqual([]);
});

test.describe("on the unlock screen", () => {
  test.use({ overrides: { vault_status: { unlocked: false, exists: true } } });

  test("help opens in a dialog", async ({ page }) => {
    await page.goto("/");
    await page.getByRole("button", { name: "Help", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Help" });
    await expect(dialog.locator(".help-article h1")).toHaveText("Unlocking and locking");
    // No app to go to while locked: screen names are plain text, not links.
    await expect(dialog.locator(".help-article").getByRole("button", { name: "Settings", exact: true })).toHaveCount(0);
    await dialog.getByRole("searchbox", { name: "Search help" }).fill("restore backup");
    await expect(dialog.locator(".help-result").first()).toContainText("Restoring a backup or moving computers");
  });
});

test.describe("when a portable copy has lost its marker", () => {
  test.use({ overrides: { vault_status: { error: { kind: "error", message: "AssetManagerData is beside this copy of Asset Manager, but portable mode is off — the assetmanager-portable marker file is missing." } } } });

  test("the start screen says why, and the help explains portable mode", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Asset Manager could not start" })).toBeVisible();
    await expect(page.getByText(/assetmanager-portable marker file is missing/)).toBeVisible();
    await page.getByRole("button", { name: "Help" }).click();
    await expect(page.getByRole("dialog", { name: "Help" }).locator(".help-article h1")).toHaveText("Portable mode");
  });
});
