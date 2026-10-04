// Add / edit an asset, update its value, record a quantity change.
//
// Every field is validated again in the backend; what happens here is
// guidance — showing a grader's scale before entry, previewing melt value,
// naming a comic from its title and issue — not a control.

import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { call, describe } from "../lib/api.js";
import { h, mount, debounce } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as store from "../lib/store.js";
import * as fmt from "../lib/format.js";
import { modal, field, textInput, select, toggle, segmented, callout, busy, toast, confirmDialog } from "../ui/components.js";

// ------------------------------------------------------------ kinds

/** What someone can add, in the order they are most likely to reach for. */
const KINDS = [
  { id: "metal", label: "Precious metal", glyph: "metal", blurb: "Bullion, rounds and bullion coins — valued from spot" },
  { id: "crypto", label: "Cryptocurrency", glyph: "crypto", blurb: "Coins and tokens, priced by coin ID" },
  { id: "comic", type: "comic", label: "Comic book", glyph: "comic", blurb: "Raw or slabbed, with grade" },
  { id: "trading_card", type: "trading_card", label: "Sports card", glyph: "card", blurb: "Player, set, parallel, grade" },
  { id: "tcg_card", type: "tcg_card", label: "TCG card", glyph: "card", blurb: "Magic, Pokémon, Yu-Gi-Oh" },
  { id: "numismatic_coin", type: "numismatic_coin", label: "Collector coin", glyph: "coin", blurb: "Year, mint mark, variety, grade" },
  { id: "watch", type: "watch", label: "Watch", glyph: "watch", blurb: "Reference, serial, box & papers" },
  { id: "jewelry", type: "jewelry", label: "Jewelry", glyph: "gem", blurb: "Metal, stones, appraisal" },
  { id: "art", type: "art", label: "Art & prints", glyph: "art", blurb: "Artist, medium, edition" },
  { id: "memorabilia", type: "memorabilia", label: "Memorabilia", glyph: "signed", blurb: "Autographs and authenticated items" },
  { id: "sealed_product", type: "sealed_product", label: "Sealed product", glyph: "box", blurb: "Wax boxes, sealed sets" },
  { id: "video_game", type: "video_game", label: "Video game", glyph: "game", blurb: "Platform, region, completeness" },
  { id: "vinyl", type: "vinyl", label: "Vinyl record", glyph: "vinyl", blurb: "Pressing, matrix, condition" },
  { id: "instrument", type: "instrument", label: "Instrument", glyph: "music", blurb: "Maker, model, serial" },
  { id: "wine", type: "wine", label: "Wine & spirits", glyph: "wine", blurb: "Producer, vintage, bottle size" },
  { id: "firearm", type: "firearm", label: "Firearm", glyph: "target", blurb: "Make, model, caliber, serial" },
  { id: "ammunition", type: "ammunition", label: "Ammunition", glyph: "target", blurb: "Caliber, load, rounds held" },
  { id: "firearm_accessory", type: "firearm_accessory", label: "Optics & accessories", glyph: "target", blurb: "Scopes, suppressors, parts" },
  { id: "cash", type: "cash", label: "Cash & accounts", glyph: "cash", blurb: "Balances held for completeness" },
  { id: "generic", type: "generic", label: "Anything else", glyph: "item", blurb: "A general item with your own details" },
];

/**
 * Suggested fields for types without a validated schema. Stored as ordinary
 * attributes; anything left blank is simply absent.
 *
 * Jewelry uses `metal_type`, not `metal`: `metal` is how a holding says it
 * follows a spot price, and "18k gold" is not a spot instrument.
 */
const SUGGESTED = {
  watch: ["brand", "model", "reference", "serial_number", "year", "box_papers", "service_history"],
  jewelry: ["metal_type", "stones", "carat", "hallmark", "appraised_by"],
  art: ["artist", "title", "medium", "dimensions", "edition", "provenance"],
  memorabilia: ["signer", "item", "authentication", "cert_number"],
  sealed_product: ["set", "year", "seal_condition"],
  video_game: ["platform", "region", "completeness", "grader", "grade"],
  vinyl: ["artist", "title", "pressing", "matrix", "condition"],
  instrument: ["maker", "model", "serial_number", "year", "condition"],
  wine: ["producer", "vintage", "varietal", "bottle_size", "storage_conditions"],
  firearm: ["manufacturer", "model", "caliber", "serial_number", "action", "barrel_length", "finish", "year", "condition"],
  ammunition: ["manufacturer", "caliber", "grain", "bullet_type", "lot_number"],
  firearm_accessory: ["brand", "model", "serial_number", "fits"],
  cash: ["institution", "account_type", "last_four"],
  generic: ["brand", "model", "serial_number"],
};

/** Attribute keys that steer pricing rather than describe the item. */
const INTERNAL_ATTRS = ["metal", "coin_id"];

/**
 * Details a record has beyond its type's own fields — from a CSV import, or
 * kept on purpose through a type change. Listed apart from the type's
 * fields so they are never mistaken for part of it, and kept on save.
 */
