// One asset: photos, value, details, and its history.

import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import { call, describe } from "../lib/api.js";
import { mediaUrl } from "../lib/media.js";
import { h, mount } from "../lib/dom.js";
import { icon, typeIcon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { busy, toast, confirmDialog, menuButton, sourceBadge, statusBadge, toggle, callout, modal, field, select, tagsInput } from "../ui/components.js";
import { valueChart } from "../ui/chart.js";
import { openEditAsset, openUpdateValue, openRecordChange } from "./asset-forms.js";

/** Attribute keys that describe how to price an asset, not the item itself. */
const INTERNAL = new Set(["preset", "coin_id", "symbol", "chain", "contract", "watch_address", "watch_chain", "custody", "metal", "weight_per_item", "weight_unit", "weight_basis", "purity", "premium_pct"]);

export async function renderAsset(root, params, ctx) {
  let detail;
  try {
    detail = await call("get_asset", { assetId: params.id });
  } catch (error) {
    mount(root, h("div", { class: "view-error" }, h("h2", {}, "Asset not found"), h("p", {}, describe(error)), h("button", { class: "btn btn-secondary", onclick: () => ctx.navigate("holdings") }, "Back to holdings")));
    return;
  }
  const a = detail.asset;
  const reload = () => ctx.refresh();

  const actions = h("div", { class: "page-actions" },
    a.status === "active" || a.status === "sold"
      ? h("button", { class: "btn btn-secondary", onclick: () => openRecordChange(a, { onSaved: reload }) }, icon("swap", { size: 16 }), "Record change")
      : null,
    h("button", { class: "btn btn-secondary", onclick: () => openEditAsset(a, { onSaved: reload }) }, icon("edit", { size: 16 }), "Edit"),
    a.status === "active" ? h("button", { class: "btn btn-primary", onclick: () => openUpdateValue(a, { onSaved: reload }) }, "Update value") : null,
    menuButton(h("button", { class: "btn btn-ghost", "aria-label": "More" }, icon("more")), [
      { label: "Add photos…", icon: "image", onSelect: () => addPhotos(a, reload) },
      "divider",
      { label: "Edit tags…", icon: "tag", onSelect: () => editTags(a, reload) },
      { label: "Duplicate", icon: "copy", onSelect: () => duplicateAsset(a, ctx) },
      { label: "Edit history…", icon: "clock", onSelect: () => editHistory(a, reload) },
      "divider",
      { label: "Move to trash…", icon: "trash", danger: true, onSelect: () => deleteAsset(a, ctx) },
    ])
  );

  mount(
    root,
    h("button", { class: "link-back", onclick: () => ctx.navigate("holdings") }, icon("back", { size: 16 }), "Holdings"),
    h("header", { class: "page-head asset-head" },
      h("div", { class: "asset-title" },
        h("span", { class: "asset-title-glyph" }, icon(typeIcon(a.type_id, a.category), { size: 22 })),
        h("div", {},
          h("h1", {}, a.name, statusBadge(a.status)),
          h("p", { class: "page-sub" }, [a.type_label, fmt.categoryLabel(a.category), a.storage_location].filter(Boolean).join(" · ")),
          a.tags?.length
            ? h("button", { class: "tag-list tag-list-button", title: "Edit tags", onclick: () => editTags(a, reload) }, a.tags.map((t) => h("span", { class: "tag-chip" }, t)))
            : null
        )
      ),
      actions
    ),
    h("div", { class: "asset-layout" },
      h("div", { class: "asset-main" }, gallery(a, detail.photos, reload), documentsCard(a, detail.photos, reload), detailsCard(a, detail), notesCard(a)),
      h("div", { class: "asset-side" }, valueCard(a, detail, reload), historyCard(a, detail, reload), custodyCard(a, detail, reload), careCard(a, detail, reload), eventsCard(detail))
    )
  );
}

// ------------------------------------------------------------ photos

async function addPhotos(a, reload) {
  const selected = await openDialog({ multiple: true, filters: [{ name: "Photos", extensions: ["jpg", "jpeg", "png", "webp"] }] });
  if (!selected) return;
  const paths = Array.isArray(selected) ? selected : [selected];
  const progress = toast(`Encrypting ${paths.length} file${paths.length === 1 ? "" : "s"}…`, { timeout: 0 });
  let added = 0;
  let deduped = 0;
  for (const path of paths) {
    try {
      const result = await call("import_photo", { assetId: a.asset_id, path });
      added += 1;
      if (result.deduplicated) deduped += 1;
    } catch (e) {
      toast(`${path.split(/[\\/]/).pop()}: ${describe(e)}`, { kind: "error" });
    }
  }
  progress();
  if (added) {
    toast(deduped ? `Added ${added}; ${deduped} ${deduped === 1 ? "was" : "were"} already in the vault and reused.` : `Added ${added} photo${added === 1 ? "" : "s"}.`, { kind: "success" });
    store.invalidate();
    reload();
  }
}

function gallery(a, photos, reload) {
  const images = photos.filter((p) => p.doc_kind === "photo" && p.media_type.startsWith("image/"));
  if (!images.length) {
    return h("section", { class: "card gallery-empty" },
      h("button", { class: "drop-zone", onclick: () => addPhotos(a, reload) },
        icon("camera", { size: 28 }),
        h("strong", {}, "Add photos"),
        h("span", {}, "JPEG, PNG or WebP. Encrypted before they touch the disk. Receipts and certificates go under Documents.")
      )
    );
  }

  let selected = images[0] ?? null;
  const main = h("div", { class: "gallery-main" });
  const strip = h("div", { class: "gallery-strip" });

  const drawMain = () => {
    if (!selected) return mount(main, h("div", { class: "gallery-placeholder" }, icon("image", { size: 32 })));
    const img = h("img", { src: mediaUrl(selected.object_id, 1024), alt: `Photo of ${a.name}` });
    mount(main,
      h("button", { class: "gallery-view", onclick: () => lightbox(images, images.indexOf(selected), a.name), "aria-label": "View larger" }, img),
      h("div", { class: "gallery-actions" },
        selected.is_primary
          ? h("span", { class: "badge badge-primary" }, icon("star", { size: 12 }), "Cover photo")
          : h("button", { class: "btn btn-overlay btn-sm", onclick: async () => {
              try {
                await call("set_primary_photo", { assetId: a.asset_id, objectId: selected.object_id });
                store.invalidate();
                reload();
              } catch (e) { toast(describe(e), { kind: "error" }); }
            } }, icon("star", { size: 14 }), "Make cover"),
        h("button", { class: "btn btn-overlay btn-sm", "aria-label": "Save a copy of this photo", title: "Save a copy…", onclick: () => saveAttachmentCopy(a, selected) }, icon("download", { size: 14 })),
        h("button", { class: "btn btn-overlay btn-sm", "aria-label": "Remove photo", onclick: () => removePhoto(a, selected, reload) }, icon("trash", { size: 14 }))
      )
    );
  };

  const drawStrip = () => {
    mount(strip,
      images.map((p) =>
        h("button", { class: p === selected ? "strip-thumb active" : "strip-thumb", onclick: () => { selected = p; drawMain(); drawStrip(); }, "aria-label": "Show photo" },
          h("img", { src: mediaUrl(p.object_id, 256), alt: "", loading: "lazy" })
        )
      ),
      h("button", { class: "strip-add", onclick: () => addPhotos(a, reload), "aria-label": "Add photos" }, icon("plus"))
    );
  };

  drawMain();
  drawStrip();
  return h("section", { class: "card gallery" }, main, strip);
}

const EXTENSIONS = { "application/pdf": "pdf", "image/jpeg": "jpg", "image/png": "png", "image/webp": "webp", "image/heic": "heic" };

/**
 * Save a decrypted copy of an attachment where the owner chooses. Opening it
 * in another app means a plaintext file on disk, so that is said first.
 */
async function saveAttachmentCopy(a, attachment) {
  const ok = await confirmDialog({
    title: "Save an unencrypted copy?",
    message: [
      "The copy is an ordinary file, readable by anything that can open the folder you choose — outside the vault's protection.",
      "Delete it when you are done with it. The original stays encrypted in the vault.",
    ],
    confirmLabel: "Choose where…",
  });
  if (!ok) return;
  const ext = EXTENSIONS[attachment.media_type] ?? "bin";
  const base = a.name.replace(/[^\w\- ]+/g, "").trim().slice(0, 60) || "attachment";
  const path = await saveDialog({ defaultPath: `${base}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
  if (!path) return;
  try {
    await call("export_attachment", { assetId: a.asset_id, objectId: attachment.object_id, path });
    toast("Copy saved. Delete it when you no longer need it.", { kind: "success" });
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

// ------------------------------------------------------------ documents

/**
 * Receipts, appraisals, certificates, warranties: evidence, listed by what
 * it is and when, so it can be found and handed over when it matters.
 */
function documentsCard(a, attachments, reload) {
  const docs = attachments.filter((p) => p.doc_kind !== "photo" || !p.media_type.startsWith("image/"));
  const add = h("button", { class: "btn btn-secondary btn-sm", onclick: () => addDocument(a, reload) }, icon("plus", { size: 14 }), "Add document…");
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, "Documents"), add),
    docs.length
      ? h("ul", { class: "doc-list" }, docs.map((d) => h("li", {},
          h("span", { class: "doc-icon" }, icon(d.media_type === "application/pdf" ? "reports" : "image", { size: 18 })),
          h("div", { class: "doc-text" },
            h("strong", {}, d.title || fmt.DOC_KIND_LABELS[d.doc_kind] || "Document"),
            h("span", { class: "muted small" }, [fmt.DOC_KIND_LABELS[d.doc_kind] ?? d.doc_kind, d.doc_date ? fmt.date(d.doc_date) : null, d.media_type === "application/pdf" ? "PDF" : "Image"].filter(Boolean).join(" · ")),
            d.note ? h("span", { class: "doc-note" }, d.note) : null
          ),
          h("div", { class: "doc-actions" },
            h("button", { class: "icon-btn", title: "Save a copy…", "aria-label": `Save a copy of ${d.title ?? "this document"}`, onclick: () => saveAttachmentCopy(a, d) }, icon("download", { size: 16 })),
            h("button", { class: "icon-btn", title: "Edit details", "aria-label": `Edit details of ${d.title ?? "this document"}`, onclick: () => documentDetails(a, d, reload) }, icon("edit", { size: 16 })),
            h("button", { class: "icon-btn", title: "Remove", "aria-label": `Remove ${d.title ?? "this document"}`, onclick: () => removePhoto(a, d, reload) }, icon("trash", { size: 16 }))
          )
        )))
      : h("p", { class: "muted small" }, "No receipts, appraisals or certificates yet. PDFs and photos of paperwork are encrypted like everything else.")
  );
}

/** The details form for a document: what it is, its title, date and note. */
function documentForm(initial) {
  const kind = select(fmt.DOC_KINDS.map((k) => [k, fmt.DOC_KIND_LABELS[k]]), initial.doc_kind ?? "receipt");
  const title = h("input", { type: "text", maxlength: 200, value: initial.title ?? "", placeholder: "e.g. Jeweller's invoice" });
  const date = h("input", { type: "date", value: initial.doc_date ?? "", max: fmt.todayIso() });
  const note = h("input", { type: "text", maxlength: 2000, value: initial.note ?? "" });
  return {
    el: h("div", { class: "form-grid" },
      field("What it is", kind),
      field("Date on the document", date),
      field("Title", title, { span: 2, hint: initial.file ? "Leave blank to use the file name." : null }),
      field("Note", note, { span: 2 })
    ),
    value: () => ({ kind: kind.value, title: title.value.trim() || null, date: date.value || null, note: note.value.trim() }),
  };
}

async function addDocument(a, reload) {
  const path = await openDialog({ filters: [{ name: "Documents", extensions: ["pdf", "jpg", "jpeg", "png", "webp"] }] });
  if (!path) return;
  const file = String(path).split(/[\\/]/).pop();
  const form = documentForm({ file });
  const save = h("button", { class: "btn btn-primary" }, "Add document");
  const m = modal({
    title: "Add a document",
    subtitle: file,
    size: "md",
    body: form.el,
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  save.addEventListener("click", () => busy(save, async () => {
    await call("import_photo", { assetId: a.asset_id, path, details: form.value() });
    store.invalidate();
    m.close();
    toast("Document added.", { kind: "success" });
    reload();
  }, "Encrypting…"));
}

async function documentDetails(a, d, reload) {
  const form = documentForm(d);
  const save = h("button", { class: "btn btn-primary" }, "Save");
  const m = modal({
    title: "Document details",
    size: "md",
    body: form.el,
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  save.addEventListener("click", () => busy(save, async () => {
    await call("describe_attachment", { assetId: a.asset_id, objectId: d.object_id, details: form.value() });
    m.close();
    toast("Saved.", { kind: "success" });
    reload();
  }));
}

async function removePhoto(a, photo, reload) {
  const ok = await confirmDialog({ title: "Remove this photo?", message: "It is removed from this asset. If no other asset uses it, the encrypted file is deleted from the vault.", confirmLabel: "Remove", danger: true });
  if (!ok) return;
  try {
    await call("remove_photo", { assetId: a.asset_id, objectId: photo.object_id });
    store.invalidate();
    reload();
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

function lightbox(images, start, name) {
  let index = start;
  const img = h("img", { class: "lightbox-img", alt: `Photo of ${name}` });
  const counter = h("span", { class: "lightbox-count" });
  const show = () => {
    img.src = mediaUrl(images[index].object_id);
    counter.textContent = `${index + 1} of ${images.length}`;
  };
  const step = (d) => { index = (index + d + images.length) % images.length; show(); };
  const m = modal({
    title: name,
    size: "full",
    body: h("div", { class: "lightbox", tabindex: "0", onkeydown: (e) => {
      if (e.key === "ArrowRight") step(1);
      if (e.key === "ArrowLeft") step(-1);
    } },
      images.length > 1 ? h("button", { class: "lightbox-nav prev", "aria-label": "Previous", onclick: () => step(-1) }, icon("back", { size: 26 })) : null,
      img,
      images.length > 1 ? h("button", { class: "lightbox-nav next", "aria-label": "Next", onclick: () => step(1) }, icon("forward", { size: 26 })) : null,
      counter
    ),
  });
  show();
  // Decrypted full-size images must not outlive the viewer.
  m.done.then(() => { img.removeAttribute("src"); });
}

// ------------------------------------------------------------ value

function valueCard(a, detail, reload) {
  const m = detail.market;
  const current = a.current_display
    ? h("div", { class: "value-big" }, fmt.money(a.current_display))
    : h("div", { class: "value-big muted" }, "No value yet");

  const meta = a.current_display
    ? h("div", { class: "value-source" }, sourceBadge(a.value_source, a.value_asof), h("span", {}, `as of ${fmt.date(a.value_asof)}`))
    : h("p", { class: "muted small" }, m ? `Waiting for a ${m.label.toLowerCase()} price.` : "Record what it is worth — your own research counts. Unvalued items are counted on the overview, never treated as zero.");

  const rows = [];
  if (a.acquired_display) {
    rows.push(["Paid", a.cost_complete
      ? fmt.money(a.acquired_display)
      : h("span", {}, fmt.money(a.acquired_display), " ",
          h("span", { class: "badge badge-attention", title: "Some of this holding was added without a price, so this covers only part of it. Edit the asset and enter the total paid for everything to see a gain." }, "Partial cost"))]);
  }
  if (a.gain_display) {
    const up = !a.gain_display.startsWith("-");
    const cost = Number(a.acquired_amount_minor);
    const pct = cost > 0 ? Number(a.gain_minor) / cost : NaN;
    rows.push(["Gain", h("span", { class: `gain ${up ? "up" : "down"}` }, icon(up ? "up" : "downRight", { size: 14 }), fmt.signedMoney(a.gain_display), Number.isFinite(pct) ? ` (${up ? "+" : ""}${fmt.percent(pct)})` : "")]);
  }
  if (a.insured_display) rows.push(["Insured for", fmt.money(a.insured_display)]);
  if (a.status === "sold") rows.push(["Sold", `${fmt.date(a.sold_date)}${a.sold_display ? ` for ${fmt.money(a.sold_display)}` : ""}`]);
  if (a.review_every_days) {
    rows.push(["Next review", a.review_due
      ? h("span", { class: "badge badge-attention" }, icon("clock", { size: 12 }), `Due — ${fmt.date(a.next_review)}`)
      : fmt.date(a.next_review)]);
  }

  let market = null;
  if (m) {
    const age = m.source_asof ? fmt.daysSince(m.source_asof) : null;
    const stale = age !== null && age >= 7;
    market = h("div", { class: "market-box" },
      h("div", { class: "market-line" },
        h("span", {}, m.label),
        m.unit_price
          ? h("strong", {}, `${fmt.unitPrice(m.unit_price, m.currency)} / ${m.unit}`)
          : h("span", { class: "muted" }, "no price yet")
      ),
      m.source_asof
        ? h("div", { class: stale ? "market-age warn" : "market-age" }, stale ? icon("warn", { size: 13 }) : icon("clock", { size: 13 }), `${m.source === "manual" ? "entered by hand" : `from ${m.source}`}, ${fmt.ago(m.source_asof)}`)
        : null,
      m.fine_troy_oz ? h("div", { class: "market-age" }, `Contains ${fmt.quantity(m.fine_troy_oz)} troy oz fine`) : null,
      a.status === "active"
        ? toggle("Follow the market price", a.pricing === "market", async (on) => {
            try {
              await call("set_pricing", { assetId: a.asset_id, pricing: on ? "market" : "manual" });
              store.invalidate();
              toast(on ? "Now valued from the market price." : "Now valued by hand only.", { kind: "success" });
              reload();
            } catch (e) {
              toast(describe(e), { kind: "error" });
              reload();
            }
          }, { hint: a.pricing === "market" ? "Revalued whenever prices update." : "Your own figures only; refreshes will not change it." })
        : null
    );
  }

  return h("section", { class: "card value-card" },
    h("div", { class: "card-label" }, "Current value"),
    current,
    meta,
    rows.length ? h("dl", { class: "kv" }, rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])) : null,
    market,
    a.attrs.watch_address ? watchBox(a, reload) : null
  );
}

/** Watch-only balance check, behind its own opt-in. */
function watchBox(a, reload) {
  const out = h("div", { class: "watch-result" });
  const check = h("button", { class: "btn btn-secondary btn-sm", onclick: () => busy(check, async () => {
    const settings = await store.settings({ fresh: true });
    if (!settings.balance_lookup) {
      mount(out, callout("info", "Balance lookup is off. Turning it on in Settings sends this address to a public block explorer, which learns someone at your IP is interested in it."));
      return;
    }
    const b = await call("lookup_balance", { chain: a.attrs.watch_chain ?? "bitcoin", address: a.attrs.watch_address, label: a.name });
    // Compare as normalized decimals: "0.50000000" and "0.5" are the same.
    const norm = (t) => (t.includes(".") ? t.replace(/0+$/, "").replace(/\.$/, "") : t);
    const differs = norm(b.confirmed) !== norm(a.quantity);
    mount(out,
      h("p", {}, `On-chain: `, h("strong", {}, `${fmt.quantity(b.confirmed)} ${b.unit}`), h("span", { class: "muted" }, ` · ${b.tx_count} transactions · ${b.source}`)),
      differs && a.status === "active"
        ? h("button", { class: "btn btn-primary btn-sm", onclick: async () => {
            try {
              await call("change_quantity", { change: { asset_id: a.asset_id, kind: "correct", quantity: b.confirmed, note: `Synced from ${b.source}` } });
              store.invalidate();
              toast("Quantity updated from the chain.", { kind: "success" });
              reload();
            } catch (e) { toast(describe(e), { kind: "error" }); }
          } }, `Set quantity to ${fmt.quantity(b.confirmed)}`)
        : h("p", { class: "muted small" }, "Matches the recorded quantity.")
    );
  }, "Checking…") }, icon("refresh", { size: 14 }), "Check balance");
  return h("div", { class: "market-box" },
    h("div", { class: "market-line" }, h("span", {}, "Watch-only address"), check),
    h("code", { class: "address" }, a.attrs.watch_address),
    out
  );
}

function historyCard(a, detail, reload) {
  const vals = detail.valuations;
  if (!vals.length) return null;

  // One currency per chart: minor units of two currencies are not one scale.
  // The current value's currency is charted; anything else stays in the
  // table below, and the chart says so.
  const currency = a.current_currency ?? vals[0].currency;
  const live = vals.filter((v) => !v.voided_at);
  const charted = [...live].reverse().filter((v) => v.currency === currency);
  const otherCurrencies = live.length - charted.length;
  const points = charted.map((v, i) => ({
    date: v.asof,
    display: v.amount,
    value: fmt.approxMajor(v.amount_minor, fmt.digitsOf(v.amount)),
    unvalued: 0,
    // A value for a different quantity: the step is partly a purchase or
    // sale, not a change in worth.
    event: i > 0 && v.quantity_at_time !== charted[i - 1].quantity_at_time,
  }));
  const chart = points.length >= 2 ? valueChart(points, { currency, height: 160, label: "Value history" }) : null;
  const notes = [];
  if (points.some((p) => p.event)) notes.push("Dots mark values recorded for a different quantity — part of that step is a purchase or sale.");
  if (otherCurrencies) notes.push(`${otherCurrencies} value${otherCurrencies === 1 ? "" : "s"} in other currencies ${otherCurrencies === 1 ? "is" : "are"} listed below but not charted.`);

  const row = (v) =>
    h("tr", { class: v.voided_at ? "voided" : null },
      h("td", {}, fmt.date(v.asof)),
      h("td", { class: "num" }, fmt.money(v.amount)),
      h("td", {}, h("div", { class: "name-cell" },
        h("span", {}, v.voided_at ? `Voided — ${v.void_reason ?? "no reason given"}` : fmt.PROVENANCE_LABELS[v.provenance] ?? v.provenance),
        h("span", { class: "sub" }, [fmt.BASIS_LABELS[v.basis] ?? v.basis, `${fmt.quantity(v.quantity_at_time)} held`, v.unit_price ? `at ${fmt.unitPrice(v.unit_price, v.currency)}` : null, v.note].filter(Boolean).join(" · ")),
        v.evidence ? evidenceDetails(v.evidence) : null
      )),
      h("td", { class: "row-action" }, v.voided_at ? null
        : h("button", { class: "icon-btn", title: "Void this value", "aria-label": `Void the value of ${v.amount} on ${v.asof}`, onclick: () => voidValuation(v, reload) }, icon("x", { size: 14 })))
    );
  const LIMIT = 12;
  const tbody = h("tbody", {}, vals.slice(0, LIMIT).map(row));
  const showAll = vals.length > LIMIT
    ? h("button", { class: "btn btn-ghost btn-sm", onclick: () => { mount(tbody, vals.map(row)); showAll.remove(); } }, `Show all ${vals.length}`)
    : null;
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, "Value history"), h("span", { class: "muted small" }, `${vals.length} record${vals.length === 1 ? "" : "s"}`)),
    chart,
    notes.length ? h("p", { class: "chart-notes" }, notes.join(" ")) : null,
    h("table", { class: "table compact" }, h("caption", { class: "sr-only" }, "Every recorded value, newest first"), tbody),
    showAll
  );
}

const COMPARABLE_LABELS = { sold: "Sold", auction: "Auction", asking: "Asking" };

/** Why a value is what it is, folded away under the row. */
function evidenceDetails(e) {
  const summary = [
    e.comparables.length ? `${e.comparables.length} comparable${e.comparables.length === 1 ? "" : "s"}` : null,
    e.range ? `range ${e.range.split(" – ").map((x) => (x.startsWith("from ") || x.startsWith("up to ") ? x : fmt.money(x))).join(" – ")}` : null,
    e.confidence ? `${e.confidence} confidence` : null,
  ].filter(Boolean).join(" · ") || "Evidence";
  return h("details", { class: "evidence" },
    h("summary", {}, summary),
    e.comparables.length
      ? h("ul", {}, e.comparables.map((c) => h("li", {},
          `${COMPARABLE_LABELS[c.kind] ?? c.kind} ${c.price ? fmt.money(c.price) : ""} — ${c.description}`,
          c.date ? ` (${fmt.date(c.date)})` : "")))
      : null,
    e.document_title ? h("p", { class: "muted small" }, `On file: ${e.document_title}`) : null
  );
}

/** Void a mistyped value: kept in the history, counted nowhere. */
async function voidValuation(v, reload) {
  const reason = textInputLike();
  const m = modal({
    title: "Void this value?",
    size: "sm",
    body: h("div", { class: "stack" },
      h("p", {}, `${fmt.money(v.amount)} as of ${fmt.date(v.asof)} will stop counting toward the current value and every past total. It stays in the history, marked as voided, so the correction is visible.`),
      field("Reason", reason, { hint: "For example: typo, wrong item, duplicate entry." })
    ),
    footer: (close) => [
      h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Cancel"),
      h("button", { class: "btn btn-danger", onclick: () => close(true) }, "Void value"),
    ],
  });
  if (!(await m.done)) return;
  try {
    await call("void_valuation", { valuationId: v.valuation_id, reason: reason.value.trim() });
    store.invalidate();
    toast("Value voided.", { kind: "success" });
    reload();
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

function textInputLike() {
  return h("input", { type: "text", maxlength: 200, autocomplete: "off", autofocus: true });
}

async function editTags(a, reload) {
  const known = await call("list_tags").catch(() => []);
  const tags = tagsInput(a.tags ?? [], known.map((t) => t.name));
  const save = h("button", { class: "btn btn-primary" }, "Save tags");
  const m = modal({
    title: "Tags",
    subtitle: a.name,
    size: "sm",
    body: h("div", { class: "stack" }, tags.el, h("p", { class: "field-hint" }, "Enter or comma adds a tag. Tags are searchable and filterable in Holdings.")),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  requestAnimationFrame(() => tags.input.focus());
  save.addEventListener("click", () => busy(save, async () => {
    await call("set_asset_tags", { assetId: a.asset_id, tags: tags.value() });
    store.invalidate();
    m.close();
    reload();
  }));
}

/** A new asset described like this one, opened for its differences. */
async function duplicateAsset(a, ctx) {
  try {
    const id = await call("duplicate_asset", { assetId: a.asset_id });
    store.invalidate();
    toast("Duplicated — without its serial or certificate number, value, history or files. Edit what differs.", { kind: "success", timeout: 7000 });
    ctx.navigate("asset", { id });
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

/** Earlier versions of the record, each restorable. */
async function editHistory(a, reload) {
  let revisions;
  try {
    revisions = await call("asset_revisions", { assetId: a.asset_id });
  } catch (e) {
    return toast(describe(e), { kind: "error" });
  }
  const FIELDS = [
    ["name", "Name"], ["type_id", "Type"], ["status", "Status"], ["storage_location", "Location"],
    ["acquired_date", "Acquired on"], ["acquired_from", "Acquired from"], ["notes", "Notes"],
    ["quantity_unit", "Unit"], ["review_every_days", "Review interval"],
  ];
  const changes = (s) => {
    const out = FIELDS.filter(([k]) => (s[k] ?? null) !== (a[k] ?? null)).map(([, label]) => label);
    if ((s.acquired_amount_minor ?? null) !== (a.acquired_amount_minor == null ? null : Number(a.acquired_amount_minor)) || (s.acquired_currency ?? null) !== (a.acquired_currency ?? null)) out.push("Total paid");
    if ((s.insured_amount_minor ?? null) !== (a.insured_amount_minor == null ? null : Number(a.insured_amount_minor)) || (s.insured_currency ?? null) !== (a.insured_currency ?? null)) out.push("Insured for");
    if (JSON.stringify(s.attrs) !== JSON.stringify(a.attrs)) out.push("Details");
    return out;
  };
  const m = modal({
    title: "Edit history",
    subtitle: a.name,
    size: "md",
    body: revisions.length
      ? h("div", { class: "stack" },
          h("p", { class: "muted small" }, "The record as it was before each edit, newest first. Restoring a version is itself an edit, so it can be undone here too."),
          h("ul", { class: "revision-list" }, revisions.map((r) => {
            const diff = changes(r.snapshot);
            return h("li", {},
              h("div", {},
                h("strong", {}, fmt.date(r.recorded_at)), " ", h("span", { class: "muted" }, new Date(r.recorded_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })),
                h("p", { class: "muted small" }, diff.length ? `Differs from now in: ${diff.join(", ")}` : "Same as now")
              ),
              diff.length
                ? h("button", { class: "btn btn-secondary btn-sm", onclick: async () => {
                    try {
                      await call("restore_revision", { assetId: a.asset_id, revisionId: r.revision_id });
                      store.invalidate();
                      m.close();
                      toast("Earlier version restored.", { kind: "success" });
                      reload();
                    } catch (e) { toast(describe(e), { kind: "error" }); }
                  } }, "Restore this version")
                : null
            );
          }))
        )
      : h("p", { class: "muted" }, "No edits yet. Each time you save changes, the version before is kept here."),
  });
}

// ------------------------------------------------------------ custody

/**
 * Where it is: home, or lent, consigned, at repair, in storage, shipped —
 * and the history of handoffs. Ownership and value are not affected.
 */
function custodyCard(a, detail, reload) {
  const card = h("section", { class: "card" });
  call("list_custody", { assetId: a.asset_id }).then((entries) => {
    const away = entries[0] && entries[0].kind !== "returned" ? entries[0] : null;
    const today = fmt.todayIso();
    const action = away
      ? h("button", { class: "btn btn-secondary btn-sm", onclick: () => handoff(a, detail, reload, true) }, "Mark as back…")
      : h("button", { class: "btn btn-secondary btn-sm", onclick: () => handoff(a, detail, reload, false) }, "Hand over…");
    mount(card,
      h("div", { class: "card-head" }, h("h2", {}, "Where it is"), action),
      away
        ? h("div", { class: "custody-now" },
            h("strong", {}, `${fmt.CUSTODY_LABELS[away.kind]} — ${away.party ?? ""}`),
            h("span", { class: "muted small" }, [`since ${fmt.date(away.date)}`, away.reference ? `ref. ${away.reference}` : null].filter(Boolean).join(" · ")),
            away.due_back ? h("span", { class: away.due_back < today ? "badge badge-attention" : "badge badge-muted" }, `${away.due_back < today ? "Overdue — was due" : "Due back"} ${fmt.date(away.due_back)}`) : null)
        : h("p", { class: "muted small" }, a.storage_location ? `Home — ${a.storage_location}.` : "Home."),
      entries.length
        ? h("details", { class: "disclosure" }, h("summary", {}, `History (${entries.length})`),
            h("ul", { class: "doc-list" }, entries.map((e) => h("li", {},
              h("div", { class: "doc-text" },
                h("strong", {}, fmt.CUSTODY_LABELS[e.kind] ?? e.kind, e.party ? ` — ${e.party}` : ""),
                h("span", { class: "muted small" }, [fmt.date(e.date), e.reference ? `ref. ${e.reference}` : null, e.contact, e.document_title ? `Document: ${e.document_title}` : null].filter(Boolean).join(" · ")),
                e.note ? h("span", { class: "doc-note" }, e.note) : null),
              h("div", { class: "doc-actions" }, h("button", { class: "icon-btn", "aria-label": "Remove this entry", onclick: async () => {
                if (!(await confirmDialog({ title: "Remove this entry?", message: "It is removed from where-it-is history.", confirmLabel: "Remove", danger: true }))) return;
                try { await call("delete_custody", { custodyId: e.custody_id }); reload(); } catch (err) { toast(describe(err), { kind: "error" }); }
              } }, icon("trash", { size: 16 }))))
            )))
        : null
    );
  }).catch(() => card.remove());
  return card;
}

function handoff(a, detail, reload, returning) {
  const docs = detail.photos.filter((p) => p.doc_kind !== "photo");
  const kind = select(["lent", "consigned", "repair", "storage", "shipped"].map((k) => [k, fmt.CUSTODY_LABELS[k]]), "lent");
  const party = h("input", { type: "text", maxlength: 200, placeholder: "Who has it" });
  const contact = h("input", { type: "text", maxlength: 300, placeholder: "Optional — phone or email" });
  const date = h("input", { type: "date", value: fmt.todayIso(), max: fmt.todayIso() });
  const due = h("input", { type: "date" });
  const reference = h("input", { type: "text", maxlength: 200, placeholder: "Consignment, ticket or tracking number" });
  const receipt = select([["", "None"], ...docs.map((d) => [d.object_id, d.title ?? fmt.DOC_KIND_LABELS[d.doc_kind]])], "");
  const note = h("input", { type: "text", maxlength: 2000 });
  const save = h("button", { class: "btn btn-primary" }, returning ? "Mark as back" : "Record handoff");
  const m = modal({
    title: returning ? "Back home" : "Hand it over",
    subtitle: a.name,
    size: "md",
    body: h("div", { class: "form-grid" },
      returning ? null : field("What", kind),
      returning ? null : field("To", party),
      field(returning ? "Came back on" : "On", date),
      returning ? null : field("Due back", due),
      returning ? null : field("Contact", contact, { hint: "Kept in the vault; never in exports or reports." }),
      field("Reference", reference),
      docs.length ? field("Receipt or document", receipt) : null,
      field("Note", note, { span: 2 })
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  save.addEventListener("click", () => busy(save, async () => {
    await call("record_custody", { entry: {
      asset_id: a.asset_id, kind: returning ? "returned" : kind.value, party: returning ? null : party.value.trim() || null,
      contact: returning ? null : contact.value.trim() || null, date: date.value, due_back: returning ? null : due.value || null,
      reference: reference.value.trim() || null, object_id: receipt.value || null, note: note.value.trim(),
    } });
    store.invalidate();
    m.close();
    reload();
  }));
}

// ------------------------------------------------------------ care

/** Services, repairs, inspections, warranties — and when the next is due. */
function careCard(a, detail, reload) {
  const card = h("section", { class: "card" });
  const add = h("button", { class: "btn btn-secondary btn-sm", onclick: () => addCare(a, detail, reload) }, icon("plus", { size: 14 }), "Add…");
  call("list_care", { assetId: a.asset_id }).then((entries) => {
    const today = fmt.todayIso();
    // The latest of each kind is the one whose "next due" counts.
    const latestOfKind = new Set();
    const seen = new Set();
    for (const e of entries) if (!seen.has(e.kind)) { seen.add(e.kind); latestOfKind.add(e.care_id); }
    mount(card,
      h("div", { class: "card-head" }, h("h2", {}, "Care & service"), add),
      entries.length
        ? h("ul", { class: "doc-list" }, entries.map((e) => {
            const due = latestOfKind.has(e.care_id) && e.next_due;
            return h("li", {},
              h("span", { class: "doc-icon" }, icon("clock", { size: 18 })),
              h("div", { class: "doc-text" },
                h("strong", {}, fmt.CARE_LABELS[e.kind] ?? e.kind, e.provider ? ` — ${e.provider}` : ""),
                h("span", { class: "muted small" }, [e.performed_on ? fmt.date(e.performed_on) : "Not done yet", e.cost_display ? fmt.money(e.cost_display) : null, e.document_title ? `Document: ${e.document_title}` : null].filter(Boolean).join(" · ")),
                e.note ? h("span", { class: "doc-note" }, e.note) : null,
                due
                  ? h("span", { class: e.next_due < today ? "badge badge-attention" : "badge badge-muted" }, `${e.kind === "warranty" ? "Ends" : "Next due"} ${fmt.date(e.next_due)}`)
                  : null
              ),
              h("div", { class: "doc-actions" },
                h("button", { class: "icon-btn", title: "Remove", "aria-label": "Remove this entry", onclick: async () => {
                  if (!(await confirmDialog({ title: "Remove this entry?", message: "It is removed from the care history.", confirmLabel: "Remove", danger: true }))) return;
                  try { await call("delete_care", { careId: e.care_id }); reload(); } catch (err) { toast(describe(err), { kind: "error" }); }
                } }, icon("trash", { size: 16 }))
              )
            );
          }))
        : h("p", { class: "muted small" }, "Services, repairs, inspections and warranties, with when the next is due — the history a buyer or insurer asks for.")
    );
  }).catch(() => card.remove());
  return card;
}

function addCare(a, detail, reload) {
  const docs = detail.photos.filter((p) => p.doc_kind !== "photo");
  const kind = select(fmt.CARE_KINDS.map((k) => [k, fmt.CARE_LABELS[k]]), "service");
  const done = h("input", { type: "date", value: fmt.todayIso(), max: fmt.todayIso() });
  const provider = h("input", { type: "text", maxlength: 200, placeholder: "Who did it" });
  const cost = h("input", { type: "text", inputmode: "decimal", class: "input-money", placeholder: "0.00" });
  const currency = select(fmt.COMMON_CURRENCIES.map((c) => [c, c]), a.current_currency ?? a.acquired_currency ?? "USD", { class: "affix affix-select", "aria-label": "Currency" });
  const next = h("input", { type: "date" });
  const invoice = select([["", "None"], ...docs.map((d) => [d.object_id, d.title ?? fmt.DOC_KIND_LABELS[d.doc_kind]])], "");
  const note = h("input", { type: "text", maxlength: 2000 });
  const inYears = (n) => {
    const base = done.value || fmt.todayIso();
    const [y, m, d] = base.split("-").map(Number);
    const date = new Date(y + n, m - 1, d);
    const pad = (x) => String(x).padStart(2, "0");
    next.value = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  };
  kind.addEventListener("change", () => {
    // A warranty is about when it ends; its start may not be known.
    if (kind.value === "warranty" && done.value === fmt.todayIso()) done.value = "";
  });
  const save = h("button", { class: "btn btn-primary" }, "Add");
  const m = modal({
    title: "Add care or service",
    subtitle: a.name,
    size: "md",
    body: h("div", { class: "form-grid" },
      field("What", kind),
      field("Done on", done, { hint: "Leave blank for something only due, like a warranty's end." }),
      field("By", provider),
      field("Cost", h("div", { class: "input-affix" }, currency, cost), { hint: "Not added to what it cost to buy." }),
      field("Next due", h("div", { class: "stack" }, next,
        h("div", { class: "btn-row" }, [1, 2, 5].map((n) => h("button", { type: "button", class: "btn btn-ghost btn-sm", onclick: () => inYears(n) }, `+${n} year${n === 1 ? "" : "s"}`)))),
        { hint: "Shown on the overview as it comes up." }),
      docs.length ? field("Invoice or document", invoice) : null,
      field("Note", note, { span: 2 })
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), save],
  });
  save.addEventListener("click", () => busy(save, async () => {
    await call("add_care", { entry: {
      asset_id: a.asset_id, kind: kind.value, performed_on: done.value || null, provider: provider.value.trim() || null,
      cost: cost.value.trim() || null, currency: currency.value, next_due: next.value || null,
      object_id: invoice.value || null, note: note.value.trim(),
    } });
    m.close();
    toast("Added to the care history.", { kind: "success" });
    reload();
  }));
}

const STATUS_EVENT_LABELS = { lost: "Marked lost", retired: "Retired", active: "Recovered" };

function eventsCard(detail) {
  const statuses = detail.status_events ?? [];
  if (!detail.events.length && !statuses.length) return null;
  // Quantity changes and status changes in one timeline, newest first.
  const entries = [
    ...detail.events.map((e) => ({ date: e.effective_date, recorded: e.recorded_at, quantity: e })),
    ...statuses.map((s) => ({ date: s.effective_date, recorded: s.recorded_at, status: s })),
  ].sort((a, b) => b.date.localeCompare(a.date) || b.recorded.localeCompare(a.recorded));
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, "Holding history")),
    h("ul", { class: "timeline" }, entries.map(({ quantity: e, status: s }) => {
      if (s) {
        return h("li", {},
          h("span", { class: `timeline-dot ev-status-${s.status}` }),
          h("span", {}, h("strong", {}, STATUS_EVENT_LABELS[s.status] ?? s.status)),
          s.note ? h("span", { class: "timeline-note" }, s.note) : null,
          h("span", { class: "timeline-date" }, fmt.date(s.effective_date))
        );
      }
      const n = e.quantity_delta.replace(/^-/, "");
      const sign = e.quantity_delta.startsWith("-") ? "−" : "+";
      return h("li", {},
        h("span", { class: `timeline-dot ev-${e.event_type}` }),
        h("span", {}, h("strong", {}, fmt.EVENT_LABELS[e.event_type] ?? e.event_type), " ", h("span", { class: "muted" }, `${sign}${fmt.quantity(n)}`)),
        e.amount_display ? h("span", { class: "muted" }, ` · ${fmt.money(e.amount_display)}`) : null,
        e.note ? h("span", { class: "timeline-note" }, e.note) : null,
        h("span", { class: "timeline-date" }, fmt.date(e.effective_date))
      );
    }))
  );
}

// ------------------------------------------------------------ details

function detailsCard(a, detail) {
  const rows = [
    ["Quantity", `${fmt.quantity(a.quantity)} ${a.quantity_unit}`],
    ["Type", a.type_label],
  ];
  if (a.acquired_date) rows.push(["Acquired", fmt.date(a.acquired_date)]);
  if (a.acquired_from) rows.push(["Acquired from", a.acquired_from]);
  if (a.storage_location) rows.push(["Location", h("span", { class: "with-icon" }, icon("pin", { size: 14 }), a.storage_location)]);

  // Metal specification, shown in words: the gross/fine distinction is the
  // detail most often gotten wrong.
  if (a.attrs.metal) {
    const metal = { XAU: "Gold", XAG: "Silver", XPT: "Platinum", XPD: "Palladium" }[a.attrs.metal] ?? a.attrs.metal;
    const unit = { troy_oz: "troy oz", gram: "g", pennyweight: "dwt", ounce: "oz" }[a.attrs.weight_unit] ?? a.attrs.weight_unit;
    rows.push(["Metal", metal]);
    rows.push(["Each weighs", `${fmt.quantity(a.attrs.weight_per_item)} ${unit} ${a.attrs.weight_basis === "fine" ? "fine" : "gross"}`]);
    if (a.attrs.weight_basis !== "fine") rows.push(["Purity", a.attrs.purity]);
    if (a.attrs.premium_pct) rows.push(["Premium over melt", `${a.attrs.premium_pct}%`]);
    if (detail.market?.fine_troy_oz) rows.push(["Fine content", `${fmt.quantity(detail.market.fine_troy_oz)} troy oz`]);
  }
  if (a.attrs.coin_id) {
    rows.push(["Coin", `${a.attrs.symbol ?? ""} (${a.attrs.coin_id})`]);
    if (a.attrs.custody) rows.push(["Held in", { self_custody: "Own wallet", exchange: "Exchange", locked: "Staked or locked" }[a.attrs.custody] ?? a.attrs.custody]);
  }

  for (const [key, value] of Object.entries(a.attrs)) {
    if (INTERNAL.has(key)) continue;
    let shown = value;
    if (key === "grader") shown = value === "raw" ? "Ungraded" : value.toUpperCase();
    rows.push([fmt.fieldLabel(key), shown]);
  }
  rows.push(["Added", fmt.date(a.created_at)]);

  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, "Details")),
    h("dl", { class: "kv kv-wide" }, rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)]))
  );
}

function notesCard(a) {
  if (!a.notes) return null;
  return h("section", { class: "card" }, h("div", { class: "card-head" }, h("h2", {}, "Notes")), h("p", { class: "notes" }, a.notes));
}

async function deleteAsset(a, ctx) {
  const ok = await confirmDialog({
    title: `Move “${a.name}” to the trash?`,
    message: [
      "It leaves your catalog, totals, reports and exports. Its history, photos and documents are kept, so you can restore it from Settings → Trash for 30 days. After that it is deleted for good.",
      "If you sold it, use “Record change → Sold all” instead: that keeps its past value in your charts.",
    ],
    confirmLabel: "Move to trash",
    danger: true,
  });
  if (!ok) return;
  try {
    await call("delete_asset", { assetId: a.asset_id });
    store.invalidate();
    const dismiss = toast(h("span", {}, `Moved ${a.name} to the trash. `,
      h("button", { class: "link", onclick: async () => {
        dismiss();
        try {
          await call("restore_asset", { assetId: a.asset_id });
          store.invalidate();
          ctx.navigate("asset", { id: a.asset_id });
        } catch (e) { toast(describe(e), { kind: "error" }); }
      } }, "Undo")), { kind: "success", timeout: 9000 });
    ctx.navigate("holdings");
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

