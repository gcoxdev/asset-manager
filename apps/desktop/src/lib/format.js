// Display formatting.
//
// Money arrives from the backend already rounded and laid out ("1234.50 USD"),
// because the number of minor digits is per-currency and only the backend
// knows which rule it applied. This module adds grouping and a symbol by
// string manipulation — the amount is never parsed into a JS number, which
// is a 53-bit float and would corrupt large values.

/** Currencies offered wherever one can be picked. Any valid code is accepted. */
export const COMMON_CURRENCIES = ["USD", "EUR", "GBP", "CAD", "AUD", "CHF", "JPY", "NZD", "SEK", "NOK", "DKK", "SGD"];

const symbolCache = new Map();

function currencySymbol(code) {
  if (symbolCache.has(code)) return symbolCache.get(code);
  let symbol = code + " ";
  try {
    const part = new Intl.NumberFormat("en-US", {
      style: "currency",
      currency: code,
      // "symbol", not "narrowSymbol": the narrow form shows US, Canadian and
      // Australian dollars all as "$". In en-US only USD keeps the bare sign;
      // the others read "CA$", "A$", so mixed holdings cannot be mistaken
      // for one another.
      currencyDisplay: "symbol",
    })
      .formatToParts(1)
      .find((p) => p.type === "currency");
    // A bare code as the "symbol" reads better with a space after it.
    if (part) symbol = /^[A-Z]{3}$/.test(part.value) ? part.value + " " : part.value;
  } catch {
    // Unknown to Intl: keep the code.
  }
  symbolCache.set(code, symbol);
  return symbol;
}

function groupDigits(intPart) {
  return intPart.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

/** "1234.50 USD" → "$1,234.50"; "-50.00 EUR" → "−€50.00". */
export function money(display) {
  if (!display) return "—";
  const match = /^(-?)(\d+)(?:\.(\d+))? ([A-Z]{3})$/.exec(display.trim());
  if (!match) return display;
  const [, sign, int, frac, code] = match;
  const body = groupDigits(int) + (frac ? "." + frac : "");
  return (sign ? "−" : "") + currencySymbol(code) + body;
}

/** Signed money for gains: "+$120.00" / "−$40.00". */
export function signedMoney(display) {
  if (!display) return "—";
  const text = money(display);
  return text.startsWith("−") ? text : "+" + text;
}

/** An {minor, currency, display} amount from the backend. */
export function amount(a) {
  return a ? money(a.display) : "—";
}

/** Decimal places used in a display string, e.g. 2 for "10.00 USD". */
export function digitsOf(display) {
  const m = /\.(\d+) /.exec(display ?? "");
  return m ? m[1].length : 0;
}

/**
 * Minor units → a JS number of major units, for PIXELS and PERCENTAGES only.
 * Never for arithmetic whose result is shown as money.
 */
export function approxMajor(minor, digits) {
  return Number(minor) / 10 ** digits;
}

/** Compact money for axis labels: "$12.3K". Approximate by design. */
export function compactMoney(value, currency) {
  try {
    return new Intl.NumberFormat("en-US", {
      style: "currency",
      currency,
      currencyDisplay: "symbol",
      notation: "compact",
      maximumFractionDigits: 1,
    }).format(value);
  } catch {
    return String(Math.round(value));
  }
}

/** A decimal string for display: grouping, trailing zeros trimmed. */
export function quantity(text) {
  if (text === null || text === undefined || text === "") return "—";
  const match = /^(-?)(\d+)(?:\.(\d+))?$/.exec(String(text).trim());
  if (!match) return String(text);
  const [, sign, int, frac] = match;
  const trimmed = (frac ?? "").replace(/0+$/, "");
  return (sign ? "−" : "") + groupDigits(int) + (trimmed ? "." + trimmed : "");
}

export function percent(fraction, digits = 1) {
  if (!Number.isFinite(fraction)) return "—";
  return (fraction * 100).toFixed(digits).replace(/\.0+$/, "") + "%";
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "2026-09-22" or an RFC 3339 timestamp → "Sep 22, 2026". */
export function date(iso) {
  if (!iso) return "—";
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso);
  if (!m) return iso;
  return `${MONTHS[Number(m[2]) - 1]} ${Number(m[3])}, ${m[1]}`;
}

export function shortDate(iso) {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso ?? "");
  if (!m) return iso ?? "";
  return `${MONTHS[Number(m[2]) - 1]} ${Number(m[3])}`;
}

function dayNumber(iso) {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso ?? "");
  if (!m) return null;
  return Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])) / 86_400_000;
}

/**
 * Today's date where the user is, as YYYY-MM-DD. Effective dates — when
 * something was bought, sold or valued — are calendar dates in the owner's
 * time zone; toISOString() would give the UTC date, which is tomorrow for
 * everyone west of Greenwich each evening.
 */
