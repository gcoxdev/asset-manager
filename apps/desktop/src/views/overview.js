// Overview: what the collection is worth, how that has moved, and what
// needs attention.

import { call } from "../lib/api.js";
import { mediaUrl } from "../lib/media.js";
import { h, mount } from "../lib/dom.js";
import { icon, typeIcon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import { segmented, emptyState, toastError } from "../ui/components.js";
import { valueChart } from "../ui/chart.js";
import { openAddAsset, openUpdateValue } from "./asset-forms.js";
import * as store from "../lib/store.js";

export async function renderOverview(root, _params, ctx) {
  const d = await call("dashboard");

  const addButton = h("button", { class: "btn btn-primary", onclick: () => openAddAsset({ onSaved: (id) => ctx.navigate("asset", { id }) }) }, icon("plus", { size: 16 }), "Add asset");

  const header = h("header", { class: "page-head" },
    h("div", {}, h("h1", {}, "Overview"), h("p", { class: "page-sub" }, fmt.date(fmt.todayIso()))),
    h("div", { class: "page-actions" }, addButton)
  );

  if (d.active_count === 0) {
    mount(root, header, welcomeEmpty(ctx));
    return;
  }

  mount(
    root,
    header,
    backupReminder(ctx),
    hero(d),
    tiles(d, ctx),
    chartCard(d),
    h("div", { class: "grid-2" }, allocationCard(d, ctx), topCard(d, ctx)),
    attentionCard(d, ctx),
    recentCard(d, ctx)
  );
}

/**
 * Overdue for a backup, by the owner's own reminder interval. Shown only
 * while the app is open — nothing runs when it is closed.
 */
function backupReminder(ctx) {
  const box = h("div");
  store.settings().then((s) => {
    if (!s?.backup_reminder_days) return;
    const days = s.last_backup_at ? fmt.daysSince(s.last_backup_at) : null;
    if (days !== null && days <= s.backup_reminder_days) return;
    mount(box, h("div", { class: "callout callout-warning reminder-banner" },
      icon("archive", { size: 18 }),
      h("div", {}, h("strong", {}, days === null ? "This vault has never been backed up. " : `Last backup ${days} days ago. `),
        "A disk failure or a lost laptop would take the whole catalog with it."),
      h("button", { class: "btn btn-secondary btn-sm", onclick: () => ctx.navigate("settings", { section: "backup" }) }, "Back up")
    ));
  }).catch(() => {});
  return box;
}

function welcomeEmpty(ctx) {
  const quick = [
    ["metal", "Precious metal", "metal"],
    ["comic", "Comic", "comic"],
    ["trading_card", "Card", "card"],
    ["crypto", "Crypto", "crypto"],
    ["watch", "Watch", "watch"],
    ["generic", "Something else", "item"],
  ];
  return h("div", { class: "card welcome-card" },
    emptyState({
      glyph: "holdings",
      title: "Your catalog is empty",
      body: "Add the first thing you own. Photos, provenance and values can follow — the catalog is useful from the first entry, with or without prices.",
    }),
    h("div", { class: "quick-kinds" },
      quick.map(([kind, label, glyph]) =>
        h("button", { class: "quick-kind", onclick: () => openAddAsset({ kind, onSaved: (id) => ctx.navigate("asset", { id }) }) }, icon(glyph, { size: 22 }), h("span", {}, label))
      )
    ),
    h("p", { class: "muted center" }, "Already keep a spreadsheet? ", h("button", { class: "link", onclick: () => ctx.navigate("reports") }, "Import a CSV"), ".")
  );
}

function hero(d) {
  const total = d.valued + d.unvalued;
  const coverage =
    d.unvalued > 0
      ? h("span", { class: "coverage-note" }, icon("info", { size: 15 }), `${d.valued} of ${total} holdings have a value — the rest are counted, not assumed to be zero`)
      : h("span", { class: "coverage-note ok" }, icon("check", { size: 15 }), `All ${total} holdings valued`);

  let gain = null;
  if (d.gain) {
    const cost = Number(d.cost_basis.minor);
    const pct = cost > 0 ? Number(d.gain.minor) / cost : NaN;
    const up = !d.gain.display.startsWith("-");
    gain = h("div", { class: `gain-chip ${up ? "up" : "down"}` },
      icon(up ? "up" : "downRight", { size: 16 }),
      h("strong", {}, fmt.signedMoney(d.gain.display)),
      Number.isFinite(pct) ? h("span", {}, `${up ? "+" : ""}${fmt.percent(pct)}`) : null,
      h("span", { class: "gain-context" }, `vs. cost, across ${d.gain_coverage} holding${d.gain_coverage === 1 ? "" : "s"} with a known price paid`)
    );
  }
  const partial = d.partial_cost > 0
    ? h("p", { class: "muted small" }, `${d.partial_cost} holding${d.partial_cost === 1 ? " is" : "s are"} left out of the gain: more was added without a price, so the recorded cost covers only part of ${d.partial_cost === 1 ? "it" : "them"}.`)
    : null;

  return h("section", { class: "hero card" },
    h("div", { class: "hero-label" }, "Collection value"),
    h("div", { class: "hero-value" }, fmt.amount(d.total)),
    h("div", { class: "hero-meta" }, coverage, gain),
    partial,
    d.skipped_currencies.length
      ? h("p", { class: "muted small" }, `Excludes holdings valued in ${d.skipped_currencies.join(", ")} — there is no currency conversion yet.`)
      : null
  );
}

function tile(label, value, sub, { glyph, onClick, tone } = {}) {
  const tag = onClick ? "button" : "div";
  return h(tag, { class: ["tile", onClick ? "tile-link" : null, tone ? `tile-${tone}` : null], onclick: onClick ?? null },
    h("div", { class: "tile-label" }, glyph ? icon(glyph, { size: 16 }) : null, label),
    h("div", { class: "tile-value" }, value),
    sub ? h("div", { class: "tile-sub" }, sub) : null
  );
}

function tiles(d, ctx) {
  return h("section", { class: "tiles" },
    tile("Holdings", String(d.active_count), `${d.by_category.length} categor${d.by_category.length === 1 ? "y" : "ies"}`, { glyph: "holdings", onClick: () => ctx.navigate("holdings") }),
    tile("Cost of valued holdings", d.gain_coverage ? fmt.amount(d.cost_basis) : "—", d.gain_coverage ? `${d.gain_coverage} with a known price paid` : "Add what you paid to see gain", { glyph: "tag" }),
    tile("Need a value", String(d.unvalued), d.unvalued ? "Counted, but not in the total" : "Nothing missing", { glyph: "info", tone: d.unvalued ? "attention" : null, onClick: d.unvalued ? () => ctx.navigate("holdings", { filter: "unvalued" }) : null }),
    tile("Due for review", String(d.review_due.length), d.review_due.length ? "Reminders you set" : "No reminders due", { glyph: "clock", tone: d.review_due.length ? "attention" : null, onClick: d.review_due.length ? () => ctx.navigate("holdings", { filter: "review" }) : null })
  );
}

const RANGES = [["90", "3M"], ["365", "1Y"], ["1095", "3Y"], ["all", "All"]];

function chartCard(d) {
  const body = h("div", { class: "chart-host" });
  const notes = h("p", { class: "chart-notes" });
  const table = h("div", { class: "chart-table", hidden: true });
  let showTable = false;
  let lastPoints = [];

  // Switching ranges quickly can return the older request last; only the
  // latest one may draw.
  let latest = 0;
  const load = async (range) => {
    const request = ++latest;
    const from = range === "all" ? null : fmt.daysAgoIso(Number(range));
    let series;
    try {
      series = await call("portfolio_series", { from, maxPoints: 160 });
    } catch (error) {
      if (request === latest) toastError(error);
      return;
    }
    if (request !== latest) return;
    const points = series.points.map((p) => ({
      date: p.date,
      display: p.total,
      value: fmt.approxMajor(p.total_minor, fmt.digitsOf(p.total)),
      unvalued: p.unvalued,
      valued: p.valued,
      event: p.quantity_event,
    }));
    lastPoints = points;
    if (points.length < 2 || points.every((p) => p.value === 0)) {
      mount(body, h("div", { class: "chart-empty" }, icon("markets", { size: 22 }), h("p", {}, "The chart fills in as values are recorded over time.")));
      notes.textContent = "";
      return;
    }
    mount(body, valueChart(points, { currency: series.currency, label: "Collection value" }));
    const parts = [];
    if (!series.complete) parts.push("Dashed where some holdings had no value on that date.");
    parts.push("Dots mark dates a holding changed — a step there is a purchase or sale, not a gain.");
    if (series.skipped_currencies.length) parts.push(`Excludes ${series.skipped_currencies.join(", ")}.`);
    notes.textContent = parts.join(" ");
    if (showTable) renderTable();
  };

  const renderTable = () => {
    const rows = lastPoints.filter((p, i) => p.event || i === lastPoints.length - 1 || i % Math.ceil(lastPoints.length / 12) === 0);
    mount(table, h("table", { class: "table compact" },
      h("thead", {}, h("tr", {}, h("th", {}, "Date"), h("th", { class: "num" }, "Value"), h("th", { class: "num" }, "Unpriced"))),
      h("tbody", {}, rows.map((p) => h("tr", {}, h("td", {}, fmt.date(p.date)), h("td", { class: "num" }, fmt.money(p.display)), h("td", { class: "num" }, String(p.unvalued)))))
    ));
  };

  const seg = segmented(RANGES, "365", (v) => load(v));
  const tableToggle = h("button", { class: "btn btn-ghost btn-sm", onclick: () => {
    showTable = !showTable;
    table.hidden = !showTable;
    tableToggle.textContent = showTable ? "Hide table" : "Show as table";
    if (showTable) renderTable();
  } }, "Show as table");

  load("365");
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("div", {}, h("h2", {}, "Value over time"), h("p", { class: "card-sub" }, "What the collection was worth on each date — value, not return")), h("div", { class: "card-tools" }, seg.el)),
    body,
    h("div", { class: "chart-foot" }, notes, tableToggle),
    table
  );
}