function otherDetails(attrs, known, inputs) {
  const keys = Object.keys(attrs).filter((key) => !known.includes(key) && !INTERNAL_ATTRS.includes(key));
  if (!keys.length) return null;
  return h("div", { class: "other-details" },
    h("p", { class: "carry-over-head" }, "Other details"),
    h("p", { class: "field-hint" }, "Not part of this type's fields; kept with the record. Clear one to remove it."),
    h("div", { class: "form-grid" }, keys.map((key) => {
      const input = textInput({ value: attrs[key] });
      inputs.set(key, input);
      return field(fmt.fieldLabel(key), input);
    }))
  );
}

const COIN_PRESETS = new Set(["ase", "maple_silver", "age", "maple_gold", "krugerrand", "platinum_eagle", "britannia_silver", "buffalo", "palladium_maple"]);
const METAL_TYPE = { XAU: "gold_bullion", XAG: "silver_bullion", XPT: "platinum_bullion", XPD: "palladium_bullion" };

function metalTypeFor(metal, preset) {
  if (preset === "junk_90") return "junk_silver";
  if (COIN_PRESETS.has(preset)) return "sovereign_coin";
  return METAL_TYPE[metal] ?? "gold_bullion";
}

/** Which form an existing asset uses. */
function kindOf(asset, collectibleIds) {
  if (asset.attrs.metal) return "metal";
  if (asset.attrs.coin_id) return "crypto";
  if (collectibleIds.includes(asset.type_id)) return asset.type_id;
  return KINDS.find((k) => k.type === asset.type_id)?.id ?? "generic";
}

/** "1234.50 USD" → "1234.50", for pre-filling an amount field. */
function amountOf(display) {
  return display ? display.split(" ")[0] : "";
}

// ------------------------------------------------------------ add / edit

/** Open the add flow: choose a kind, then fill the form. */
export function openAddAsset({ onSaved, kind } = {}) {
  const m = modal({ title: "Add to your catalog", subtitle: "What are you adding?", size: "lg", body: h("div") });
  const body = m.dialog.querySelector(".modal-body");

  const showForm = (kindId) => {
    renderForm(m, body, { mode: "create", kindId, onSaved: (id) => { m.close(id); onSaved?.(id); } });
  };

  if (kind) return showForm(kind), m.done;

  mount(
    body,
    h("div", { class: "kind-grid" },
      KINDS.map((k) =>
        h("button", { class: "kind", onclick: () => showForm(k.id) },
          h("span", { class: "kind-icon" }, icon(k.glyph, { size: 22 })),
          h("strong", {}, k.label),
          h("span", {}, k.blurb)
        )
      )
    )
  );
  return m.done;
}

export async function openEditAsset(asset, { onSaved } = {}) {
  const collectibles = await store.collectibleTypes();
  const kindId = kindOf(asset, collectibles.map((c) => c.id));
  const m = modal({ title: `Edit ${asset.name}`, size: "lg", body: h("div") });
  renderForm(m, m.dialog.querySelector(".modal-body"), {
    mode: "edit",
    kindId,
    asset,
    onSaved: (id) => { m.close(id); onSaved?.(id); },
  });
  return m.done;
}

/**
 * The add/edit form for one kind. The kind decides the type and the fields;
 * there is no separate type control, because a type the fields were not
 * built for is how a watch ends up with comic-book fields. To change an
 * existing asset's type, the edit form goes back to the kind picker
 * ("Change type…") and comes back with the form for the new kind.
 *
 * `retypedFrom`: the type label the asset had, when this form is showing it
 * as a different kind than it is stored as. Details both kinds use carry
 * over; the rest are listed apart and dropped on save unless kept.
 */
