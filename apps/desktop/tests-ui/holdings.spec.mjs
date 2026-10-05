// Holdings at scale: a large catalog renders a chunk at a time, stays
// responsive, and selection covers every match rather than what is drawn.

import { test, expect, callsTo, fixtures } from "./harness.mjs";

const N = 5000;
const catalog = () => {
  const [one] = fixtures().calls.list_assets[0].result.filter((a) => a.name.startsWith("Amazing"));
  return Array.from({ length: N }, (_, i) => ({
    ...one,
    asset_id: `id-${i}`,
    name: `Item ${i}`,
    category: i % 2 ? "collectibles" : "valuables",
    tags: i % 5 ? [] : ["insured"],
  }));
};

test.use({ overrides: { list_assets: catalog() } });

/** Run an action in the page; milliseconds until the next frame is drawn. */
const timed = (page, action) =>
  page.evaluate(async (src) => {
    const [selector, text] = src;
    const el = [...document.querySelectorAll(selector)].find((e) => !text || e.textContent.includes(text));
    const start = performance.now();
    el.click();
    await new Promise((r) => requestAnimationFrame(() => setTimeout(r)));
    return performance.now() - start;
  }, action);

async function openHoldings(page) {
  await page.goto("/");
  await page.locator("nav").getByRole("button", { name: "Holdings", exact: true }).click();
  await expect(page.locator(".results-summary")).toContainText(`${N} holdings`);
}

test("a large catalog renders a chunk at a time and loads more on scroll", async ({ page }) => {
  await openHoldings(page);
  await expect(page.locator("tbody tr")).toHaveCount(200);
  await expect(page.locator(".load-more")).toContainText(`Showing 200 of ${N.toLocaleString()}`);
  await page.locator(".load-more").scrollIntoViewIfNeeded();
  await expect(page.locator("tbody tr")).toHaveCount(400);
  // The button asks for the next chunk too. (Pressed in place: scrolling to
  // it would load the next chunk first and move it.)
  const before = await page.locator("tbody tr").count();
  await page.getByRole("button", { name: "Show more" }).evaluate((b) => b.click());
  await expect(page.locator("tbody tr")).toHaveCount(before + 200);

  // The card grid pages the same way.
  await page.getByRole("button", { name: "Grid", exact: true }).click();
  await expect(page.locator(".asset-card")).toHaveCount(200);
  await page.locator(".load-more").scrollIntoViewIfNeeded();
  await expect(page.locator(".asset-card")).toHaveCount(400);
});

test("filtering, paging and selecting stay quick", async ({ page }) => {
  await openHoldings(page);
  // Budgets far above what this takes (~130 ms), well below the seconds
  // that rendering everything at once used to cost.
  expect(await timed(page, [".chip", "Valuables"])).toBeLessThan(1500);
  expect(await timed(page, [".load-more button"])).toBeLessThan(1500);
  expect(await timed(page, ["button", "Select"])).toBeLessThan(1500);
  expect(await timed(page, ["input[data-select]"])).toBeLessThan(300);
});

test("select all takes every match, not just the rows drawn", async ({ page, answers }) => {
  answers.list_sets = [];
  answers.save_set = "set-1";
  await openHoldings(page);
  await page.locator(".chip", { hasText: "Valuables" }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("button", { name: `Select all ${(N / 2).toLocaleString()}` }).click();
  await expect(page.locator(".selection-bar strong")).toHaveText(`${N / 2} selected`);
  await expect(page.locator("input[data-select]").first()).toBeChecked();
  // A row toggled off drops out; the rest stay.
  await page.locator("input[data-select]").first().uncheck();
  await expect(page.locator(".selection-bar strong")).toHaveText(`${N / 2 - 1} selected`);

  await page.getByRole("button", { name: "Add to set…" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("textbox").fill("Valuables");
  await dialog.getByRole("button", { name: "Add" }).click();
  const [args] = await callsTo(page, "save_set");
  expect(args.add).toHaveLength(N / 2 - 1);
  expect(args.add).not.toContain("id-0");
});

test("bulk value entry saves only what was changed", async ({ page, answers }) => {
  answers.set_prices = (args) => args.entries.map((e) => ({ asset_id: e.asset_id, ok: true, error: null }));
  await openHoldings(page);
  await page.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("menuitem", { name: "Update many values…" }).click();
  await page.getByRole("textbox", { name: "Value of Item 3", exact: true }).fill("42.50");
  await page.getByRole("button", { name: "Save values" }).click();
  const [args] = await callsTo(page, "set_prices");
  expect(args.entries).toEqual([expect.objectContaining({ asset_id: "id-3", amount: "42.50", currency: "USD" })]);
});
