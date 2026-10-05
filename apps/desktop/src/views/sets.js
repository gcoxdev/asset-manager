// Sets, splitting a holding, and dividing a purchase among items.
//
// A set is a list of items with an optional target count ("12 of 20"). It
// has no value of its own — its value is its members' — so nothing is ever
// counted twice.

import { call, describe } from "../lib/api.js";
import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { modal, field, select, busy, toast, confirmDialog, callout, textInput } from "../ui/components.js";

function completion(s) {
  return s.target_count ? `${s.members} of ${s.target_count}` : `${s.members} item${s.members === 1 ? "" : "s"}`;
}

/** All sets: create, open, edit, delete. */
export async function openSets(ctx, { focus = null } = {}) {
  const m = modal({ title: "Sets", size: "lg", body: h("div") });
  const body = m.dialog.querySelector(".modal-body");

  async function listStep() {
    const sets = await call("list_sets");
    mount(body,
      h("p", { class: "lede" }, "Group items that belong together — a coin set, a card checklist, a tool kit — and see how complete it is and what it is worth together."),
      h("div", { class: "btn-row" }, h("button", { class: "btn btn-secondary btn-sm", onclick: () => editStep(null) }, icon("plus", { size: 14 }), "New set")),
      sets.length
        ? h("table", { class: "table compact", style: { marginTop: "12px" } }, h("tbody", {}, sets.map((s) => h("tr", { class: "row-link", tabindex: "0", onclick: () => setStep(s.set_id), onkeydown: (e) => e.key === "Enter" && setStep(s.set_id) },
            h("td", {}, h("div", { class: "name-cell" }, h("span", {}, s.name), h("span", { class: "sub" }, completion(s)))),
            h("td", { class: "num" }, fmt.money(s.value_display), s.unvalued ? h("div", { class: "muted small" }, `${s.unvalued} without a value`) : null),
            h("td", { class: "num" }, s.target_count ? h("progress", { max: s.target_count, value: Math.min(s.members, s.target_count), "aria-label": `${completion(s)} collected` }) : null)
          ))))
        : h("p", { class: "muted small", style: { marginTop: "12px" } }, "No sets yet. Select items in Holdings and choose “Add to set…”, or start one here.")
    );
  }

  async function setStep(setId) {
    const [sets, members] = await Promise.all([call("list_sets"), call("set_members", { setId })]);
    const s = sets.find((x) => x.set_id === setId);
    if (!s) return listStep();
    mount(body,
      h("div", { class: "card-head" },
        h("div", {}, h("h3", {}, s.name), h("p", { class: "card-sub" }, `${completion(s)} · ${fmt.money(s.value_display)}${s.notes ? ` · ${s.notes}` : ""}`)),
        h("div", { class: "btn-row" },
          h("button", { class: "btn btn-ghost btn-sm", onclick: () => editStep(s) }, "Edit"),
          members.length > 1 ? h("button", { class: "btn btn-secondary btn-sm", onclick: () => dividePurchase(members.map((x) => x.asset_id), ctx, () => setStep(setId)) }, "Divide a purchase…") : null,
          h("button", { class: "btn btn-ghost btn-sm danger-text", onclick: async () => {
            if (!(await confirmDialog({ title: `Delete the set “${s.name}”?`, message: "The set goes; its items stay in your catalog.", confirmLabel: "Delete set", danger: true }))) return;
            await call("delete_set", { setId });
            listStep();
          } }, "Delete"))),
      h("table", { class: "table compact" }, h("tbody", {}, members.map((x) => h("tr", {},
        h("td", {}, h("button", { class: "link", onclick: () => { m.close(); ctx.navigate("asset", { id: x.asset_id }); } }, x.name), " ", x.status !== "active" ? h("span", { class: "badge badge-muted" }, fmt.STATUS_LABELS[x.status] ?? x.status) : null),
        h("td", { class: "muted small" }, x.type_label),
        h("td", { class: "num" }, x.current_display ? fmt.money(x.current_display) : "—"),
        h("td", { class: "row-action" }, h("button", { class: "icon-btn", "aria-label": `Remove ${x.name} from the set`, onclick: async () => {
          await call("remove_from_set", { setId, assetId: x.asset_id });
          setStep(setId);
        } }, icon("x", { size: 14 })))
      )))),
      h("div", { class: "form-actions" }, h("div", { class: "btn-row" }, h("button", { class: "btn btn-ghost", onclick: () => listStep() }, icon("back", { size: 16 }), "All sets")))
    );
  }

  function editStep(existing) {
    const name = textInput({ value: existing?.name ?? "", maxlength: 200, placeholder: "e.g. Morgan dollars 1878–1921" });
    const target = textInput({ value: existing?.target_count ?? "", inputmode: "numeric", placeholder: "Optional" });
    const notes = textInput({ value: existing?.notes ?? "", maxlength: 2000 });
    const error = h("p", { class: "form-error", role: "alert" });
    const save = h("button", { class: "btn btn-primary" }, existing ? "Save" : "Create set");
    save.addEventListener("click", () => busy(save, async () => {
      error.textContent = "";
      try {
        const id = await call("save_set", { setId: existing?.set_id ?? null, name: name.value, targetCount: target.value.trim() ? Number(target.value) : null, notes: notes.value, add: null });
        setStep(id);
      } catch (e) { error.textContent = describe(e); }
    }));
    mount(body,
      h("div", { class: "form-grid" }, field("Name", name, { span: 2 }), field("How many make it complete", target), field("Notes", notes)),
      error,
      h("div", { class: "form-actions" }, h("div", { class: "btn-row" }, h("button", { class: "btn btn-ghost", onclick: () => (existing ? setStep(existing.set_id) : listStep()) }, "Cancel"), save))
    );
  }

  (focus ? setStep(focus) : listStep()).catch((e) => toast(describe(e), { kind: "error" }));
  await m.done;
  store.invalidate();
  ctx.refresh();
}