async function renderForm(m, body, { mode, kindId, asset, onSaved, retypedFrom = null }) {
  const kind = KINDS.find((k) => k.id === kindId) ?? KINDS.at(-1);
  const [collectibles, graders, settings] = await Promise.all([
    store.collectibleTypes(),
    store.graders(),
    store.settings(),
  ]);
  const currency = settings.currency;
  const schema = collectibles.find((c) => c.id === kindId);
  const editing = mode === "edit";

  // The fields this kind's form is built from.
  const kindKeys = schema ? [...schema.required, ...schema.optional] : SUGGESTED[kind.type] ?? SUGGESTED.generic;
  // After a type change, only details the new kind also uses come into its
  // form. The others are held apart, shown, and kept only if asked.
  const attrs = { ...(asset?.attrs ?? {}) };
  const leftovers = {};
  if (retypedFrom) {
    for (const key of Object.keys(attrs)) {
      if (!kindKeys.includes(key) && !INTERNAL_ATTRS.includes(key)) {
        leftovers[key] = attrs[key];
        delete attrs[key];
      }
    }
  }
  let keepLeftovers = false;

  // Retitle for the chosen kind.
  const title = m.dialog.querySelector(".modal-title");
  const subtitle = m.dialog.querySelector(".modal-subtitle");
  title.textContent = editing ? `Edit ${asset.name}` : `Add ${kind.label.toLowerCase()}`;
  if (subtitle) subtitle.textContent = editing ? asset.type_label : kind.blurb;

  const error = h("p", { class: "form-error", role: "alert" });
  const sections = [];
  const collect = []; // functions that write into the form object

  // --- identity -------------------------------------------------------
  const name = textInput({ value: asset?.name ?? "", placeholder: "Name", maxlength: 500 });
  const namePreview = h("p", { class: "field-hint" });

  const quantity = textInput({ value: editing ? asset.quantity : "1", inputmode: "decimal", disabled: editing });
  const unitDefault = kindId === "crypto" ? (attrs.symbol || "BTC") : "item";
  const unit = textInput({ value: asset?.quantity_unit ?? unitDefault, maxlength: 40 });

  let identity;
  let pricingToggle = null;
  let followMarket = !editing;

  if (kindId === "metal") {
    identity = await metalFields(attrs, collect, quantity, (preset) => {
      if (!name.value || name.dataset.auto === "1") {
        name.value = preset.label;
        name.dataset.auto = "1";
      }
    });
    name.placeholder = "e.g. American Gold Eagles";
  } else if (kindId === "crypto") {
    identity = await cryptoFields(attrs, collect, unit, (coinName) => {
      if (!name.value || name.dataset.auto === "1") {
        name.value = coinName;
        name.dataset.auto = "1";
      }
    });
    name.placeholder = "e.g. Bitcoin — hardware wallet";
  } else if (schema) {
    identity = collectibleFields(schema, graders, attrs, collect, () => previewName());
    name.placeholder = "Leave blank to name it from the fields above";
  } else {
    identity = freeFields(kindKeys, attrs, collect);
  }
  name.addEventListener("input", () => (name.dataset.auto = "0"));

  function leftoverSection() {
    const keys = Object.keys(leftovers);
    if (!keys.length) return null;
    return h("div", { class: "carry-over" },
      h("p", { class: "carry-over-head" }, `From the ${retypedFrom.toLowerCase()} — not used by ${kind.label.toLowerCase()}`),
      h("dl", { class: "kv" }, keys.map((key) => [h("dt", {}, fmt.fieldLabel(key)), h("dd", {}, leftovers[key])])),
      toggle("Keep these as other details", false, (on) => (keepLeftovers = on), {
        hint: "Off: they are removed when you save. On: they are kept with the record, apart from this type's fields.",
      })
    );
  }

  const previewName = debounce(async () => {
    if (!schema) return;
    const form = buildForm();
    try {
      const derived = await call("validate_asset", { form: { ...form, name: null } });
      namePreview.textContent = `Will be listed as “${derived}”`;
      namePreview.classList.remove("is-error");
    } catch (e) {
      // A required field not yet filled in is the normal state of a form
      // being typed into — guidance, not an error.
      const message = describe(e);
      const incomplete = /required|missing|needs/i.test(message);
      namePreview.textContent = incomplete ? "Fill in the starred fields and the name will follow." : message;
      namePreview.classList.toggle("is-error", !incomplete);
    }
  }, 200);

  sections.push(
    h("section", { class: "form-section" },
      h("h3", {}, "Details"),
      identity.el,
      leftoverSection(),
      h("div", { class: "form-grid" },
        field("Name", name, { span: 2, hint: null })
      ),
      namePreview
    )
  );

  // --- ownership -------------------------------------------------------
  const acquiredDate = h("input", { type: "date", value: asset?.acquired_date ?? "", max: fmt.todayIso() });
  // Each amount keeps the currency it was recorded in. The base currency is
  // only what a new, empty amount starts in — changing it later must not
  // relabel a stored USD 1,000 as EUR 1,000 when the form is saved.
  const price = moneyInput(asset?.acquired_currency ?? currency, amountOf(asset?.acquired_display), { codes: [currency] });
  const partialCost = editing && asset.acquired_display && !asset.cost_complete;
  let costCoversHolding = false;
  const coversToggle = partialCost
    ? toggle("This is the total for everything held", false, (on) => (costCoversHolding = on),
        { hint: "For when the units added without a price cost nothing extra — a gift, or included in this figure." })
    : null;
  const from = textInput({ value: asset?.acquired_from ?? "", placeholder: "Dealer, show, auction, gift…" });
  const location = textInput({ value: asset?.storage_location ?? "", placeholder: kindId === "crypto" ? "Wallet or exchange" : "Safe, deposit box, shelf…" });

  sections.push(
    h("section", { class: "form-section" },
      h("h3", {}, "Ownership"),
      h("div", { class: "form-grid" },
        field(kindId === "metal" && attrs.preset === "junk_90" ? "Face value ($)" : "Quantity", quantity, {
          hint: editing ? "Change quantity with “Record change” so the history shows when." : kindId === "crypto" ? "Exact — up to 18 decimal places." : null,
        }),
        field("Unit", unit),
        field("Acquired on", acquiredDate),
        field("Total paid", price.el, {
          hint: partialCost
            ? "Covers only part of this holding — some was added without a price. Enter what everything held cost in total to complete it."
            : "For the whole position. Leave blank if unknown — never enter 0 for unknown.",
        }),
        coversToggle ? h("div", { class: "span-2" }, coversToggle) : null,
        field("Acquired from", from),
        field(kindId === "crypto" ? "Held at" : "Storage location", location)
      )
    )
  );

  // --- value -----------------------------------------------------------
  const currentValue = moneyInput(currency, "");
  const insured = moneyInput(asset?.insured_currency ?? currency, amountOf(asset?.insured_display), { codes: [currency] });
  const review = select(
    [["", "Never"], ["30", "Every month"], ["90", "Every 3 months"], ["180", "Every 6 months"], ["365", "Every year"]],
    asset?.review_every_days ? String(asset.review_every_days) : ""
  );
  if (review.value === "" && asset?.review_every_days) {
    review.append(h("option", { value: String(asset.review_every_days) }, `Every ${asset.review_every_days} days`));
    review.value = String(asset.review_every_days);
  }
  const statusSelect = editing && asset.status !== "sold"
    ? select([["active", "Held"], ["lost", "Lost"], ["retired", "Retired"]], asset.status)
    : null;
  // A change of status is dated: the day it was lost, or came back. Totals
  // before that date keep the item.
  const statusDate = h("input", { type: "date", value: fmt.todayIso(), max: fmt.todayIso() });
  const statusDateField = statusSelect
    ? field("Since", statusDate, { hint: "Totals before this date still include it." })
    : null;
  if (statusDateField) {
    statusDateField.hidden = true;
    statusSelect.addEventListener("change", () => (statusDateField.hidden = statusSelect.value === asset.status));
  }

  const marketKind = kindId === "metal" || kindId === "crypto";
  if (marketKind && !editing) {
    pricingToggle = toggle(
      kindId === "metal" ? "Value from the spot price" : "Value from the coin price",
      true,
      (on) => {
        followMarket = on;
        currentValueField.hidden = on;
      },
      { hint: "Refreshes when prices update. Entering a value by hand later turns this off." }
    );
  }
  const currentValueField = field("Current value", currentValue.el, { hint: "What it would sell for today. You can add a dated history later." });
  if (marketKind && !editing) currentValueField.hidden = true;

  sections.push(
    h("section", { class: "form-section" },
      h("h3", {}, "Value & insurance"),
      pricingToggle,
      h("div", { class: "form-grid" },
        editing ? null : currentValueField,
        field("Insured for", insured.el),
        field("Remind me to revalue", review, { hint: "Items due appear on the overview." }),
        statusSelect ? field("Status", statusSelect, { hint: "Sold is recorded with “Record change”." }) : null,
        statusDateField
      )
    )
  );

  // --- notes -----------------------------------------------------------
  const notes = h("textarea", { rows: 3, maxlength: 20000, placeholder: "Provenance, condition, anything worth remembering" });
  notes.value = asset?.notes ?? "";
  sections.push(h("section", { class: "form-section" }, h("h3", {}, "Notes"), notes));

  function buildForm() {
    const form = {
      type_id: kind.type ?? schema?.id ?? kindId,
      name: name.value.trim() || null,
      quantity: quantity.value.trim(),
      quantity_unit: unit.value.trim(),
      acquired_date: acquiredDate.value || null,
      acquired_price: price.value(),
      acquired_from: from.value.trim() || null,
      storage_location: location.value.trim() || null,
      notes: notes.value,
      insured_value: insured.value(),
      acquired_currency: price.currency(),
      insured_currency: insured.currency(),
      currency,
      attrs: {},
      review_every_days: review.value ? Number(review.value) : null,
    };
    for (const fn of collect) fn(form);
    if (keepLeftovers) form.attrs = { ...leftovers, ...form.attrs };
    // An asset whose type has no form of its own opens in the general form;
    // saving it there must not quietly turn it into a generic item. Only
    // "Change type…" changes a type.
    if (editing && !retypedFrom && !schema && !marketKind && kind.type !== asset.type_id) form.type_id = asset.type_id;
    if (!editing) {
      form.pricing = marketKind && followMarket ? "market" : "manual";
      form.current_value = form.pricing === "manual" ? currentValue.value() : null;
    } else {
      form.status = statusSelect ? statusSelect.value : asset.status;
      if (statusSelect && statusSelect.value !== asset.status) form.status_date = statusDate.value || null;
      form.cost_covers_holding = costCoversHolding;
      delete form.quantity;
    }
    return form;
  }

  if (schema) {
    for (const input of identity.el.querySelectorAll("input,select")) {
      input.addEventListener("input", previewName);
      input.addEventListener("change", previewName);
    }
    previewName();
  }

  const save = h("button", { class: "btn btn-primary", type: "submit" }, editing ? "Save changes" : "Add to catalog");
  // Metal and crypto are not offered either way: changing them means
  // changing how the asset is priced, not just which fields it has.
  const changeType = editing && !marketKind && asset.status !== "sold"
    ? h("button", { class: "btn btn-ghost", type: "button", onclick: () => {
        const form = buildForm();
        const money = (amount, code) => (amount ? `${amount} ${code}` : null);
        // What has been typed so far comes along to the new kind's form.
        const draft = {
          ...asset,
          name: form.name ?? asset.name,
          quantity_unit: form.quantity_unit,
          acquired_date: form.acquired_date,
          acquired_from: form.acquired_from,
          storage_location: form.storage_location,
          notes: form.notes,
          review_every_days: form.review_every_days,
          // Set-aside details travel too, so changing again — or back —
          // can still use them; each form decides again what applies.
          attrs: { ...leftovers, ...form.attrs },
          acquired_display: money(form.acquired_price, form.acquired_currency),
          acquired_currency: form.acquired_price ? form.acquired_currency : asset.acquired_currency,
          insured_display: money(form.insured_value, form.insured_currency),
          insured_currency: form.insured_value ? form.insured_currency : asset.insured_currency,
        };
        chooseNewKind(m, body, { draft, currentKind: kindId, onSaved, retypedFrom: retypedFrom ?? asset.type_label, currentRetyped: Boolean(retypedFrom) });
      } }, "Change type…")
    : null;
  const back = editing
    ? h("button", { class: "btn btn-ghost", type: "button", onclick: () => m.close() }, "Cancel")
    : h("button", { class: "btn btn-ghost", type: "button", onclick: () => openAddAssetInPlace(m, body, onSaved) }, icon("back", { size: 16 }), "Other type");

  const formEl = h(
    "form",
    {
      class: "asset-form",
      onsubmit: async (event) => {
        event.preventDefault();
        error.textContent = "";
        const form = buildForm();
        await busy(save, async () => {
          try {
            if (editing) {
              await call("update_asset", { assetId: asset.asset_id, form });
              store.invalidate();
              toast("Saved.", { kind: "success" });
              onSaved(asset.asset_id);
            } else {
              const id = await call("create_asset", { form });
              store.invalidate();
              toast(`Added ${form.name ?? "to your catalog"}.`, { kind: "success" });
              onSaved(id);
            }
          } catch (e) {
            error.textContent = describe(e);
            error.scrollIntoView({ block: "nearest" });
          }
        });
      },
    },
    sections,
    h("div", { class: "form-actions" }, error, h("div", { class: "btn-row" }, changeType, back, save))
  );
  if (retypedFrom) {
    formEl.prepend(callout("info", `Changing from ${retypedFrom.toLowerCase()} to ${kind.label.toLowerCase()}. Details both use carry over; nothing is saved until you choose Save changes.`));
  }
  mount(body, formEl);
  requestAnimationFrame(() => formEl.querySelector("input:not([disabled]),select")?.focus());
}

