// Markets: spot metal prices and coin prices, with the feed status honest
// about quotas and age. Every price here can be typed by hand — a dead feed
// or a spent quota never blocks valuation.

import { call, describe } from "../lib/api.js";
import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { busy, toast, textInput, select, field, callout, emptyState } from "../ui/components.js";

const METAL_ICON_CLASS = { XAU: "gold", XAG: "silver", XPT: "platinum", XPD: "palladium" };

export async function renderMarkets(root, _params, ctx) {
  const [spots, metalsStatus, cryptoStatus, coins, settings, rates] = await Promise.all([
    call("spot_prices"),
    call("metals_provider_status"),
    call("crypto_provider_status"),
    call("crypto_prices"),
    store.settings(),
    call("list_rates"),
  ]);

  const afterPrices = (summary) => {
    store.invalidate();
    if (summary?.unpriced?.length) {
      toast(`${summary.updated} holding${summary.updated === 1 ? "" : "s"} revalued; ${summary.unpriced.length} still waiting for a price.`, { kind: "info" });
    } else if (summary?.updated) {
      toast(`${summary.updated} holding${summary.updated === 1 ? "" : "s"} revalued.`, { kind: "success" });
    }
    ctx.refresh();
  };

  const calculator = await calculatorSection();
  mount(
    root,
    h("header", { class: "page-head" },
      h("div", {}, h("h1", {}, "Markets"), h("p", { class: "page-sub" }, "The prices your market-tracked holdings follow"))
    ),
    metalsSection(spots, metalsStatus, settings, afterPrices, ctx),
    cryptoSection(coins, cryptoStatus, afterPrices, ctx),
    ratesSection(rates, settings, ctx),
    calculator
  );
}

/**
 * Exchange rates the owner records. Totals convert other currencies at the
 * latest rate on or before their date; nothing is fetched, because asking
 * a provider would say which currencies are held.
 */
