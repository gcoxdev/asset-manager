// Catalog health: what is missing from the figures, and what to do next.
//
// Pure functions over the asset list, shared by the overview card and the
// holdings filters, so "3 items are under-insured" and the list that link
// opens can never disagree. Money is compared with BigInt and only within
// one currency; shares of value are approximate by design (they size a bar,
// they are not a figure anyone relies on).

import { daysSince } from "./format.js";

/** A value older than this is stale, whatever the reminder setting. */
export const STALE_DAYS = 365;

const held = (a) => a.status === "active";

/** Each check: which assets fail it. Keys match the holdings "missing" filter. */
export const CHECKS = {
  value: (a) => a.current_amount_minor == null,
  stale: (a) => a.current_amount_minor != null && a.value_asof != null && daysSince(a.value_asof) > STALE_DAYS,
  partial_cost: (a) => a.acquired_amount_minor != null && a.cost_complete === false,
  insurance: (a) => a.insured_amount_minor == null,
  underinsured: (a) =>
    a.insured_amount_minor != null &&
    a.current_amount_minor != null &&
    a.insured_currency === a.current_currency &&
    BigInt(a.insured_amount_minor) < BigInt(a.current_amount_minor),
  photo: (a) => !a.primary_photo,
  document: (a) => !a.document_count,
  review: (a) => Boolean(a.review_due),
  away: (a) => Boolean(a.away),
};

/** Where something is kept, at the top level: "Safe / Top shelf" → "Safe". */
export function topLocation(a) {
  return (a.storage_location ?? "").split(" / ")[0].trim() || null;
}

/**
 * Health of the held catalog in `currency`.
 * Returns counts per check and the largest concentrations of value.
 */
export function catalogHealth(assets, currency) {
  const active = assets.filter(held);
  const counts = Object.fromEntries(Object.entries(CHECKS).map(([k, test]) => [k, active.filter(test).length]));

  // Concentration, over holdings valued in the base currency.
  const valued = active.filter((a) => a.current_amount_minor != null && a.current_currency === currency);
  const total = valued.reduce((s, a) => s + BigInt(a.current_amount_minor), 0n);
  const share = (part) => (total > 0n ? Number((part * 10000n) / total) / 10000 : 0);
  const byLocation = new Map();
  for (const a of valued) {
    const where = topLocation(a) ?? "(no location)";
    byLocation.set(where, (byLocation.get(where) ?? 0n) + BigInt(a.current_amount_minor));
  }
  const places = [...byLocation].sort((x, y) => (y[1] > x[1] ? 1 : y[1] < x[1] ? -1 : 0));
  const largest = valued.reduce((best, a) => (!best || BigInt(a.current_amount_minor) > BigInt(best.current_amount_minor) ? a : best), null);

  return {
    held: active.length,
    counts,
    location: places.length ? { name: places[0][0], share: share(places[0][1]), places: places.length } : null,
    largest: largest ? { asset_id: largest.asset_id, name: largest.name, share: share(BigInt(largest.current_amount_minor)) } : null,
  };
}