/** Pick the kind an existing asset should become, then show its form. */
function chooseNewKind(m, body, { draft, currentKind, onSaved, retypedFrom, currentRetyped }) {
  const backToForm = (kindId, changed) =>
    renderForm(m, body, { mode: "edit", kindId, asset: draft, onSaved, retypedFrom: changed ? retypedFrom : null });
  mount(
    body,
    h("p", { class: "lede" }, "What is this, really? Details you entered carry over to the new form."),
    h("div", { class: "kind-grid" },
      KINDS.filter((k) => !["metal", "crypto", currentKind].includes(k.id)).map((k) =>
        h("button", { class: "kind", onclick: () => backToForm(k.id, true) },
          h("span", { class: "kind-icon" }, icon(k.glyph, { size: 22 })),
          h("strong", {}, k.label),
          h("span", {}, k.blurb)
        )
      )
    ),
    h("div", { class: "form-actions" }, h("div", { class: "btn-row" },
      h("button", { class: "btn btn-ghost", type: "button", onclick: () => backToForm(currentKind, currentRetyped) },
        icon("back", { size: 16 }), "Back to the form")
    ))
  );
}

function openAddAssetInPlace(m, body, onSaved) {
  m.dialog.querySelector(".modal-title").textContent = "Add to your catalog";
  const subtitle = m.dialog.querySelector(".modal-subtitle");
  if (subtitle) subtitle.textContent = "What are you adding?";
  mount(
    body,
    h("div", { class: "kind-grid" },
      KINDS.map((k) =>
        h("button", { class: "kind", onclick: () => renderForm(m, body, { mode: "create", kindId: k.id, onSaved }) },
          h("span", { class: "kind-icon" }, icon(k.glyph, { size: 22 })),
          h("strong", {}, k.label),
          h("span", {}, k.blurb)
        )
      )
    )
  );
}

