// Workflows through the real frontend: the shell, holdings, an asset, its
// documents, sets and reports — with layout checks for regressions a unit
// test cannot see.

import { readFileSync } from "node:fs";
import { test, expect, callsTo, fixtures } from "./harness.mjs";

const ids = () => fixtures().ids;

/** Go to a section with the sidebar. */
const nav = (page, name) => page.locator("nav").getByRole("button", { name, exact: true }).click();

/** Open an asset from Holdings. */
async function openAsset(page, name) {
  await nav(page, "Holdings");
  await page.getByRole("link", { name }).or(page.getByRole("button", { name })).or(page.getByText(name, { exact: true })).first().click();
  await expect(page.getByRole("heading", { name, level: 1 })).toBeVisible();
}

test("opens into the overview and lists holdings", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
  await nav(page, "Holdings");
  await expect(page.getByText("Gold Eagles").first()).toBeVisible();
  await expect(page.getByText("Amazing Fantasy #15").first()).toBeVisible();
});

test.describe("when the vault is locked", () => {
  test.use({ overrides: { vault_status: { unlocked: false, exists: true } } });

  test("unlocks with the passphrase and enters the app", async ({ page, answers }) => {
    answers.unlock_vault = null;
    await page.goto("/");
    const pass = page.getByRole("textbox", { name: "Passphrase" });
    await pass.fill("correct horse battery staple");
    await pass.press("Enter");
    await expect(page.getByRole("heading", { name: "Overview" })).toBeVisible();
    expect(await callsTo(page, "unlock_vault")).toEqual([{ secret: "correct horse battery staple", useRecoveryKey: false }]);
  });
});

test("an auto-lock clears everything decrypted from the screen", async ({ page }) => {
  await page.goto("/");
  await nav(page, "Holdings");
  await expect(page.getByText("Gold Eagles").first()).toBeVisible();
  await page.evaluate(() => window.__emit("vault-auto-locked", null));
  await expect(page.getByText("Locked after inactivity.")).toBeVisible();
  await expect(page.locator("body")).not.toContainText("Gold Eagles");
});

