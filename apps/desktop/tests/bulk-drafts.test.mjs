import { test } from "node:test";
import assert from "node:assert/strict";
import { pendingValues, settleValues } from "../src/bulk-drafts.js";

test("saving includes all drafts and preserves the date and per-asset currencies", () => {
  const drafts = new Map([
    ["hidden", { value: "12", original: "10", currency: "USD" }],
    ["paged-out", { value: "24", original: "20", currency: "EUR" }],
    ["unchanged", { value: "30", original: "30", currency: "USD" }],
    ["blank", { value: "", original: "40", currency: "USD" }],
  ]);
  assert.deepEqual(pendingValues(drafts, "2026-01-15"), [
    { asset_id: "hidden", amount: "12", currency: "USD", asof: "2026-01-15" },
    { asset_id: "paged-out", amount: "24", currency: "EUR", asof: "2026-01-15" },
  ]);
});

test("partial success retains failed and concurrently edited drafts", () => {
  const state = { saved: new Map(), drafts: new Map(["a", "b", "c"].map((id) => [id, { value: "20", original: "10", currency: "USD" }])) };
  const entries = pendingValues(state.drafts, "2026-01-15");
  state.drafts.get("c").value = "30";
  settleValues(state, entries, [{ asset_id: "a", ok: true }, { asset_id: "b", ok: false, error: "Invalid" }, { asset_id: "c", ok: true }]);
  assert.equal(state.drafts.has("a"), false);
  assert.equal(state.drafts.get("b").error, "Invalid");
  assert.equal(state.drafts.get("c").value, "30");
  assert.equal(state.drafts.get("c").original, "20");
  assert.deepEqual(pendingValues(state.drafts, "2026-01-15").map((e) => e.asset_id), ["b", "c"]);
});
