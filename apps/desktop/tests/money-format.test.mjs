// Pure frontend logic, run with Node's built-in test runner: `npm test`.
// These are the places a float or a time zone would silently corrupt a
// figure, so they are pinned here rather than trusted to the view code.

import { test } from "node:test";
import assert from "node:assert/strict";

import * as fmt from "../src/lib/format.js";
import { cmpMoney, cmpBig, sumDisplay } from "../src/lib/money.js";

test("money is laid out from its string, never through a float", () => {
  assert.equal(fmt.money("90071992547409.93 USD"), "$90,071,992,547,409.93");
  assert.equal(fmt.money("-50.00 EUR"), "−€50.00");
  assert.equal(fmt.money("1000 JPY"), "¥1,000");
});

test("dollars other than the US dollar are never shown as a bare $", () => {
  assert.equal(fmt.money("10.00 CAD"), "CA$10.00");
  assert.equal(fmt.money("10.00 AUD"), "A$10.00");
});

test("today is the local calendar date, not the UTC one", () => {
  const now = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  assert.equal(fmt.todayIso(), `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`);
  assert.equal(fmt.daysAgoIso(0), fmt.todayIso());
});

test("large minor-unit amounts compare exactly", () => {
  // Beyond 2^53 these two differ by one but are equal as floats.
  assert.equal(cmpBig("9007199254740993", "9007199254740992"), -1);
  assert.equal(cmpBig(null, "1"), 1, "unknowns sort last");
});

test("amounts in different currencies are grouped, never ranked against each other", () => {
  const items = [
    { m: "100000", c: "JPY" },
    { m: "500", c: "USD" },
    { m: "90000", c: "USD" },
    { m: "2000", c: "CAD" },
  ];
  const sorted = [...items].sort((a, b) => cmpMoney(a.m, a.c, b.m, b.c, "USD")).map((i) => `${i.m} ${i.c}`);
  assert.deepEqual(sorted, ["90000 USD", "500 USD", "2000 CAD", "100000 JPY"]);
});

test("a total sums one currency exactly and leaves the others out", () => {
  const assets = [
    { current_amount_minor: "9007199254740993", current_currency: "USD", current_display: "90071992547409.93 USD" },
    { current_amount_minor: "7", current_currency: "USD", current_display: "0.07 USD" },
    { current_amount_minor: "500", current_currency: "EUR", current_display: "5.00 EUR" },
  ];
  assert.equal(sumDisplay(assets, "USD"), "90071992547410.00 USD");
  assert.equal(sumDisplay([], "USD"), null);
});

test("a converted value joins the base-currency total; an unconverted one does not", async () => {
  const { sumDisplay, valueInBase } = await import("../src/lib/money.js");
  const assets = [
    { current_amount_minor: "10000", current_currency: "USD", current_display: "100.00 USD" },
    { current_amount_minor: "10000", current_currency: "EUR", current_display: "100.00 EUR", value_in_base_minor: "11000", value_in_base_display: "110.00 USD" },
    { current_amount_minor: "500", current_currency: "GBP", current_display: "5.00 GBP" },
  ];
  assert.equal(sumDisplay(assets, "USD"), "210.00 USD");
  assert.equal(valueInBase(assets[2], "USD"), null);
});
