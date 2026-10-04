// Holdings: the catalog as a searchable, sortable list — plus bulk value
// entry, because setting prices one dialog at a time does not scale to a few
// hundred collectibles.

import { call, describe } from "../lib/api.js";
import { mediaUrl } from "../lib/media.js";
import { h, mount, debounce } from "../lib/dom.js";
import { icon, typeIcon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { sourceBadge, statusBadge, emptyState, select, segmented, busy, toast, menuButton } from "../ui/components.js";
import { openAddAsset } from "./asset-forms.js";
import { exportCsv, importCsv } from "./reports.js";

const PAGE = 300;

const SORTS = [
  ["updated", "Recently changed"],
  ["name", "Name"],
  ["value", "Value, high to low"],
  ["gain", "Gain, high to low"],
  ["acquired", "Date acquired"],
];

/** Compare decimal/minor-unit strings exactly, without float conversion. */
function cmpBig(a, b) {
  const x = a == null ? null : BigInt(a);
  const y = b == null ? null : BigInt(b);
  if (x === y) return 0;
  if (x === null) return 1; // unknowns last, whatever the direction
  if (y === null) return -1;
  return x > y ? -1 : 1;
}

/** Sum minor units exactly and lay the result out like the backend does. */
function sumDisplay(assets, currency) {
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

export async function renderHoldings(root, params, ctx) {
  const prefs = store.ui();
  if (params.category) prefs.category = params.category;
  if (params.sort) prefs.sort = params.sort;
  let special = params.filter ?? null; // "unvalued" | "review"
  if (special) prefs.status = "active";

  const [assets, settings] = await Promise.all([store.assets(), store.settings()]);
  let searchIds = null;
  let bulk = false;
  let shown = PAGE;

  const search = h("input", { type: "search", class: "search-input", placeholder: "Search names, notes, cert numbers…", value: prefs.query, "data-search": "", "aria-label": "Search holdings" });
  const results = h("div", { class: "results" });
  const summary = h("div", { class: "results-summary" });
  const chips = h("div", { class: "chips", role: "group", "aria-label": "Category" });

  const statusSelect = select([["active", "Held"], ["sold", "Sold"], ["inactive", "Lost & retired"], ["all", "Everything"]], prefs.status, { "aria-label": "Status" });
  statusSelect.addEventListener("change", () => { prefs.status = statusSelect.value; shown = PAGE; draw(); });
  const sortSelect = select(SORTS, prefs.sort, { "aria-label": "Sort" });
  sortSelect.addEventListener("change", () => { prefs.sort = sortSelect.value; draw(); });
  const layout = segmented([["list", "List"], ["grid", "Grid"]], prefs.layout, (v) => { prefs.layout = v; draw(); });

  const runSearch = debounce(async () => {
    prefs.query = search.value;
    const q = search.value.trim();
    searchIds = q ? await call("search_assets", { query: q }) : null;
    shown = PAGE;
    draw();
  }, 160);
  search.addEventListener("input", runSearch);
  search.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && search.value) { search.value = ""; runSearch(); }
  });

  const addButton = h("button", { class: "btn btn-primary", onclick: () => openAddAsset({ onSaved: (id) => ctx.navigate("asset", { id }) }) }, icon("plus", { size: 16 }), "Add asset");
  const more = menuButton(h("button", { class: "btn btn-secondary", "aria-label": "More actions" }, icon("more")), [
    { label: "Update many values…", icon: "edit", onSelect: () => { bulk = true; prefs.layout = "list"; draw(); } },
    "divider",
    { label: "Import CSV…", icon: "upload", onSelect: () => importCsv(() => ctx.refresh()) },
    { label: "Export CSV…", icon: "download", onSelect: () => exportCsv() },
  ]);

  function filtered() {
    let list = assets;
    if (prefs.status === "active") list = list.filter((a) => a.status === "active");
    else if (prefs.status === "sold") list = list.filter((a) => a.status === "sold");
    else if (prefs.status === "inactive") list = list.filter((a) => a.status === "lost" || a.status === "retired");
    if (prefs.category !== "all") list = list.filter((a) => a.category === prefs.category);
    if (special === "unvalued") list = list.filter((a) => a.current_amount_minor == null);
    if (special === "review") list = list.filter((a) => a.review_due);

    if (searchIds) {
      const rank = new Map(searchIds.map((id, i) => [id, i]));
      list = list.filter((a) => rank.has(a.asset_id)).sort((a, b) => rank.get(a.asset_id) - rank.get(b.asset_id));
      return list; // relevance order while searching
    }
    const sorted = [...list];
    switch (prefs.sort) {
      case "name": sorted.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" })); break;
      case "value": sorted.sort((a, b) => cmpBig(a.current_amount_minor, b.current_amount_minor)); break;
      case "gain": sorted.sort((a, b) => cmpBig(a.gain_minor, b.gain_minor)); break;
      case "acquired": sorted.sort((a, b) => (b.acquired_date ?? "").localeCompare(a.acquired_date ?? "")); break;
      default: break; // backend order: most recently changed first
    }
    return sorted;
  }

  function drawChips() {
    const counts = new Map();
    for (const a of assets) if (a.status === "active") counts.set(a.category, (counts.get(a.category) ?? 0) + 1);
    const order = ["metals", "crypto", "collectibles", "valuables", "cash", "other"];
    const cats = order.filter((c) => counts.has(c) || prefs.category === c);
    const chip = (id, label, count) =>
      h("button", {
        class: prefs.category === id ? "chip active" : "chip",
        "aria-pressed": prefs.category === id ? "true" : "false",
        onclick: () => { prefs.category = id; shown = PAGE; draw(); },
      }, label, count != null ? h("span", { class: "chip-count" }, String(count)) : null);
    mount(chips, chip("all", "All", null), cats.map((c) => chip(c, fmt.categoryLabel(c), counts.get(c) ?? 0)));
  }

  function draw() {
    drawChips();
    const list = filtered();
    const total = sumDisplay(list, settings.currency);
    const unpriced = list.filter((a) => a.current_amount_minor == null).length;
    mount(summary,
      h("span", {}, `${list.length} ${list.length === 1 ? "holding" : "holdings"}`),
      total ? h("span", {}, " · ", h("strong", {}, fmt.money(total))) : null,
      unpriced ? h("span", { class: "muted" }, ` · ${unpriced} without a value`) : null,
      special
        ? h("button", { class: "chip active chip-dismiss", onclick: () => { special = null; draw(); } },
            special === "unvalued" ? "No value yet" : "Due for review", icon("x", { size: 14 }))
        : null
    );

    if (!assets.length) {
      mount(results, emptyState({ title: "Nothing here yet", body: "Add your first asset, or import a spreadsheet.", actions: [addButton.cloneNode(true)] }));
      results.querySelector("button")?.addEventListener("click", () => openAddAsset({ onSaved: (id) => ctx.navigate("asset", { id }) }));
      return;
    }
    if (!list.length) {
      mount(results, emptyState({ glyph: "search", title: "No matches", body: searchIds ? "Nothing matches that search with these filters." : "Nothing matches these filters." }));
      return;
    }

    const page = list.slice(0, shown);
    const moreButton = list.length > shown
      ? h("button", { class: "btn btn-secondary load-more", onclick: () => { shown = list.length; draw(); } }, `Show all ${list.length}`)
      : null;

    if (bulk) mount(results, bulkTable(page, settings, () => { bulk = false; ctx.refresh(); }, () => { bulk = false; draw(); }), moreButton);
    else if (prefs.layout === "grid") mount(results, grid(page, ctx), moreButton);
    else mount(results, table(page, ctx), moreButton);
  }

  mount(
    root,
    h("header", { class: "page-head" },
      h("div", {}, h("h1", {}, "Holdings"), h("p", { class: "page-sub" }, "Everything in your catalog")),
      h("div", { class: "page-actions" }, more, addButton)
    ),
    h("div", { class: "toolbar" },
      h("div", { class: "search" }, icon("search", { size: 17 }), search),
      h("div", { class: "toolbar-right" }, statusSelect, sortSelect, layout.el)
    ),
    chips,
    summary,
    results
  );

  if (prefs.query) await runSearch();
  else draw();
  if (params.focusSearch) search.focus();
}