function allocationCard(d, ctx) {
  const total = Number(d.total.minor) || 0;
  const rows = d.by_category.map((c) => {
    const share = total > 0 ? Number(c.value.minor) / total : 0;
    return h("button", { class: "alloc-row", onclick: () => ctx.navigate("holdings", { category: c.category }) },
      h("div", { class: "alloc-top" },
        h("span", { class: "alloc-name" }, fmt.categoryLabel(c.category)),
        h("span", { class: "alloc-value" }, fmt.amount(c.value)),
      ),
      h("div", { class: "alloc-bar", role: "presentation" }, h("span", { style: { width: `${Math.max(share * 100, share > 0 ? 1.5 : 0)}%` } })),
      h("div", { class: "alloc-meta" },
        h("span", {}, `${c.count} holding${c.count === 1 ? "" : "s"}${c.unvalued ? ` · ${c.unvalued} unpriced` : ""}`),
        h("span", {}, total > 0 ? fmt.percent(share) : "—")
      )
    );
  });
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, "Allocation")),
    h("div", { class: "alloc" }, rows)
  );
}

function thumb(brief, size = 40) {
  return brief.primary_photo
    ? h("img", { class: "thumb", src: mediaUrl(brief.primary_photo, 256), alt: "", width: size, height: size, loading: "lazy" })
    : h("span", { class: "thumb thumb-glyph", style: { width: `${size}px`, height: `${size}px` } }, icon(typeIcon(brief.type_id, brief.category), { size: 18 }));
}