function ratesSection(rates, settings, ctx) {
  const from = select(fmt.COMMON_CURRENCIES.filter((c) => c !== settings.currency).map((c) => [c, c]), "EUR", { "aria-label": "From currency" });
  const to = select(fmt.COMMON_CURRENCIES.map((c) => [c, c]), settings.currency, { "aria-label": "To currency" });
  const rate = h("input", { type: "text", inputmode: "decimal", placeholder: "e.g. 1.0842", "aria-label": "Rate", class: "input-sm" });
  const asof = h("input", { type: "date", value: fmt.todayIso(), max: fmt.todayIso(), "aria-label": "As of" });
  const add = h("button", { class: "btn btn-primary btn-sm", onclick: () => busy(add, async () => {
    await call("record_rate", { from: from.value, to: to.value, rate: rate.value, asof: asof.value });
    store.invalidate();
    toast("Rate recorded. Totals now include that currency.", { kind: "success" });
    ctx.refresh();
  }) }, "Add rate");
  // The latest of each pair, then older ones folded away.
  const seen = new Set();
  const latest = rates.filter((r) => {
    const key = [r.from_currency, r.to_currency].sort().join("/");
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  const row = (r) => h("tr", {},
    h("td", {}, `1 ${r.from_currency} = ${r.rate} ${r.to_currency}`),
    h("td", { class: "muted small" }, fmt.date(r.asof)),
    h("td", { class: "row-action" }, h("button", { class: "icon-btn", "aria-label": "Delete this rate", onclick: async () => {
      try { await call("delete_rate", { rateId: r.rate_id }); store.invalidate(); ctx.refresh(); } catch (e) { toast(describe(e), { kind: "error" }); }
    } }, icon("x", { size: 14 })))
  );
  return h("section", { class: "card", id: "rates" },
    h("div", { class: "card-head" }, h("div", {}, h("h2", {}, "Exchange rates"), h("p", { class: "card-sub" }, `Holdings valued in another currency count toward ${settings.currency} totals at the latest rate on or before each total's date. Their own values are never changed.`))),
    h("div", { class: "inline-form rate-form" }, h("span", {}, "1"), from, h("span", {}, "="), rate, to, h("span", {}, "as of"), asof, add),
    latest.length
      ? h("table", { class: "table compact" }, h("tbody", {}, latest.map(row)))
      : h("p", { class: "muted small" }, "No rates yet. Without one, holdings in other currencies are listed but left out of totals."),
    rates.length > latest.length
      ? h("details", { class: "disclosure" }, h("summary", {}, `Earlier rates (${rates.length - latest.length})`),
          h("table", { class: "table compact" }, h("tbody", {}, rates.filter((r) => !latest.includes(r)).map(row))))
      : null
  );
}

function freshnessBadge(freshness, ageHours) {
  if (!freshness) return h("span", { class: "badge badge-muted" }, "Not set");
  const warn = ["stale", "out of date"].includes(freshness);
  const days = Math.floor((ageHours ?? 0) / 24);
  const age = days >= 1 ? `${days}d old` : "today";
  return h("span", { class: warn ? "badge badge-attention" : "badge badge-fresh" }, warn ? icon("warn", { size: 12 }) : icon("clock", { size: 12 }), `${freshness} · ${age}`);
}

function metalsSection(spots, status, settings, afterPrices, ctx) {
  const q = status.quota;
  const cards = spots.map((s) => {
    const input = textInput({ inputmode: "decimal", placeholder: "Price per troy oz", "aria-label": `${s.metal_name} price per troy ounce` });
    const setButton = h("button", { class: "btn btn-secondary btn-sm", type: "submit" }, "Set");
    return h("div", { class: `spot-card spot-${METAL_ICON_CLASS[s.metal]}` },
      h("div", { class: "spot-head" }, h("span", { class: "spot-swatch", "aria-hidden": "true" }), h("strong", {}, s.metal_name), h("span", { class: "spot-code" }, s.metal)),
      h("div", { class: "spot-price" }, s.price_per_troy_oz ? fmt.unitPrice(s.price_per_troy_oz, s.currency) : h("span", { class: "muted" }, "—")),
      h("div", { class: "spot-unit" }, "per troy ounce"),
      h("div", { class: "spot-meta" }, freshnessBadge(s.freshness, s.age_hours), s.source ? h("span", { class: "muted small" }, s.source === "manual" ? "entered by hand" : s.source) : null),
      h("form", { class: "spot-form", onsubmit: async (e) => {
        e.preventDefault();
        if (!input.value.trim()) return;
        await busy(setButton, async () => {
          const summary = await call("set_spot_price", { metal: s.metal, price: input.value.trim(), currency: settings.currency });
          toast(`${s.metal_name} set to ${fmt.unitPrice(input.value.trim().replace(/[,$]/g, ""), settings.currency)}.`, { kind: "success" });
          afterPrices(summary);
        });
      } }, h("div", { class: "input-affix" }, h("span", { class: "affix" }, settings.currency), input), setButton)
    );
  });

  const refresh = h("button", { class: "btn btn-primary", onclick: () => busy(refresh, async () => {
    const r = await call("refresh_spot_prices", { automatic: false });
    toast(r.updated.length ? `Updated ${r.updated.join(", ")}. ${r.quota.remaining} of ${r.quota.monthly_limit} requests left this month.` : "The feed returned no usable prices.", { kind: r.updated.length ? "success" : "warning" });
    afterPrices(r.revalued);
  }, "Fetching…") }, icon("refresh", { size: 16 }), "Update from metals.dev");

  const used = q.used_this_month;
  const meter = h("div", { class: "quota" },
    h("div", { class: "quota-bar", role: "meter", "aria-valuemin": "0", "aria-valuemax": String(q.monthly_limit), "aria-valuenow": String(used), "aria-label": "Requests used this month" },
      h("span", { style: { width: `${Math.min(100, (used / q.monthly_limit) * 100)}%` }, class: q.remaining < 10 ? "low" : "" })
    ),
    h("span", { class: "muted small" }, `${used} of ${q.monthly_limit} free requests used this month · one request updates all four metals`)
  );

  return h("section", { class: "card" },
    h("div", { class: "card-head" },
      h("div", {}, h("h2", {}, "Precious metals"), h("p", { class: "card-sub" }, "Spot price per troy ounce. Staleness is judged by when the source priced it, not when it was fetched.")),
      h("div", { class: "card-tools" }, status.configured ? refresh : h("button", { class: "btn btn-secondary", onclick: () => ctx.navigate("settings", { section: "feeds" }) }, icon("key", { size: 16 }), "Add a price feed key"))
    ),
    h("div", { class: "spot-grid" }, cards),
    status.configured ? meter : h("p", { class: "muted small" }, "No price feed configured. Typing a price costs nothing and works offline; a free metals.dev key adds one-click updates.")
  );
}

function cryptoSection(coins, status, afterPrices, ctx) {
  const refresh = h("button", { class: "btn btn-primary", onclick: () => busy(refresh, async () => {
    const r = await call("refresh_crypto_prices", {});
    const parts = [];
    if (r.updated.length) parts.push(`Updated ${r.updated.length} coin${r.updated.length === 1 ? "" : "s"}.`);
    if (r.missing.length) parts.push(`No price for ${r.missing.join(", ")} — check the coin ID.`);
    toast(parts.join(" ") || "Nothing to update.", { kind: r.missing.length ? "warning" : "success" });
    afterPrices(r.revalued);
  }, "Fetching…") }, icon("refresh", { size: 16 }), "Update from CoinGecko");

  const body = coins.length
    ? h("table", { class: "table" },
        h("thead", {}, h("tr", {}, h("th", {}, "Coin"), h("th", { class: "num" }, "Held"), h("th", { class: "num" }, "Price"), h("th", {}, "Priced"), h("th", { class: "num" }, "Set by hand"))),
        h("tbody", {}, coins.map((c) => {
          const input = textInput({ inputmode: "decimal", placeholder: "Price", class: "input-money input-sm", "aria-label": `${c.name} price` });
          const set = h("button", { class: "btn btn-secondary btn-sm", type: "submit" }, "Set");
          const age = c.source_asof ? fmt.daysSince(c.source_asof) : null;
          return h("tr", {},
            h("td", {}, h("div", { class: "name-cell" }, h("span", { class: "name" }, c.name), h("span", { class: "sub" }, `${c.symbol} · ${c.coin_id}`))),
            h("td", { class: "num" }, fmt.quantity(c.held)),
            h("td", { class: "num" }, c.unit_price ? fmt.unitPrice(c.unit_price, c.currency) : h("span", { class: "muted" }, "—")),
            h("td", {}, c.source_asof ? h("span", { class: age >= 2 ? "badge badge-attention" : "badge badge-fresh" }, `${c.source === "manual" ? "by hand" : c.source} · ${fmt.ago(c.source_asof)}`) : h("span", { class: "badge badge-muted" }, "No price")),
            h("td", { class: "num" }, h("form", { class: "inline-form", onsubmit: async (e) => {
              e.preventDefault();
              if (!input.value.trim()) return;
              await busy(set, async () => afterPrices(await call("set_coin_price", { coinId: c.coin_id, price: input.value.trim() })));
            } }, input, set))
          );
        }))
      )
    : emptyState({ glyph: "crypto", title: "No crypto holdings", body: "Add a coin from Holdings to track its price here." });

  return h("section", { class: "card" },
    h("div", { class: "card-head" },
      h("div", {}, h("h2", {}, "Cryptocurrency"), h("p", { class: "card-sub" }, "Priced by coin ID, not ticker — several tokens can share a symbol. Only coins you hold are requested.")),
      h("div", { class: "card-tools" }, coins.length ? (status.configured ? refresh : h("button", { class: "btn btn-secondary", onclick: () => ctx.navigate("settings", { section: "feeds" }) }, icon("key", { size: 16 }), "Add a CoinGecko key")) : null)
    ),
    body,
    status.configured && coins.length ? h("p", { class: "attribution" }, status.attribution) : null
  );
}

async function calculatorSection() {
  const presets = await store.presets();
  const preset = select([["", "Custom…"], ...presets.map((p) => [p.id, p.label])], "");
  const metal = select([["XAU", "Gold"], ["XAG", "Silver"], ["XPT", "Platinum"], ["XPD", "Palladium"]], "XAU");
  const qty = textInput({ value: "1", inputmode: "decimal" });
  const weight = textInput({ value: "1", inputmode: "decimal" });
  const unit = select([["troy_oz", "troy oz"], ["gram", "grams"], ["pennyweight", "pennyweight"], ["ounce", "oz (avoirdupois)"]], "troy_oz");
  const basis = select([["gross", "Gross"], ["fine", "Fine"]], "gross");
  const purity = textInput({ value: "0.999", inputmode: "decimal" });
  const premium = textInput({ inputmode: "decimal", placeholder: "0" });
  const result = h("div", { class: "calc-result", "aria-live": "polite" });

  preset.addEventListener("change", () => {
    const p = presets.find((x) => x.id === preset.value);
    if (!p) return;
    metal.value = p.metal;
    weight.value = p.weight;
    unit.value = p.unit;
    basis.value = p.basis;
    purity.value = p.purity;
  });

  const calc = h("button", { class: "btn btn-primary", type: "submit" }, "Calculate");
  const form = h("form", { class: "calc", onsubmit: async (e) => {
    e.preventDefault();
    await busy(calc, async () => {
      try {
        const r = await call("value_metal_holding", { request: { metal: metal.value, quantity: qty.value || "1", weight_per_item: weight.value, unit: unit.value, basis: basis.value, purity: purity.value, premium_pct: premium.value || null } });
        mount(result,
          h("div", { class: "mini-stat" }, h("span", {}, "Fine metal"), h("strong", {}, `${fmt.quantity(r.fine_troy_oz)} troy oz`)),
          h("div", { class: "mini-stat" }, h("span", {}, "Melt value"), h("strong", {}, fmt.money(r.melt))),
          h("div", { class: "mini-stat" }, h("span", {}, "Premium"), h("strong", {}, fmt.money(r.premium_amount))),
          h("div", { class: "mini-stat" }, h("span", {}, "Market value"), h("strong", {}, fmt.money(r.market))),
          r.needs_caveat ? callout("warning", `Based on a ${r.freshness} spot price from ${fmt.date(r.source_asof)}.`) : null
        );
      } catch (err) {
        mount(result, h("p", { class: "form-error" }, describe(err)));
      }
    });
  } },
    h("div", { class: "form-grid cols-4" },
      field("Product", preset, { span: 2 }),
      field("Metal", metal),
      field("Quantity", qty),
      field("Weight each", h("div", { class: "input-pair" }, weight, unit), { span: 2 }),
      field("Weight is", basis),
      field("Purity", purity),
      field("Premium %", premium)
    ),
    h("div", { class: "btn-row" }, calc)
  );
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("div", {}, h("h2", {}, "Melt calculator"), h("p", { class: "card-sub" }, "Value metal you have not catalogued yet, at the stored spot price. Melt and premium are shown apart — a coin's premium can dwarf its metal."))),
    form,
    result
  );
}