function thumb(a, size = 44) {
  return a.primary_photo
    ? h("img", { class: "thumb", src: mediaUrl(a.primary_photo, 256), alt: "", width: size, height: size, loading: "lazy" })
    : h("span", { class: "thumb thumb-glyph", style: { width: `${size}px`, height: `${size}px` } }, icon(typeIcon(a.type_id, a.category), { size: 20 }));
}

function subtitle(a) {
  return [a.type_label, a.storage_location].filter(Boolean).join(" · ");
}

function gainCell(a) {
  if (!a.gain_display) return h("span", { class: "muted" }, "—");
  const up = !a.gain_display.startsWith("-");
  return h("span", { class: `gain ${up ? "up" : "down"}` }, icon(up ? "up" : "downRight", { size: 14 }), fmt.signedMoney(a.gain_display));
}

function valueCell(a) {
  return h("div", { class: "value-cell" },
    h("span", { class: "value-amount" }, a.current_display ? fmt.money(a.current_display) : h("span", { class: "muted" }, "No value")),
    h("span", { class: "value-meta" },
      a.current_display ? sourceBadge(a.value_source, a.value_asof) : null,
      a.value_asof ? h("span", { class: "muted" }, fmt.ago(a.value_asof)) : null,
      a.review_due ? h("span", { class: "badge badge-attention" }, icon("clock", { size: 12 }), "Review") : null
    )
  );
}

