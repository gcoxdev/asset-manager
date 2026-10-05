// README screenshots, from the demo catalog (src-tauri/tests/demo_fixtures.rs).
// Not part of the test run: `npm run screenshots` records the demo and
// writes docs/screenshots/*.png.

import { test, expect } from "./harness.mjs";

const OUT = new URL("../../../docs/screenshots/", import.meta.url).pathname;

test.skip(!process.env.SCREENSHOTS, "run with npm run screenshots");
test.use({ viewport: { width: 1360, height: 850 }, deviceScaleFactor: 2 });

const settle = (page) => page.evaluate(() => document.fonts.ready).then(() => page.waitForTimeout(400));

test.describe("locked", () => {
  test.use({ overrides: { vault_status: { unlocked: false, exists: true } } });
  test("unlock screen", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Unlock your vault" })).toBeVisible();
    await settle(page);
    await page.screenshot({ path: `${OUT}/unlock.png` });
  });
});

test("overview", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await expect(page.locator(".chart svg, svg.chart").first()).toBeVisible();
  await settle(page);
  await page.screenshot({ path: `${OUT}/overview.png` });
});

test("holdings", async ({ page }) => {
  await page.goto("/");
  await page.locator("nav").getByRole("button", { name: "Holdings", exact: true }).click();
  await page.getByRole("combobox", { name: "Sort" }).selectOption("value");
  await expect(page.locator("tbody tr").first()).toBeVisible();
  await settle(page);
  await page.screenshot({ path: `${OUT}/holdings.png` });
});