/**
 * An amount input with its currency alongside. Given `codes`, the currency
 * can be switched among those and the one it starts in — choosing one is an
 * explicit statement about what the number means, never a conversion.
 */
function moneyInput(currency, value, { codes } = {}) {
  const input = textInput({ value: value ?? "", inputmode: "decimal", placeholder: "0.00", class: "input-money" });
  let code = currency;
  let affix;
  const options = [...new Set([currency, ...(codes ?? []), ...fmt.COMMON_CURRENCIES])];
  if (codes) {
    affix = select(options.map((c) => [c, c]), currency, { class: "affix affix-select", "aria-label": "Currency" });
    affix.addEventListener("change", () => (code = affix.value));
  } else {
    affix = h("span", { class: "affix" }, currency);
  }
  const el = h("div", { class: "input-affix" }, affix, input);
  return { el, input, value: () => input.value.trim() || null, currency: () => code };
}

// ------------------------------------------------------------ kind fields

async function metalFields(attrs, collect, quantityInput, onPreset) {
  const presets = await store.presets();
  const preset = select([["", "Custom…"], ...presets.map((p) => [p.id, p.label])], attrs.preset ?? "");
  const metal = select([["XAU", "Gold"], ["XAG", "Silver"], ["XPT", "Platinum"], ["XPD", "Palladium"]], attrs.metal ?? "XAU");
  const weight = textInput({ value: attrs.weight_per_item ?? "", inputmode: "decimal", placeholder: "1" });
  const unit = select([["troy_oz", "troy oz"], ["gram", "grams"], ["pennyweight", "pennyweight"], ["ounce", "oz (avoirdupois)"]], attrs.weight_unit ?? "troy_oz");
  const basis = select([["gross", "Gross — whole item, alloy included"], ["fine", "Fine — metal content only"]], attrs.weight_basis ?? "gross");
  const purity = textInput({ value: attrs.purity ?? "0.999", inputmode: "decimal" });
  const premium = textInput({ value: attrs.premium_pct ?? "", inputmode: "decimal", placeholder: "0" });
  const preview = h("div", { class: "metal-preview", "aria-live": "polite" });

  const apply = () => {
    const p = presets.find((x) => x.id === preset.value);
    if (!p) return;
    metal.value = p.metal;
    weight.value = p.weight;
    unit.value = p.unit;
    basis.value = p.basis;
    purity.value = p.purity;
    onPreset(p);
    refresh();
  };
  preset.addEventListener("change", apply);

  const refresh = debounce(async () => {
    if (!weight.value.trim()) {
      mount(preview, h("span", { class: "muted" }, "Enter a weight to see the metal content and value."));
      return;
    }
    try {
      const r = await call("value_metal_holding", {
        request: {
          metal: metal.value,
          quantity: quantityInput.value.trim() || "1",
          weight_per_item: weight.value,
          unit: unit.value,
          basis: basis.value,
          purity: purity.value,
          premium_pct: premium.value || null,
        },
      });
      mount(
        preview,
        stat("Fine metal", `${fmt.quantity(r.fine_troy_oz)} troy oz`),
        stat("Melt", fmt.money(r.melt)),
        stat("With premium", fmt.money(r.market)),
        h("span", { class: r.needs_caveat ? "metal-preview-note warn" : "metal-preview-note" },
          `at ${fmt.unitPrice(r.spot_used, r.currency)}/oz spot${r.needs_caveat ? ` — ${r.freshness}` : ""}`)
      );
    } catch (e) {
      mount(preview, h("span", { class: "muted" }, describe(e)));
    }
  }, 250);
  for (const el of [metal, weight, unit, basis, purity, premium, quantityInput]) {
    el.addEventListener("input", refresh);
    el.addEventListener("change", refresh);
  }
  setTimeout(refresh, 50);

  collect.push((form) => {
    form.attrs.metal = metal.value;
    form.attrs.weight_per_item = weight.value.trim();
    form.attrs.weight_unit = unit.value;
    form.attrs.weight_basis = basis.value;
    form.attrs.purity = purity.value.trim();
    if (premium.value.trim()) form.attrs.premium_pct = premium.value.trim();
    if (preset.value) form.attrs.preset = preset.value;
    form.type_id = metalTypeFor(metal.value, preset.value);
  });

  const el = h("div", { class: "form-grid" },
    field("Product", preset, { span: 2, hint: "Picking a product fills in weight and purity — the figures most often entered wrong." }),
    field("Metal", metal),
    field("Weight of one item", h("div", { class: "input-pair" }, weight, unit)),
    field("Weight is", basis, { hint: "An American Gold Eagle weighs 1.0909 oz gross at .9167 — about 1 oz of gold. Getting this wrong misprices by ~8%." }),
    field("Purity", purity, { hint: "As a fraction: .999, .9167, .900" }),
    field("Premium over melt", h("div", { class: "input-affix" }, premium, h("span", { class: "affix" }, "%")), { hint: "What a dealer pays above metal value. Blank for melt." }),
    h("div", { class: "span-2" }, preview)
  );
  return { el };
}