function table(list, ctx) {
  const open = (a) => ctx.navigate("asset", { id: a.asset_id });
  return h("table", { class: "table holdings-table" },
    h("thead", {}, h("tr", {},
      h("th", { class: "col-photo" }, h("span", { class: "sr-only" }, "Photo")),
      h("th", {}, "Name"),
      h("th", { class: "num" }, "Quantity"),
      h("th", { class: "num" }, "Paid"),
      h("th", { class: "num" }, "Value"),
      h("th", { class: "num" }, "Gain")
    )),
    h("tbody", {}, list.map((a) =>
      h("tr", {
        class: "row-link",
        tabindex: "0",
        onclick: () => open(a),
        onkeydown: (e) => {
          if (e.key === "Enter") open(a);
          if (e.key === "ArrowDown") { e.preventDefault(); e.currentTarget.nextElementSibling?.focus(); }
          if (e.key === "ArrowUp") { e.preventDefault(); e.currentTarget.previousElementSibling?.focus(); }
        },
      },
        h("td", { class: "col-photo" }, thumb(a)),
        h("td", {}, h("div", { class: "name-cell" }, h("span", { class: "name" }, a.name, statusBadge(a.status)), h("span", { class: "sub" }, subtitle(a)))),
        h("td", { class: "num" }, fmt.quantity(a.quantity), h("span", { class: "unit" }, ` ${a.quantity_unit}`)),
        h("td", { class: "num" }, a.acquired_display ? fmt.money(a.acquired_display) : h("span", { class: "muted" }, "—")),
        h("td", { class: "num" }, valueCell(a)),
        h("td", { class: "num" }, gainCell(a))
      )
    ))
  );
}

function grid(list, ctx) {
  return h("div", { class: "card-grid" }, list.map((a) =>
    h("button", { class: "asset-card", onclick: () => ctx.navigate("asset", { id: a.asset_id }) },
      h("div", { class: "asset-card-media" },
        a.primary_photo
          ? h("img", { src: mediaUrl(a.primary_photo, 256), alt: "", loading: "lazy" })
          : h("span", { class: "asset-card-glyph" }, icon(typeIcon(a.type_id, a.category), { size: 34 })),
        statusBadge(a.status)
      ),
      h("div", { class: "asset-card-body" },
        h("strong", { class: "asset-card-name" }, a.name),
        h("span", { class: "sub" }, a.type_label),
        h("div", { class: "asset-card-foot" },
          h("span", { class: "value-amount" }, a.current_display ? fmt.money(a.current_display) : h("span", { class: "muted" }, "No value")),
          a.current_display ? sourceBadge(a.value_source) : null
        )
      )
    )
  ));
}

