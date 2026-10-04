// Holdings: the catalog as a searchable, sortable list — plus bulk value
// entry, because setting prices one dialog at a time does not scale to a few
// hundred collectibles.

import { call, describe } from "../lib/api.js";
import { mediaUrl } from "../lib/media.js";
import { h, mount, debounce } from "../lib/dom.js";
import { icon, typeIcon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import { cmpMoney, sumDisplay } from "../lib/money.js";
import * as store from "../lib/store.js";
import { sourceBadge, statusBadge, emptyState, select, segmented, busy, toast, toastError, menuButton, modal, field, confirmDialog, tagsInput, suggestInput } from "../ui/components.js";
import { openAddAsset } from "./asset-forms.js";
import { exportCsv, importCsv } from "./reports.js";
import { importSpreadsheet } from "./spreadsheet-import.js";

const PAGE = 300;

const SORTS = [
  ["updated", "Recently changed"],
  ["name", "Name"],
  ["value", "Value, high to low"],
  ["gain", "Gain, high to low"],
  ["acquired", "Date acquired"],
];

export async function renderHoldings(root, params, ctx) {
  const prefs = store.ui();
  if (params.category) prefs.category = params.category;
  if (params.sort) prefs.sort = params.sort;
  let special = params.filter ?? null; // "unvalued" | "review"
  if (special) prefs.status = "active";

  const [assets, settings, tagList, locationList, savedViews] = await Promise.all([
    store.assets(),
    store.settings(),
    call("list_tags"),
    call("list_locations"),
    call("list_saved_views"),
  ]);
  let searchIds = null;
  let bulk = false;
  let shown = PAGE;
  // Selecting many for one change. Kept across redraws (sorting, filtering)
  // until the action is done or selection is turned off.
  let selecting = false;
  const selected = new Set();
  // Unsaved bulk values survive a redraw — a sort, a filter — rather than
  // vanishing; leaving bulk entry with any asks first.
  const drafts = new Map();

  const search = h("input", { type: "search", class: "search-input", placeholder: "Search names, notes, cert numbers…", value: prefs.query, "data-search": "", "aria-label": "Search holdings" });
  const results = h("div", { class: "results" });
  const summary = h("div", { class: "results-summary" });
  const chips = h("div", { class: "chips", role: "group", "aria-label": "Category" });

  const statusSelect = select([["active", "Held"], ["sold", "Sold"], ["inactive", "Lost & retired"], ["all", "Everything"]], prefs.status, { "aria-label": "Status" });
  statusSelect.addEventListener("change", () => { prefs.status = statusSelect.value; shown = PAGE; draw(); });
  const sortSelect = select(SORTS, prefs.sort, { "aria-label": "Sort" });
  sortSelect.addEventListener("change", () => { prefs.sort = sortSelect.value; draw(); });
  const layout = segmented([["list", "List"], ["grid", "Grid"]], prefs.layout, (v) => { prefs.layout = v; draw(); });

  // --- filters beyond category and status ---------------------------------
  const tagSelect = select([["", "Any tag"], ...tagList.map((t) => [t.name, `${t.name} (${t.count})`])], prefs.tag, { "aria-label": "Tag" });
  tagSelect.addEventListener("change", () => { prefs.tag = tagSelect.value; shown = PAGE; draw(); });
  const locationSelect = select([["", "Any location"], ...locationList.map((l) => [l.name, l.name])], prefs.location, { "aria-label": "Location" });
  locationSelect.addEventListener("change", () => { prefs.location = locationSelect.value; shown = PAGE; draw(); });
  const missingSelect = select([
    ["", "Nothing missing"], ["photo", "No photo"], ["document", "No documents"], ["value", "No value"], ["insurance", "No insured value"], ["review", "Due for review"],
  ], prefs.missing, { "aria-label": "Missing" });
  missingSelect.addEventListener("change", () => { prefs.missing = missingSelect.value; shown = PAGE; draw(); });

  // --- saved views ------------------------------------------------------------
  const VIEW_KEYS = ["query", "category", "status", "sort", "tag", "location", "missing"];
  const applyView = (view) => {
    for (const k of VIEW_KEYS) if (k in view) prefs[k] = view[k];
    ctx.refresh();
  };
  const viewsMenu = menuButton(h("button", { class: "btn btn-secondary", "aria-label": "Saved views" }, icon("filter", { size: 16 }), "Views"), [
    ...savedViews.map((v) => ({ label: v.name, onSelect: () => applyView(v.view) })),
    savedViews.length ? "divider" : null,
    { label: "Save this view…", icon: "plus", onSelect: () => saveView() },
    savedViews.length ? { label: "Delete a view…", icon: "trash", onSelect: () => deleteView() } : null,
  ]);
  async function saveView() {
    const name = h("input", { type: "text", maxlength: 80, autofocus: true, placeholder: "e.g. Uninsured watches in the safe" });
    const m = modal({
      title: "Save this view",
      size: "sm",
      body: h("div", { class: "stack" },
        field("Name", name),
        h("p", { class: "field-hint" }, "Saves the search, filters and sort as they are now. Saved views are kept inside the encrypted vault.")),
      footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Cancel"), h("button", { class: "btn btn-primary", onclick: () => close(true) }, "Save")],
    });
    if (!(await m.done)) return;
    const view = Object.fromEntries(VIEW_KEYS.map((k) => [k, prefs[k]]));
    try {
      await call("save_view", { name: name.value, view });
      toast("View saved.", { kind: "success" });
      ctx.refresh();
    } catch (e) { toastError(e); }
  }
  async function deleteView() {
    const choice = select(savedViews.map((v) => [v.name, v.name]), savedViews[0]?.name);
    const m = modal({
      title: "Delete a saved view",
      size: "sm",
      body: field("View", choice),
      footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Cancel"), h("button", { class: "btn btn-danger", onclick: () => close(true) }, "Delete")],
    });
    if (!(await m.done)) return;
    try {
      await call("delete_saved_view", { name: choice.value });
      ctx.refresh();
    } catch (e) { toastError(e); }
  }

  const selectButton = h("button", { class: "btn btn-secondary", "aria-pressed": "false", onclick: () => {
    selecting = !selecting;
    if (!selecting) selected.clear();
    selectButton.setAttribute("aria-pressed", String(selecting));
    selectButton.classList.toggle("active", selecting);
    if (selecting) prefs.layout = "list";
    draw();
  } }, icon("check", { size: 16 }), "Select");

  // Searches can finish out of order — a short query is slower than the
  // longer one typed after it — so only the latest is applied, and none once
  // the view has gone.
  let searchSeq = 0;
  let disposed = false;
  const runSearch = debounce(async () => {
    const seq = ++searchSeq;
    prefs.query = search.value;
    const q = search.value.trim();
    let ids = null;
    try {
      ids = q ? await call("search_assets", { query: q }) : null;
    } catch (error) {
      if (seq === searchSeq && !disposed) toastError(error);
      return;
    }
    if (seq !== searchSeq || disposed) return;
    searchIds = ids;
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
    { label: "Tags & locations…", icon: "tag", onSelect: () => manageNames(tagList, locationList, ctx) },
    "divider",
    { label: "Import a spreadsheet…", icon: "upload", onSelect: () => importSpreadsheet(() => ctx.refresh()) },
    { label: "Re-import an export…", icon: "upload", onSelect: () => importCsv(() => ctx.refresh()) },
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
    if (prefs.tag) list = list.filter((a) => a.tags?.some((t) => t.toLowerCase() === prefs.tag.toLowerCase()));
    // A location includes the places inside it: "Safe" finds "Safe / Top shelf".
    if (prefs.location) list = list.filter((a) => a.storage_location === prefs.location || a.storage_location?.startsWith(`${prefs.location} / `));
    switch (prefs.missing) {
      case "photo": list = list.filter((a) => !a.primary_photo); break;
      case "document": list = list.filter((a) => !a.document_count); break;
      case "value": list = list.filter((a) => a.current_amount_minor == null); break;
      case "insurance": list = list.filter((a) => a.insured_amount_minor == null); break;
      case "review": list = list.filter((a) => a.review_due); break;
      default: break;
    }

    if (searchIds) {
      const rank = new Map(searchIds.map((id, i) => [id, i]));
      list = list.filter((a) => rank.has(a.asset_id)).sort((a, b) => rank.get(a.asset_id) - rank.get(b.asset_id));
      return list; // relevance order while searching
    }
    const sorted = [...list];
    switch (prefs.sort) {
      case "name": sorted.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" })); break;
      // Gain exists only when value and cost share a currency, so it is in
      // the value's currency.
      case "value": sorted.sort((a, b) => cmpMoney(a.current_amount_minor, a.current_currency, b.current_amount_minor, b.current_currency, settings.currency)); break;
      case "gain": sorted.sort((a, b) => cmpMoney(a.gain_minor, a.current_currency, b.gain_minor, b.current_currency, settings.currency)); break;
      case "acquired": sorted.sort((a, b) => (b.acquired_date ?? "").localeCompare(a.acquired_date ?? "")); break;
      default: break; // backend order: most recently changed first
    }
    return sorted;
  }

  function drawChips() {
    const counts = new Map();
    for (const a of assets) if (a.status === "active") counts.set(a.category, (counts.get(a.category) ?? 0) + 1);
    const order = ["metals", "crypto", "collectibles", "valuables", "firearms", "cash", "other"];
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
    const foreign = [...new Set(list.filter((a) => a.current_amount_minor != null && a.current_currency !== settings.currency).map((a) => a.current_currency))].sort();
    const foreignCount = list.filter((a) => a.current_amount_minor != null && a.current_currency !== settings.currency).length;
    mount(summary,
      h("span", {}, `${list.length} ${list.length === 1 ? "holding" : "holdings"}`),
      total ? h("span", {}, " · ", h("strong", {}, fmt.money(total))) : null,
      foreignCount
        ? h("span", { class: "muted", title: "There is no currency conversion, so these are not in the total." },
            ` · ${foreignCount} valued in ${foreign.join(", ")} not included`)
        : null,
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

    const leaveBulk = async () => {
      const unsaved = [...drafts.values()].filter((d) => d.value !== d.original).length;
      if (unsaved && !(await confirmDialog({ title: "Discard unsaved values?", message: `${unsaved} value${unsaved === 1 ? " has" : "s have"} not been saved.`, confirmLabel: "Discard", danger: true }))) return;
      drafts.clear();
      bulk = false;
      draw();
    };
    if (bulk) mount(results, bulkTable(page, settings, drafts, () => { bulk = false; drafts.clear(); ctx.refresh(); }, leaveBulk), moreButton);
    else if (prefs.layout === "grid") mount(results, grid(page, ctx), moreButton);
    else mount(results, selecting ? selectionBar(list, page) : null, table(page, ctx, selecting ? { selected, onToggle: () => draw() } : null), moreButton);
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
    h("div", { class: "toolbar toolbar-filters" },
      h("div", { class: "toolbar-right" }, tagSelect, locationSelect, missingSelect),
      h("div", { class: "toolbar-right" }, viewsMenu, selectButton)
    ),
    chips,
    summary,
    results
  );

  /** What can be done to everything selected. */
  function selectionBar(list, page) {
    const ids = () => [...selected];
    const n = selected.size;
    const run = async (label, action) => {
      try {
        const count = await action();
        if (count === undefined) return;
        store.invalidate();
        toast(`${label} — ${count} asset${count === 1 ? "" : "s"}.`, { kind: "success" });
        selected.clear();
        ctx.refresh();
      } catch (e) { toastError(e); }
    };
    const bulkEdit = (label, change) => run(label, () => call("bulk_edit", { assetIds: ids(), change }));
    const ask = async (title, control, hint) => {
      const m = modal({
        title,
        subtitle: `${n} selected`,
        size: "sm",
        body: h("div", { class: "stack" }, control.el ?? control, hint ? h("p", { class: "field-hint" }, hint) : null),
        footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Cancel"), h("button", { class: "btn btn-primary", onclick: () => close(true) }, "Apply")],
      });
      return m.done;
    };
    const actions = [
      h("button", { class: "btn btn-secondary btn-sm", disabled: !n, onclick: async () => {
        const tags = tagsInput([], tagList.map((t) => t.name), { label: "Tags to add" });
        if (await ask("Add tags", tags)) bulkEdit("Tagged", { add_tags: tags.value() });
      } }, "Add tags…"),
      h("button", { class: "btn btn-secondary btn-sm", disabled: !n, onclick: async () => {
        const present = [...new Set(assets.filter((a) => selected.has(a.asset_id)).flatMap((a) => a.tags ?? []))];
        if (!present.length) return toast("None of the selected assets have tags.", { kind: "info" });
        const choice = select(present.map((t) => [t, t]), present[0]);
        if (await ask("Remove a tag", field("Tag", choice))) bulkEdit("Untagged", { remove_tags: [choice.value] });
      } }, "Remove tag…"),
      h("button", { class: "btn btn-secondary btn-sm", disabled: !n, onclick: async () => {
        const where = suggestInput({ placeholder: "Leave blank to clear the location", maxlength: 500 }, locationList.map((l) => l.name));
        if (await ask("Move to", field("Storage location", where.el), "Each item's previous location is kept in its edit history.")) {
          bulkEdit("Moved", { storage_location: where.input.value.trim() || null });
        }
      } }, "Move to…"),
      h("button", { class: "btn btn-secondary btn-sm", disabled: !n, onclick: async () => {
        const review = select([["", "Never"], ["30", "Every month"], ["90", "Every 3 months"], ["180", "Every 6 months"], ["365", "Every year"]], "");
        if (await ask("Revaluation reminder", field("Remind me to revalue", review))) {
          bulkEdit("Reminder set", { review_every_days: review.value ? Number(review.value) : null });
        }
      } }, "Reminder…"),
      h("button", { class: "btn btn-secondary btn-sm", disabled: !n, onclick: () => ctx.navigate("reports", { claim: ids() }) }, "Start a claim…"),
      h("button", { class: "btn btn-ghost btn-sm danger-text", disabled: !n, onclick: async () => {
        const ok = await confirmDialog({ title: `Move ${n} asset${n === 1 ? "" : "s"} to the trash?`, message: "They leave the catalog, totals and reports, and can be restored from Settings → Trash for 30 days.", confirmLabel: "Move to trash", danger: true });
        if (ok) run("Moved to trash", () => call("bulk_trash", { assetIds: ids() }));
      } }, "Trash…"),
    ];
    const allShown = page.every((a) => selected.has(a.asset_id));
    return h("div", { class: "bulk-bar selection-bar", role: "region", "aria-label": "Selection" },
      h("div", {},
        h("strong", {}, n ? `${n} selected` : "Select assets"),
        " ",
        h("button", { class: "link", onclick: () => {
          if (allShown) for (const a of page) selected.delete(a.asset_id);
          else for (const a of page) selected.add(a.asset_id);
          draw();
        } }, allShown ? "Clear shown" : `Select all ${page.length} shown`),
        list.length > page.length ? h("span", { class: "muted small" }, ` (of ${list.length} — show all to select the rest)`) : null
      ),
      h("div", { class: "bulk-actions" }, actions)
    );
  }

  if (prefs.query) await runSearch();
  else draw();
  if (params.focusSearch) search.focus();
  return () => {
    disposed = true;
  };
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

function table(list, ctx, selection = null) {
  const toggleRow = (a) => {
    if (selection.selected.has(a.asset_id)) selection.selected.delete(a.asset_id);
    else selection.selected.add(a.asset_id);
    selection.onToggle();
  };
  const open = (a) => (selection ? toggleRow(a) : ctx.navigate("asset", { id: a.asset_id }));
  return h("table", { class: selection ? "table holdings-table selecting" : "table holdings-table" },
    h("thead", {}, h("tr", {},
      selection ? h("th", { class: "col-check" }, h("span", { class: "sr-only" }, "Selected")) : null,
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
        "aria-selected": selection ? String(selection.selected.has(a.asset_id)) : null,
        onkeydown: (e) => {
          if (e.key === "Enter" || (selection && e.key === " ")) { e.preventDefault(); open(a); }
          if (e.key === "ArrowDown") { e.preventDefault(); e.currentTarget.nextElementSibling?.focus(); }
          if (e.key === "ArrowUp") { e.preventDefault(); e.currentTarget.previousElementSibling?.focus(); }
        },
      },
        selection
          ? h("td", { class: "col-check" }, h("input", {
              type: "checkbox",
              checked: selection.selected.has(a.asset_id),
              "aria-label": `Select ${a.name}`,
              onclick: (e) => { e.stopPropagation(); toggleRow(a); },
            }))
          : null,
        h("td", { class: "col-photo" }, thumb(a)),
        h("td", {}, h("div", { class: "name-cell" },
          h("span", { class: "name" }, a.name, statusBadge(a.status)),
          h("span", { class: "sub" }, subtitle(a)),
          a.tags?.length ? h("span", { class: "tag-list" }, a.tags.map((t) => h("span", { class: "tag-chip" }, t))) : null
        )),
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
function bulkTable(list, settings, drafts, onDone, onCancel) {
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
    const draft = drafts.get(a.asset_id);
    const input = h("input", { type: "text", inputmode: "decimal", class: "input-money bulk-input", value: draft ? draft.value : original, placeholder: "—", "aria-label": `Value of ${a.name}` });
    input.dataset.original = original;
    input.addEventListener("input", () => {
      drafts.set(a.asset_id, { value: input.value.trim(), original });
      countChanges();
    });
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
        for (const r of results) {
          if (!r.ok) continue;
          inputs.get(r.asset_id).dataset.original = inputs.get(r.asset_id).value.trim();
          drafts.delete(r.asset_id);
        }
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

/**
 * Rename a tag (merging into another of that name), or move a location —
 * everything stored there, and in places inside it, comes along.
 */
function manageNames(tagList, locationList, ctx) {
  const row = (kind, item) => {
    const input = h("input", { type: "text", value: item.name, maxlength: kind === "tag" ? 60 : 500, "aria-label": `New name for ${item.name}` });
    const apply = h("button", { class: "btn btn-secondary btn-sm", onclick: () => busy(apply, async () => {
      const to = input.value.trim();
      if (!to || to === item.name) return;
      if (kind === "tag") {
        await call("rename_tag", { from: item.name, to });
        toast(`Renamed tag to “${to}”.`, { kind: "success" });
      } else {
        const n = await call("rename_location", { from: item.name, to });
        toast(`Moved ${n} asset${n === 1 ? "" : "s"} to “${to}”.`, { kind: "success" });
      }
      store.invalidate();
      m.close();
      ctx.refresh();
    }) }, kind === "tag" ? "Rename" : "Move");
    return h("tr", {}, h("td", {}, input), h("td", { class: "num muted small" }, String(item.count)), h("td", { class: "row-action" }, apply));
  };
  const section = (title, hint, kind, items) => h("section", { class: "stack" },
    h("h3", {}, title),
    h("p", { class: "field-hint" }, hint),
    items.length
      ? h("table", { class: "table compact" }, h("tbody", {}, items.map((i) => row(kind, i))))
      : h("p", { class: "muted small" }, kind === "tag" ? "No tags yet." : "No locations yet.")
  );
  const m = modal({
    title: "Tags & locations",
    size: "md",
    body: h("div", { class: "stack" },
      section("Tags", "Renaming onto an existing tag merges the two.", "tag", tagList),
      section("Locations", "Moving a location moves everything at it, including places inside it (Safe / Top shelf). Each item's previous location stays in its edit history.", "location", locationList)
    ),
  });
}
