import { test } from "node:test";
import assert from "node:assert/strict";

import { catalogHealth, CHECKS, topLocation } from "../src/lib/health.js";
import { daysAgoIso } from "../src/lib/format.js";

const asset = (over) => ({
  asset_id: over.name, name: "x", status: "active", current_amount_minor: "100000", current_currency: "USD",
  value_asof: daysAgoIso(10), acquired_amount_minor: null, cost_complete: true, insured_amount_minor: null,
  insured_currency: null, primary_photo: "p", document_count: 1, review_due: false, storage_location: null, ...over,
});

test("each check finds exactly the assets with that gap", () => {
  const assets = [
    asset({ name: "stale", value_asof: daysAgoIso(400) }),
    asset({ name: "under", insured_amount_minor: "50000", insured_currency: "USD" }),
    asset({ name: "covered", insured_amount_minor: "150000", insured_currency: "USD" }),
    asset({ name: "other-ccy", insured_amount_minor: "5", insured_currency: "EUR" }),
    asset({ name: "partial", acquired_amount_minor: "100", cost_complete: false }),
    asset({ name: "bare", primary_photo: null, document_count: 0 }),
    asset({ name: "sold", status: "sold", primary_photo: null }),
  ];
  const names = (check) => assets.filter((a) => a.status === "active").filter(CHECKS[check]).map((a) => a.name);
  assert.deepEqual(names("stale"), ["stale"]);
  assert.deepEqual(names("underinsured"), ["under"], "never compared across currencies");
  assert.deepEqual(names("partial_cost"), ["partial"]);
  assert.deepEqual(names("photo"), ["bare"], "sold items are not counted");
  assert.equal(catalogHealth(assets, "USD").counts.photo, 1);
});

test("concentration is measured by top-level place and by item", () => {
  const assets = [
    asset({ name: "a", storage_location: "Safe / Top shelf", current_amount_minor: "600" }),
    asset({ name: "b", storage_location: "Safe", current_amount_minor: "200" }),
    asset({ name: "c", storage_location: "Bank box", current_amount_minor: "200" }),
    asset({ name: "d", current_amount_minor: "999999", current_currency: "JPY" }),
  ];
  const h = catalogHealth(assets, "USD");
  assert.equal(topLocation(assets[0]), "Safe");
  assert.deepEqual(h.location, { name: "Safe", share: 0.8, places: 2 });
  assert.equal(h.largest.name, "a");
  assert.equal(h.largest.share, 0.6, "foreign-currency items are not in the shares");
});
