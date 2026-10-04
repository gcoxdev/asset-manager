// Exact money arithmetic for the frontend.
//
// Amounts arrive as minor-unit strings because JavaScript numbers are 53-bit
// floats. Anything here that compares or adds them does so with BigInt, and
// never across currencies.

import * as fmt from "./format.js";

/** Compare decimal/minor-unit strings exactly, without float conversion. */
export function cmpBig(a, b) {
  const x = a == null ? null : BigInt(a);
  const y = b == null ? null : BigInt(b);
  if (x === y) return 0;
  if (x === null) return 1; // unknowns last, whatever the direction
  if (y === null) return -1;
  return x > y ? -1 : 1;
}

/**
 * Compare amounts that may be in different currencies. Minor units of two
 * currencies are not comparable — 100,000 JPY is not more than 1,000 USD —
 * and there is no exchange rate to make them so. So the base currency comes
 * first, then each other currency as its own group, largest first within it.
 */
export function cmpMoney(a, aCurrency, b, bCurrency, base) {
  if (a == null || b == null || aCurrency === bCurrency) return cmpBig(a, b);
  const rank = (c) => (c === base ? 0 : 1);
  return rank(aCurrency) - rank(bCurrency) || String(aCurrency).localeCompare(String(bCurrency));
}

/** Sum minor units exactly and lay the result out like the backend does. */
export function sumDisplay(assets, currency) {
  let digits = null;
  let total = 0n;
  let count = 0;
  for (const a of assets) {
    if (a.current_amount_minor == null || a.current_currency !== currency) continue;
    digits ??= fmt.digitsOf(a.current_display);
    total += BigInt(a.current_amount_minor);
    count += 1;
  }
  if (!count) return null;
  const negative = total < 0n;
  let s = (negative ? -total : total).toString().padStart((digits ?? 0) + 1, "0");
  if (digits) s = `${s.slice(0, -digits)}.${s.slice(-digits)}`;
  return `${negative ? "-" : ""}${s} ${currency}`;
}