/**
 * Bulk value entry. Each row is saved independently and reports back, so
 * one bad cell does not discard the rest.
 */
function bulkTable(list, settings, onDone, onCancel) {
  const asof = h("input", { type: "date", value: fmt.todayIso(), max: fmt.todayIso(), "aria-label": "Values as of" });
  const inputs = new Map();
  const errors = new Map();
  const dirty = h("span", { class: "muted" }, "No changes yet");

  const countChanges = () => {
    const n = [...inputs.values()].filter((i) => i.value.trim() && i.value.trim() !== i.dataset.original).length;
    dirty.textContent = n ? `${n} change${n === 1 ? "" : "s"}` : "No changes yet";
    save.disabled = n === 0;
  };

  const rows = list.map((a, index) => {
    const original = a.current_display ? a.current_display.split(" ")[0] : "";
    const input = h("input", { type: "text", inputmode: "decimal", class: "input-money bulk-input", value: original, placeholder: "—", "aria-label": `Value of ${a.name}` });
    input.dataset.original = original;
    input.addEventListener("input", countChanges);
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === "ArrowDown") { e.preventDefault(); [...inputs.values()][index + 1]?.focus(); }
      if (e.key === "ArrowUp") { e.preventDefault(); [...inputs.values()][index - 1]?.focus(); }
    });
    inputs.set(a.asset_id, input);
    const err = h("div", { class: "field-error" });
    errors.set(a.asset_id, err);
    return h("tr", {},
      h("td", { class: "col-photo" }, thumb(a, 36)),
      h("td", {}, h("div", { class: "name-cell" }, h("span", { class: "name" }, a.name), h("span", { class: "sub" }, a.pricing === "market" ? `${subtitle(a)} · follows the market — a typed value switches it to manual` : subtitle(a)))),
      h("td", { class: "num" }, a.current_display ? fmt.money(a.current_display) : h("span", { class: "muted" }, "—")),
      h("td", { class: "num" }, h("div", { class: "input-affix" }, h("span", { class: "affix" }, a.current_currency ?? settings.currency), input), err)
    );
  });

  const save = h("button", { class: "btn btn-primary", disabled: true, onclick: async () => {
    const entries = [];
    for (const a of list) {
      const input = inputs.get(a.asset_id);
      const v = input.value.trim();
      if (v && v !== input.dataset.original) {
        entries.push({ asset_id: a.asset_id, amount: v, currency: a.current_currency ?? settings.currency, asof: asof.value });
      }
    }
    await busy(save, async () => {
      const results = await call("set_prices", { entries });
      let failed = 0;
      for (const r of results) {
        const err = errors.get(r.asset_id);
        err.textContent = r.ok ? "" : describe({ message: r.error });
        if (!r.ok) failed += 1;
      }
      store.invalidate();
      const saved = results.length - failed;
      if (failed) {
        toast(`Saved ${saved}; ${failed} need${failed === 1 ? "s" : ""} fixing.`, { kind: "warning" });
        for (const r of results) if (r.ok) inputs.get(r.asset_id).dataset.original = inputs.get(r.asset_id).value.trim();
        countChanges();
      } else {
        toast(`Saved ${saved} value${saved === 1 ? "" : "s"}.`, { kind: "success" });
        onDone();
      }
    }, "Saving…");
  } }, "Save values");

  return h("div", { class: "bulk" },
    h("div", { class: "bulk-bar" },
      h("div", {}, h("strong", {}, "Update values"), h("span", { class: "muted" }, " — type the value of each whole holding. Blank rows are left alone.")),
      h("div", { class: "bulk-actions" }, h("label", { class: "inline-label" }, "As of ", asof), dirty, h("button", { class: "btn btn-ghost", onclick: onCancel }, "Cancel"), save)
    ),
    h("table", { class: "table" },
      h("thead", {}, h("tr", {}, h("th", { class: "col-photo" }), h("th", {}, "Name"), h("th", { class: "num" }, "Current"), h("th", { class: "num" }, "New value"))),
      h("tbody", {}, rows)
    )
  );
}