function topCard(d, ctx) {
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, "Largest holdings"), h("button", { class: "btn btn-ghost btn-sm", onclick: () => ctx.navigate("holdings", { sort: "value" }) }, "View all")),
    d.top_holdings.length
      ? h("ul", { class: "brief-list" }, d.top_holdings.map((b) =>
          h("li", {}, h("button", { class: "brief", onclick: () => ctx.navigate("asset", { id: b.asset_id }) },
            thumb(b),
            h("span", { class: "brief-text" }, h("strong", {}, b.name), h("span", {}, b.type_label)),
            h("span", { class: "brief-value" }, fmt.amount(b.value))
          ))
        ))
      : h("p", { class: "muted" }, "Values you record will rank here.")
  );
}

function attentionCard(d, ctx) {
  if (!d.needs_value.length && !d.review_due.length) return null;
  const item = (b, action) =>
    h("li", { class: "attention-item" },
      h("button", { class: "brief", onclick: () => ctx.navigate("asset", { id: b.asset_id }) },
        thumb(b, 36),
        h("span", { class: "brief-text" }, h("strong", {}, b.name), h("span", {}, b.reason))
      ),
      action
    );
  const valueButton = (b) =>
    h("button", { class: "btn btn-secondary btn-sm", onclick: async () => {
      const all = await store.assets();
      const asset = all.find((a) => a.asset_id === b.asset_id);
      if (asset) openUpdateValue(asset, { onSaved: () => ctx.refresh() });
    } }, "Set value");

  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, "Needs attention")),
    h("div", { class: "grid-2 inner" },
      d.needs_value.length
        ? h("div", {}, h("h3", { class: "list-title" }, `No value yet · ${d.unvalued}`), h("ul", { class: "brief-list" }, d.needs_value.slice(0, 6).map((b) => item(b, valueButton(b)))))
        : null,
      d.review_due.length
        ? h("div", {}, h("h3", { class: "list-title" }, `Due for review · ${d.review_due.length}`), h("ul", { class: "brief-list" }, d.review_due.slice(0, 6).map((b) => item(b, valueButton(b)))))
        : null
    )
  );
}

function recentCard(d, ctx) {
  if (!d.recent.length) return null;
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, "Recent activity")),
    h("ul", { class: "timeline" }, d.recent.map((e) =>
      h("li", {},
        h("span", { class: `timeline-dot ev-${e.event_type}` }),
        h("button", { class: "link-plain", onclick: () => ctx.navigate("asset", { id: e.asset_id }) }, e.name),
        h("span", { class: "muted" }, ` — ${fmt.EVENT_LABELS[e.event_type] ?? e.event_type}${e.event_type === "dispose" ? "" : ` ${fmt.quantity(e.quantity_delta.replace(/^-/, ""))}`}`),
        h("span", { class: "timeline-date" }, fmt.date(e.effective_date))
      )
    ))
  );
}