test("a toast's close button is centred on its first line", async ({ page, answers }) => {
  answers.save_set = "new-set";
  await page.goto("/");
  await nav(page, "Holdings");
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select Gold Eagles" }).check();
  await page.getByRole("button", { name: "Add to set…" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Add" }).click();
  const toast = page.locator(".toast").first();
  await expect(toast).toBeVisible();
  await page.waitForTimeout(250); // entry animation
  const [close, icon, line] = await Promise.all([
    toast.locator(".toast-close").boundingBox(),
    toast.locator("> .icon").boundingBox(),
    toast.locator(".toast-text").evaluate((el) => {
      const range = document.createRange();
      range.setStart(el.firstChild, 0);
      range.setEnd(el.firstChild, 1);
      const r = range.getClientRects()[0];
      return { y: r.y, height: r.height };
    }),
  ]);
  const mid = (b) => b.y + b.height / 2;
  expect(Math.abs(mid(close) - mid(line))).toBeLessThanOrEqual(1.5);
  expect(Math.abs(mid(icon) - mid(line))).toBeLessThanOrEqual(1.5);
  expect(await callsTo(page, "save_set")).toEqual([{ setId: ids().set, name: "Gold", targetCount: 20, notes: "", add: [ids().eagles] }]);
});

test("the asset form's action bar is whole at every scroll position", async ({ page }) => {
  await page.goto("/");
  await openAsset(page, "Amazing Fantasy #15");
  await page.getByRole("button", { name: "Edit" }).first().click();
  const dialog = page.getByRole("dialog");
  const body = dialog.locator(".modal-body");
  const bar = dialog.locator(".form-actions");
  await expect(bar).toBeVisible();
  for (const where of ["top", "middle", "bottom"]) {
    await body.evaluate((el, w) => { el.scrollTop = w === "top" ? 0 : w === "middle" ? el.scrollHeight / 2 - el.clientHeight / 2 : el.scrollHeight; }, where);
    const [b, d] = await Promise.all([bar.boundingBox(), body.boundingBox()]);
    expect(b.y + b.height, `bar inside the dialog when scrolled to the ${where}`).toBeLessThanOrEqual(d.y + d.height + 0.5);
    const padding = await bar.evaluate((el) => parseFloat(getComputedStyle(el).paddingBottom));
    const button = await bar.locator(".btn-primary").boundingBox();
    expect(d.y + d.height - (button.y + button.height), `padding below the buttons at the ${where}`).toBeGreaterThanOrEqual(padding - 0.5);
  }
});

test("a PDF document opens in the viewer and renders its pages", async ({ page }) => {
  await page.goto("/");
  await openAsset(page, "Amazing Fantasy #15");
  await page.getByRole("button", { name: "Appraisal 2026", exact: true }).click();
  const viewer = page.getByRole("dialog", { name: "Appraisal 2026" });
  await expect(viewer.getByText("Page 1 of 2")).toBeVisible();
  // The first page is drawn: dark text pixels on white.
  await expect.poll(() => viewer.locator(".pdf-page canvas").first().evaluate((c) => {
    if (!c.width) return 0;
    const data = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
    let dark = 0;
    for (let i = 0; i < data.length; i += 4) if (data[i] < 100 && data[i + 3] > 0) dark++;
    return dark;
  }), { timeout: 10_000 }).toBeGreaterThan(200);
  await expect(viewer.getByRole("img", { name: "Page 2 of 2" })).toHaveCount(1);

  await viewer.getByRole("button", { name: "Zoom in" }).click();
  await expect(viewer.locator(".pdf-zoom")).toHaveText(/%$/);
  await viewer.getByRole("button", { name: "Fit width" }).click();
  await expect(viewer.locator(".pdf-zoom")).toHaveText("Fit");
  await expect(viewer.getByRole("button", { name: "Save a copy…" })).toBeVisible();

  // Closing leaves no decrypted page behind.
  await page.keyboard.press("Escape");
  await expect(viewer).toHaveCount(0);
  await expect(page.locator(".pdf-page canvas")).toHaveCount(0);
});

test("a password-protected PDF says so instead of failing", async ({ page }) => {
  // The sample appraisal, encrypted with qpdf (user password "secret").
  const encrypted = readFileSync(new URL("./encrypted.pdf", import.meta.url));
  await page.route("http://asset.localhost/media/**", (route) => route.fulfill({ status: 200, contentType: "application/pdf", body: encrypted }));
  await page.goto("/");
  await openAsset(page, "Amazing Fantasy #15");
  await page.getByRole("button", { name: "Appraisal 2026", exact: true }).click();
  await expect(page.getByRole("dialog").locator(".pdf-error")).toContainText("password-protected");
});

test("sets show how complete they are", async ({ page }) => {
  await page.goto("/");
  await nav(page, "Holdings");
  await page.getByRole("button", { name: "More" }).click();
  await page.getByRole("menuitem", { name: "Sets…" }).click();
  const dialog = page.getByRole("dialog", { name: "Sets" });
  await expect(dialog.getByText("Gold")).toBeVisible();
  await expect(dialog.getByText("1 of 20")).toBeVisible();
  await dialog.getByText("Gold").click();
  await expect(dialog.getByRole("button", { name: "Gold Eagles", exact: true })).toBeVisible();
});

test("the insurance report shows the evidence behind a value", async ({ page }) => {
  await page.goto("/");
  await nav(page, "Reports");
  await page.getByRole("button", { name: "Preview report" }).click();
  const comic = page.locator(".report-item", { hasText: "Amazing Fantasy #15" });
  await expect(comic.getByText("Basis for value")).toBeVisible();
  await expect(comic).toContainText("Medium confidence");
  await expect(comic).toContainText("CGC 9.8, Heritage");
  const [args] = await callsTo(page, "insurance_report");
  expect(args.options.include_evidence).toBe(true);
});
