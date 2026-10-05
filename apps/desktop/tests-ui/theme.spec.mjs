// The theme: System follows the computer; Light and Dark override it, are
// remembered, and apply from the very first screen.

import { test, expect, callsTo } from "./harness.mjs";

/** Whether the page is drawn dark, judged by its background. */
const isDark = (page) =>
  page.evaluate(() => {
    const [r, g, b] = getComputedStyle(document.body).backgroundColor.match(/\d+/g).map(Number);
    return (r + g + b) / 3 < 80;
  });

const openAppearance = async (page) => {
  await page.locator("nav").getByRole("button", { name: "Settings", exact: true }).click();
  return page.locator("#appearance");
};

test.describe("on a computer set to dark", () => {
  test.use({ colorScheme: "dark" });

  test("System follows it; Light and Dark override it and are saved", async ({ page, answers }) => {
    answers.set_theme = (args) => ({ theme: args.theme });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
    expect(await isDark(page)).toBe(true);

    const card = await openAppearance(page);
    await expect(card.getByRole("button", { name: "System" })).toHaveAttribute("aria-pressed", "true");

    await card.getByRole("button", { name: "Light" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    expect(await isDark(page)).toBe(false);

    await card.getByRole("button", { name: "System" }).click();
    await expect(page.locator("html")).not.toHaveAttribute("data-theme", /./);
    expect(await isDark(page)).toBe(true);

    expect(await callsTo(page, "set_theme")).toEqual([{ theme: "light" }, { theme: "system" }]);
  });
});

test.describe("on a computer set to light, with Dark chosen", () => {
  test.use({ colorScheme: "light", overrides: { get_preferences: { theme: "dark" }, vault_status: { unlocked: false, exists: true } } });

  test("the unlock screen is already dark", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Unlock your vault" })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    expect(await isDark(page)).toBe(true);
  });
});