export function todayIso() {
  const now = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** The local calendar date `days` before today, as YYYY-MM-DD. */
export function daysAgoIso(days) {
  const then = new Date();
  then.setDate(then.getDate() - days);
  const pad = (n) => String(n).padStart(2, "0");
  return `${then.getFullYear()}-${pad(then.getMonth() + 1)}-${pad(then.getDate())}`;
}

/** Whole days from `iso` to today. */
export function daysSince(iso) {
  const then = dayNumber(iso);
  const now = dayNumber(todayIso());
  return then === null || now === null ? null : Math.round(now - then);
}

/** "today", "yesterday", "3 days ago", "5 months ago". */
export function ago(iso) {
  if (!iso) return "never";
  // Timestamps carry hours; use them for the same-day case.
  if (iso.length > 10) {
    const ms = Date.now() - Date.parse(iso);
    if (Number.isFinite(ms) && ms < 86_400_000) {
      const hours = Math.floor(ms / 3_600_000);
      if (hours < 1) return "just now";
      return `${hours} hour${hours === 1 ? "" : "s"} ago`;
    }
  }
  const days = daysSince(iso);
  if (days === null) return iso;
  if (days <= 0) return "today";
  if (days === 1) return "yesterday";
  if (days < 45) return `${days} days ago`;
  const months = Math.round(days / 30.4);
  if (months < 18) return `${months} month${months === 1 ? "" : "s"} ago`;
  return `${Math.round(days / 365)} years ago`;
}

export function bytes(text) {
  const n = Number(text);
  if (!Number.isFinite(n)) return "—";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = n / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i += 1;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[i]}`;
}

export const CATEGORY_LABELS = {
  metals: "Precious metals",
  crypto: "Crypto",
  collectibles: "Collectibles",
  valuables: "Valuables",
  investments: "Investments",
  household: "Home & household",
  vehicles: "Vehicles",
  property: "Property",
  firearms: "Firearms",
  cash: "Cash & accounts",
  other: "Other",
};

export function categoryLabel(id) {
  return CATEGORY_LABELS[id] ?? id;
}

export const BASIS_LABELS = {
  estimated_resale: "Estimated resale",
  replacement: "Replacement",
  insured: "Insured value",
  melt: "Melt value",
};

export const PROVENANCE_LABELS = {
  manual: "Entered by hand",
  api: "Market feed",
  appraisal: "Appraisal",
};

export const EVENT_LABELS = {
  acquire: "Acquired",
  add: "Bought more",
  remove: "Sold some",
  dispose: "Sold",
  correct: "Corrected count",
  split: "Split off",
};

export const DOC_KINDS = ["receipt", "appraisal", "certificate", "warranty", "manual", "other"];

export const DOC_KIND_LABELS = {
  photo: "Photo",
  receipt: "Receipt",
  appraisal: "Appraisal",
  certificate: "Certificate",
  warranty: "Warranty",
  manual: "Manual",
  other: "Document",
};

export const CARE_KINDS = ["service", "repair", "inspection", "cleaning", "appraisal", "battery", "warranty", "other"];

export const CARE_LABELS = {
  service: "Service",
  repair: "Repair",
  inspection: "Inspection",
  cleaning: "Cleaning",
  appraisal: "Appraisal",
  battery: "Battery",
  warranty: "Warranty",
  other: "Other care",
};

export const CUSTODY_LABELS = {
  lent: "Lent",
  consigned: "Consigned",
  repair: "At repair",
  storage: "In outside storage",
  shipped: "Shipped",
  returned: "Back home",
};

export const STATUS_LABELS = { active: "Held", sold: "Sold", lost: "Lost", retired: "Retired" };

/** "player_or_character" → "Player or character". */
export function fieldLabel(key) {
  const special = {
    cert_number: "Cert number",
    serial_number: "Serial number",
    card_number: "Card number",
    set_code: "Set code",
    key_issue: "Key issue",
    print_run: "Print run",
    player_or_character: "Player or character",
    box_papers: "Box & papers",
    vin: "VIN",
    hull_id: "Hull ID",
    isbn: "ISBN",
    isin: "ISIN",
    ownership_share: "Your share (%)",
    square_feet: "Square feet",
    year_built: "Year built",
    catalogue_number: "Catalogue number",
    set_number: "Set number",
    held_at: "Held at",
  };
  if (special[key]) return special[key];
  const words = key.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/**
 * A unit price from a quote ("2014.3", "0.00001234") with at least two
 * decimals, keeping every digit the source gave — sub-cent prices are real.
 */
export function unitPrice(decimal, currency) {
  if (!decimal) return "—";
  const [int, frac = ""] = String(decimal).split(".");
  return money(`${int}.${frac.padEnd(2, "0")} ${currency ?? "USD"}`);
}
