// The wishlist: things wanted, kept apart from things owned.
//
// A wish is never an asset and never counts toward a total. "I bought it"
// opens the add form filled in from the wish; once saved, the wish is kept
// as got, pointing at the asset it became.

import { call, describe } from "../lib/api.js";
import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { modal, field, select, busy, toast, confirmDialog, emptyState, textInput } from "../ui/components.js";
import { openAddAsset, kindForType, loadCustomKinds } from "./asset-forms.js";

const PRIORITY_LABELS = { high: "High", normal: "Normal", low: "Low" };

export async function renderWishlist(root, _params, ctx) {
  const wishes = await call("list_wishes");
  const wanted = wishes.filter((w) => !w.acquired_asset_id);
  const got = wishes.filter((w) => w.acquired_asset_id);
  const add = h("button", { class: "btn btn-primary", onclick: () => wishEditor(null, ctx) }, icon("plus", { size: 16 }), "Add a wish");

  const row = (w) => h("li", { class: "wish" },
    h("div", { class: "wish-text" },
      h("strong", {}, w.name, " ", w.priority !== "normal" ? h("span", { class: w.priority === "high" ? "badge badge-attention" : "badge badge-muted" }, PRIORITY_LABELS[w.priority]) : null),
      h("span", { class: "muted small" }, [w.type_label, w.quantity !== "1" ? `${fmt.quantity(w.quantity)} wanted` : null, w.target_display ? `up to ${fmt.money(w.target_display)}` : null].filter(Boolean).join(" · ")),
      w.notes ? h("span", { class: "doc-note" }, w.notes) : null,
      w.owned_matches.length
        ? h("span", { class: "warn-text" }, "You may already own this: ", w.owned_matches.map((m, i) => [i ? ", " : "", h("button", { class: "link", onclick: () => ctx.navigate("asset", { id: m.asset_id }) }, m.name)]))
        : null
    ),
    h("div", { class: "btn-row" },
      h("button", { class: "btn btn-secondary btn-sm", onclick: () => bought(w, ctx) }, icon("check", { size: 14 }), "I bought it"),
      h("button", { class: "icon-btn", "aria-label": `Edit ${w.name}`, onclick: () => wishEditor(w, ctx) }, icon("edit", { size: 16 })),
      h("button", { class: "icon-btn", "aria-label": `Remove ${w.name}`, onclick: () => remove(w, ctx) }, icon("trash", { size: 16 }))
    )
  );

  mount(root,
    h("header", { class: "page-head" },
      h("div", {}, h("h1", {}, "Wishlist"), h("p", { class: "page-sub" }, "Things you want — never counted as owned")),
      h("div", { class: "page-actions" }, add)
    ),
    wanted.length
      ? h("section", { class: "card" }, h("ul", { class: "wish-list" }, wanted.map(row)))
      : h("section", { class: "card" }, emptyState({ glyph: "star", title: "Nothing on the wishlist", body: "Add what you are looking for — with the most you would pay — and see at a glance if you already have one." })),
    got.length
      ? h("section", { class: "card" },
          h("div", { class: "card-head" }, h("h2", {}, "Got")),
          h("ul", { class: "wish-list" }, got.map((w) => h("li", { class: "wish" },
            h("div", { class: "wish-text" }, h("strong", {}, w.name), h("span", { class: "muted small" }, `Got ${fmt.date(w.acquired_at)}`)),
            h("div", { class: "btn-row" },
              h("button", { class: "btn btn-ghost btn-sm", onclick: () => ctx.navigate("asset", { id: w.acquired_asset_id }) }, "Open"),
              h("button", { class: "icon-btn", "aria-label": `Remove ${w.name}`, onclick: () => remove(w, ctx) }, icon("trash", { size: 16 }))
            )))))
      : null
  );
}

async function remove(w, ctx) {
  if (!(await confirmDialog({ title: `Remove “${w.name}”?`, message: "It is removed from the wishlist. Nothing in your catalog changes.", confirmLabel: "Remove", danger: true }))) return;
  try { await call("delete_wish", { wishId: w.wish_id }); ctx.refresh(); } catch (e) { toast(describe(e), { kind: "error" }); }
}

/** Catalog it, filled in from the wish, then mark the wish as got. */
async function bought(w, ctx) {
  await loadCustomKinds();
  const seed = {
    name: w.name,
    attrs: {},
    acquired_date: fmt.todayIso(),
    acquired_display: w.target_display,
    acquired_currency: w.currency,
    quantity_unit: "item",
  };
  openAddAsset({
    kind: kindForType(w.type_id),
    seed,
    onSaved: async (assetId) => {
      try {
        await call("wish_acquired", { wishId: w.wish_id, assetId });
        toast("Added to your catalog and marked as got.", { kind: "success" });
      } catch (e) { toast(describe(e), { kind: "error" }); }
      ctx.refresh();
    },
  });
}

async function wishEditor(existing, ctx) {
  const types = await store.types();
  const settings = await store.settings();
  const name = textInput({ value: existing?.name ?? "", maxlength: 300, placeholder: "e.g. Omega Speedmaster, 1960s" });
  const type = select(types.map((t) => [t.type_id, t.label]), existing?.type_id ?? "generic");
  const target = h("input", { type: "text", inputmode: "decimal", class: "input-money", value: existing?.target_display?.split(" ")[0] ?? "", placeholder: "Optional" });
  const currency = select(fmt.COMMON_CURRENCIES.map((c) => [c, c]), existing?.currency ?? settings.currency, { class: "affix affix-select", "aria-label": "Currency" });
  const quantity = textInput({ value: existing?.quantity ?? "1", inputmode: "decimal" });
  const priority = select(Object.entries(PRIORITY_LABELS), existing?.priority ?? "normal");
  const notes = h("textarea", { rows: 3, maxlength: 5000, placeholder: "Condition, variant, where to look" });
  notes.value = existing?.notes ?? "";
  const save = h("button", { class: "btn btn-primary" }, existing ? "Save" : "Add to wishlist");
  const m = modal({
    title: existing ? "Edit wish" : "Add a wish",
    size: "md",
    body: h("div", { class: "form-grid" },
      field("What", name, { span: 2 }),
      field("Type", type),
      field("The most you would pay", h("div", { class: "input-affix" }, currency, target)),
      field("How many", quantity),
      field("Priority", priority),
      field("Notes", notes, { span: 2 })
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  save.addEventListener("click", () => busy(save, async () => {
    await call("save_wish", { form: {
      wish_id: existing?.wish_id ?? null, name: name.value, type_id: type.value, target: target.value.trim() || null,
      currency: currency.value, quantity: quantity.value, priority: priority.value, notes: notes.value,
    } });
    m.close();
    ctx.refresh();
  }));
}