function stat(label, value) {
  return h("div", { class: "mini-stat" }, h("span", {}, label), h("strong", {}, value));
}

async function cryptoFields(attrs, collect, unitInput, onCoin) {
  const coins = await store.coins();
  const known = coins.find((c) => c.coin_id === attrs.coin_id);
  const coin = select([...coins.map((c) => [c.coin_id, `${c.name} (${c.symbol})`]), ["__other", "Other coin or token…"]],
    attrs.coin_id ? (known ? attrs.coin_id : "__other") : "bitcoin");
  const coinId = textInput({ value: attrs.coin_id ?? "bitcoin", placeholder: "e.g. shiba-inu" });
  const symbol = textInput({ value: attrs.symbol ?? "BTC", placeholder: "e.g. SHIB", maxlength: 12 });
  const custody = select([["self_custody", "My own wallet"], ["exchange", "An exchange"], ["locked", "Staked or locked"]], attrs.custody ?? "self_custody");
  const watch = textInput({ value: attrs.watch_address ?? "", placeholder: "Optional — a public address, never a seed phrase", class: "input-mono" });
  const watchChain = select([["bitcoin", "Bitcoin"], ["ethereum", "Ethereum"]], attrs.watch_chain ?? "bitcoin");

  const custom = h("div", { class: "form-grid", hidden: coin.value !== "__other" },
    field("Coin ID", coinId, { hint: "CoinGecko's ID, from the coin's page URL. Symbols are ambiguous — several tokens share one." }),
    field("Symbol", symbol)
  );
  coin.addEventListener("change", () => {
    const c = coins.find((x) => x.coin_id === coin.value);
    custom.hidden = Boolean(c);
    if (c) {
      coinId.value = c.coin_id;
      symbol.value = c.symbol;
      onCoin(c.name);
      // Follow the coin unless the unit was typed by hand.
      if (unitInput.value === "coin" || coins.some((x) => x.symbol === unitInput.value)) unitInput.value = c.symbol;
    }
  });
  if (!attrs.coin_id) onCoin("Bitcoin");

  collect.push((form) => {
    form.attrs.coin_id = coinId.value.trim().toLowerCase();
    form.attrs.symbol = symbol.value.trim().toUpperCase();
    form.attrs.custody = custody.value;
    if (watch.value.trim()) {
      form.attrs.watch_address = watch.value.trim();
      form.attrs.watch_chain = watchChain.value;
    }
    form.type_id = "crypto";
  });

  const el = h("div", {},
    h("div", { class: "form-grid" }, field("Coin", coin), field("Held in", custody)),
    custom,
    h("details", { class: "disclosure", open: Boolean(attrs.watch_address) },
      h("summary", {}, "Watch-only address (optional)"),
      h("div", { class: "form-grid" },
        field("Address", watch, { span: 1, hint: "Lets you check the on-chain balance later. Anything that looks like a seed phrase or private key is refused." }),
        field("Chain", watchChain)
      )
    )
  );
  return { el };
}