/** Add items to an existing set, or a new one. */
export async function addToSet(assetIds, ctx) {
  const sets = await call("list_sets");
  const choice = select([["", "A new set…"], ...sets.map((s) => [s.set_id, `${s.name} (${completion(s)})`])], sets[0]?.set_id ?? "");
  const name = textInput({ placeholder: "Name of the new set", maxlength: 200 });
  const nameField = field("New set", name);
  nameField.hidden = choice.value !== "";
  choice.addEventListener("change", () => (nameField.hidden = choice.value !== ""));
  const m = modal({
    title: `Add ${assetIds.length} to a set`,
    size: "sm",
    body: h("div", { class: "stack" }, field("Set", choice), nameField),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Cancel"), h("button", { class: "btn btn-primary", onclick: () => close(true) }, "Add")],
  });
  if (!(await m.done)) return;
  try {
    const existing = sets.find((s) => s.set_id === choice.value);
    await call("save_set", { setId: existing?.set_id ?? null, name: existing?.name ?? name.value, targetCount: existing?.target_count ?? null, notes: existing?.notes ?? "", add: assetIds });
    toast("Added to the set.", { kind: "success" });
    ctx.refresh();
  } catch (e) { toast(describe(e), { kind: "error" }); }
}

/** Divide one purchase's price among the items it bought. */
export async function dividePurchase(assetIds, ctx, after = null) {
  const settings = await store.settings();
  const total = h("input", { type: "text", inputmode: "decimal", class: "input-money", placeholder: "0.00" });
  const currency = select(fmt.COMMON_CURRENCIES.map((c) => [c, c]), settings.currency, { class: "affix affix-select", "aria-label": "Currency" });
  const method = select([["equal", "Equally"], ["value", "In proportion to their current values"]], "equal");
  const m = modal({
    title: "Divide a purchase",
    subtitle: `${assetIds.length} items bought together`,
    size: "sm",
    body: h("div", { class: "stack" },
      h("p", {}, "For a lot or a set bought for one price: each item's cost becomes its share, adding up to the price exactly."),
      field("Price paid for all of them", h("div", { class: "input-affix" }, currency, total)),
      field("Divide", method),
      callout("info", "This replaces each item's recorded cost; the earlier figures stay in their edit history.")
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Cancel"), h("button", { class: "btn btn-primary", onclick: () => close(true) }, "Divide")],
  });
  if (!(await m.done)) return;
  try {
    const shares = await call("allocate_purchase", { assetIds, total: total.value, currency: currency.value, byValue: method.value === "value" });
    store.invalidate();
    toast(`Divided among ${shares.length} items.`, { kind: "success" });
    (after ?? (() => ctx.refresh()))();
  } catch (e) { toast(describe(e), { kind: "error" }); }
}

/** Make part of a holding an item of its own. */
export function splitAsset(a, ctx) {
  const quantity = textInput({ inputmode: "decimal", placeholder: `Less than ${fmt.quantity(a.quantity)}` });
  const name = textInput({ value: `${a.name} — part`, maxlength: 500 });
  const save = h("button", { class: "btn btn-primary" }, "Split off");
  const m = modal({
    title: "Split off part of this holding",
    subtitle: `${a.name} · ${fmt.quantity(a.quantity)} ${a.quantity_unit}`,
    size: "sm",
    body: h("div", { class: "stack" },
      h("p", {}, "The part becomes an item of its own — to sell, insure or store separately. Cost and value are divided in proportion; nothing is recorded as sold, and every total stays the same."),
      field("How many", quantity), field("Name of the new item", name)
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  save.addEventListener("click", () => busy(save, async () => {
    const id = await call("split_asset", { assetId: a.asset_id, quantity: quantity.value, name: name.value });
    store.invalidate();
    m.close();
    toast("Split off as a new item.", { kind: "success" });
    ctx.navigate("asset", { id });
  }));
}