function collectibleFields(schema, graders, attrs, collect, onChange) {
  const inputs = new Map();
  const make = (key) => {
    if (key === "grader") {
      const s = select(graders.map((g) => [g.id, g.label]), attrs.grader ?? "raw");
      s.addEventListener("change", () => constrain());
      return s;
    }
    if (["signed", "key_issue", "autographed", "rookie"].includes(key)) {
      return select([["", "—"], ["yes", "Yes"], ["no", "No"]], attrs[key] ?? "");
    }
    return textInput({ value: attrs[key] ?? "", inputmode: ["year", "print_run", "grade"].includes(key) ? "decimal" : null });
  };
  const constrain = () => {
    const grader = graders.find((g) => g.id === inputs.get("grader")?.value);
    const grade = inputs.get("grade");
    if (!grade) return;
    if (!grader?.numeric) {
      grade.value = "";
      grade.disabled = true;
      grade.placeholder = "Ungraded";
    } else {
      grade.disabled = false;
      grade.placeholder = `${grader.min}–${grader.max}`;
    }
    onChange();
  };

  const required = schema.required.map((key) => {
    const input = make(key);
    inputs.set(key, input);
    return field(fmt.fieldLabel(key), input, { required: true });
  });
  const optional = schema.optional.map((key) => {
    const input = make(key);
    inputs.set(key, input);
    return field(fmt.fieldLabel(key), input);
  });
  const extras = otherDetails(attrs, [...schema.required, ...schema.optional], inputs);
  setTimeout(constrain);

  const scan = h("button", {
    type: "button",
    class: "btn btn-secondary btn-sm",
    onclick: async () => {
      const path = await openDialog({ filters: [{ name: "Photo of a label", extensions: ["jpg", "jpeg", "png", "webp"] }] });
      if (!path) return;
      await busy(scan, async () => {
        const result = await call("scan_slab_label", { path });
        // Suggested, never applied silently beyond the form: a barcode is a
        // claim printed on a label, not proof of anything.
        if (result.cert_number && inputs.get("cert_number")) inputs.get("cert_number").value = result.cert_number;
        if (result.likely_grader && inputs.get("grader")) {
          inputs.get("grader").value = result.likely_grader;
          constrain();
        }
        toast(
          result.cert_number
            ? `Read certificate ${result.cert_number}${result.likely_grader ? ` (${result.likely_grader.toUpperCase()})` : ""}. Check it against the slab.`
            : `Read “${result.text}” but could not find a certificate number.`,
          { kind: result.cert_number ? "success" : "warning" }
        );
      }, "Reading…");
    },
  }, icon("scan", { size: 16 }), "Scan slab label…");

  collect.push((form) => {
    for (const [key, input] of inputs) {
      if (input.disabled) continue;
      const v = input.value.trim();
      if (v) form.attrs[key] = v;
    }
    form.type_id = schema.id;
  });

  return {
    el: h("div", {},
      inputs.has("cert_number") ? h("div", { class: "scan-row" }, scan, h("span", { class: "field-hint" }, "Reads the barcode from a photo of the label. It tells you which certificate to look up — it does not verify the item.")) : null,
      h("div", { class: "form-grid" }, required, optional),
      extras
    ),
  };
}

function freeFields(keys, attrs, collect) {
  const inputs = new Map();
  const fields = keys.map((key) => {
    const input = textInput({ value: attrs[key] ?? "" });
    inputs.set(key, input);
    return field(fmt.fieldLabel(key), input);
  });
  const extras = otherDetails(attrs, keys, inputs);
  collect.push((form) => {
    for (const [key, input] of inputs) {
      const v = input.value.trim();
      if (v) form.attrs[key] = v;
    }
  });
  return { el: h("div", {}, h("div", { class: "form-grid" }, fields), extras) };
}

// ------------------------------------------------------------ update value

export async function openUpdateValue(asset, { onSaved } = {}) {
  const settings = await store.settings();
  const amount = moneyInput(asset.current_currency ?? settings.currency, amountOf(asset.current_display));
  const asof = h("input", { type: "date", value: fmt.todayIso(), max: fmt.todayIso() });
  const basis = select([["estimated_resale", "Estimated resale"], ["replacement", "Replacement cost"], ["insured", "Insured value"], ["melt", "Melt value"]], "estimated_resale");
  const source = select([["manual", "My own estimate or research"], ["appraisal", "A professional appraisal"]], "manual");
  const note = textInput({ placeholder: "e.g. three recent sold listings, CGC 9.8", maxlength: 500 });
  const error = h("p", { class: "form-error", role: "alert" });
  amount.input.setAttribute("autofocus", "");

  const save = h("button", { class: "btn btn-primary", type: "submit", form: "value-form" }, "Save value");
  const m = modal({
    title: "Update value",
    subtitle: asset.name,
    size: "sm",
    body: h("form", {
      id: "value-form",
      class: "stack",
      onsubmit: async (event) => {
        event.preventDefault();
        error.textContent = "";
        if (!amount.value()) {
          error.textContent = "Enter a value.";
          return;
        }
        await busy(save, async () => {
          const [result] = await call("set_prices", {
            entries: [{ asset_id: asset.asset_id, amount: amount.value(), currency: asset.current_currency ?? settings.currency, asof: asof.value, basis: basis.value, provenance: source.value, note: note.value.trim() || null }],
          });
          if (!result.ok) {
            error.textContent = describe({ message: result.error ?? "could not save that value" });
            return;
          }
          store.invalidate();
          toast("Value recorded.", { kind: "success" });
          m.close(true);
          onSaved?.();
        });
      },
    },
      asset.pricing === "market" ? callout("info", "This asset follows the market. Entering a value by hand switches it to manual so a later price refresh cannot overwrite your figure. You can switch back on its page.") : null,
      field("Value of the whole holding", amount.el, { hint: `Currently held: ${fmt.quantity(asset.quantity)} ${asset.quantity_unit}` }),
      h("div", { class: "form-grid" }, field("As of", asof), field("What it measures", basis)),
      field("Source", source),
      field("Note", note),
      error
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  return m.done;
}

// ------------------------------------------------------------ record change

export async function openRecordChange(asset, { onSaved } = {}) {
  const settings = await store.settings();
  const currency = asset.acquired_currency ?? settings.currency;
  let kind = "add";
  const qty = textInput({ inputmode: "decimal", autofocus: true });
  const when = h("input", { type: "date", value: fmt.todayIso(), max: fmt.todayIso() });
  const price = moneyInput(currency, "", { codes: [settings.currency] });
  const note = textInput({ maxlength: 500, placeholder: "Optional" });
  const error = h("p", { class: "form-error", role: "alert" });
  const qtyLabel = h("label", { for: "chg-qty" }, "How many");
  qty.id = "chg-qty";
  const qtyField = h("div", { class: "field" }, qtyLabel, qty);
  const priceLabel = h("label", {}, "Price paid (total)");
  const priceField = h("div", { class: "field" }, priceLabel, price.el);
  const explain = h("p", { class: "field-hint" });

  const held = fmt.quantity(asset.quantity);
  const update = () => {
    qtyField.hidden = kind === "dispose";
    priceField.hidden = kind === "correct";
    qtyLabel.textContent = { add: "How many more", remove: "How many sold", correct: "The correct quantity" }[kind] ?? "";
    priceLabel.textContent = kind === "add" ? "Price paid (total)" : "Sale price (total)";
    explain.textContent = {
      add: "Adds to the holding and, if you give a price in the cost's currency, to its cost. Without one, the recorded cost no longer covers the whole holding and no gain is shown until you enter the total paid.",
      remove: "Reduces the holding; its cost is reduced in proportion.",
      dispose: `Records everything held on that date as sold (${held} ${asset.quantity_unit} today). The asset keeps its history and stops counting toward today's total.`,
      correct: "Sets the quantity held on that date, without recording a trade — for when the number was simply wrong.",
    }[kind];
  };

  const seg = segmented([["add", "Bought more"], ["remove", "Sold some"], ["dispose", "Sold all"], ["correct", "Fix count"]], kind, (v) => {
    kind = v;
    update();
  });
  update();

  const save = h("button", { class: "btn btn-primary", type: "submit", form: "change-form" }, "Record");
  const m = modal({
    title: "Record a change",
    subtitle: `${asset.name} · ${held} ${asset.quantity_unit} held`,
    size: "sm",
    body: h("form", {
      id: "change-form",
      class: "stack",
      onsubmit: async (event) => {
        event.preventDefault();
        error.textContent = "";
        if (kind === "dispose") {
          const ok = await confirmDialog({ title: "Record as sold?", message: `Everything held on ${fmt.date(when.value)} will be recorded as sold that day.`, confirmLabel: "Record sale" });
          if (!ok) return;
        }
        await busy(save, async () => {
          try {
            await call("change_quantity", {
              change: { asset_id: asset.asset_id, kind, quantity: qty.value.trim() || null, effective_date: when.value, amount: kind === "correct" ? null : price.value(), currency: price.currency(), note: note.value.trim() || null },
            });
            store.invalidate();
            toast("Change recorded.", { kind: "success" });
            m.close(true);
            onSaved?.();
          } catch (e) {
            error.textContent = describe(e);
          }
        });
      },
    },
      seg.el,
      explain,
      qtyField,
      h("div", { class: "form-grid" }, field("Date", when), priceField),
      field("Note", note),
      error
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  return m.done;
}

export { KINDS };
