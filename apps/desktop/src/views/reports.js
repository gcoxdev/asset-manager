// Reports: the insurance report, and CSV export/import.
//
// Both leave the vault's protection by design. Each says so before anything
// is written, not after — once the file exists, the catalog is outside the
// encryption.

import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import { call, describe } from "../lib/api.js";
import { mediaUrl } from "../lib/media.js";
import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { busy, toast, confirmDialog, modal, callout, field } from "../ui/components.js";
import { runPrint } from "./onboarding.js";
import { importSpreadsheet, saveTemplate } from "./spreadsheet-import.js";

export async function renderReports(root, params, ctx) {
  if (params?.claim?.length) setTimeout(() => openClaimWorkbench(params.claim, null));
  const options = { include_locations: false, include_notes: false, include_photos: true, include_lost: false, include_documents: true };
  const check = (key, label, hint) => {
    const input = h("input", { type: "checkbox", checked: options[key], onchange: () => (options[key] = input.checked) });
    return h("label", { class: "check" }, input, h("span", {}, h("span", {}, label), h("span", { class: "check-hint" }, hint)));
  };

  const preview = h("div", { class: "report-preview" });
  const build = h("button", { class: "btn btn-primary", onclick: () => busy(build, async () => {
    const report = await call("insurance_report", { options });
    mount(preview, reportSheet(report, { screen: true }),
      h("div", { class: "btn-row sticky-actions" },
        h("button", { class: "btn btn-ghost", onclick: () => mount(preview) }, "Close preview"),
        h("button", { class: "btn btn-primary", onclick: () => printReport(report) }, icon("printer", { size: 16 }), "Print or save as PDF")
      )
    );
    preview.scrollIntoView({ behavior: "smooth", block: "start" });
  }, "Building…") }, icon("reports", { size: 16 }), "Preview report");

  mount(
    root,
    h("header", { class: "page-head" }, h("div", {}, h("h1", {}, "Reports"), h("p", { class: "page-sub" }, "Documents for insurers, and spreadsheets for bulk edits"))),
    h("section", { class: "card" },
      h("div", { class: "card-head" }, h("div", {}, h("h2", {}, "Insurance inventory"), h("p", { class: "card-sub" }, "Every held item with its photos, identifying details, cost and value — and where each value came from, so an assessor can tell a market price from an estimate."))),
      h("div", { class: "check-list" },
        check("include_photos", "Include photos", "Up to four per item, embedded in the document."),
        check("include_documents", "List documents on file", "Receipts, appraisals and certificates by title and date, so an assessor knows what to ask for. Their contents are not included."),
        check("include_locations", "Include storage locations", "Usually unnecessary for a claim — and a list of where valuables are kept is exactly what should not leak."),
        check("include_notes", "Include notes", "Your free-text notes, as written."),
        check("include_lost", "Include items marked lost", "For a claim: each lost item with the date it was lost and its value from before.")
      ),
      callout("warning", "The printed report and any PDF you save are not encrypted. Treat the file as you would the items themselves."),
      h("div", { class: "btn-row" }, build)
    ),
    preview,
    h("section", { class: "card" },
      h("div", { class: "card-head" }, h("div", {}, h("h2", {}, "Prepare an insurance claim"), h("p", { class: "card-sub" }, "For items lost, stolen or damaged: just those items, valued as they were before the loss, with their receipts and photos ready to send."))),
      h("div", { class: "btn-row" }, h("button", { class: "btn btn-primary", onclick: () => openClaimWorkbench([], ctx) }, icon("reports", { size: 16 }), "Prepare a claim…"))
    ),
    h("section", { class: "card" },
      h("div", { class: "card-head" }, h("div", {}, h("h2", {}, "Import your own spreadsheet"), h("p", { class: "card-sub" }, "An inventory you already keep — any columns, prices as you wrote them. You say what each column holds and check every row before anything is added."))),
      h("ul", { class: "rules" },
        h("li", {}, "Save it as CSV from Excel, Numbers or Google Sheets. Comma, semicolon and tab separated files all work."),
        h("li", {}, "Prices are read as written — $1,299.50 or 1.299,50 — and dates in the order you choose."),
        h("li", {}, "Likely duplicates of what is already in your catalog are pointed out before you import.")
      ),
      h("div", { class: "btn-row" },
        h("button", { class: "btn btn-primary", onclick: () => importSpreadsheet(() => ctx.refresh()) }, icon("upload", { size: 16 }), "Import a spreadsheet…"),
        h("button", { class: "btn btn-ghost", onclick: () => saveTemplate() }, icon("download", { size: 16 }), "Save a template…")
      )
    ),
    h("section", { class: "card" },
      h("div", { class: "card-head" }, h("div", {}, h("h2", {}, "Spreadsheet round trip"), h("p", { class: "card-sub" }, "For a few hundred items, export → edit in a spreadsheet → import beats any form."))),
      h("ul", { class: "rules" },
        h("li", {}, "Each row keeps its asset ID, so importing updates rather than duplicating."),
        h("li", {}, "A blank cell leaves the stored value alone; a single dash (-) clears it."),
        h("li", {}, "A changed value becomes a dated valuation; a changed quantity becomes a correction in the history."),
        h("li", {}, "An old export is refused if the catalog has changed since, so it cannot overwrite newer edits."),
        h("li", {}, "Amounts are in minor units (cents): 1299.50 is written 129950.")
      ),
      callout("info", "CSV is not a backup: it has no photos or history, and it is plain text. Use Settings → Backup to keep a restorable copy."),
      h("div", { class: "btn-row" },
        h("button", { class: "btn btn-secondary", onclick: () => importCsv(() => ctx.refresh()) }, icon("upload", { size: 16 }), "Import CSV…"),
        h("button", { class: "btn btn-secondary", onclick: () => exportCsv() }, icon("download", { size: 16 }), "Export CSV…")
      )
    )
  );
}

export function reportSheet(report, { screen, claim } = {}) {
  const total = report.valued + report.unvalued;
  const byCategory = new Map();
  for (const item of report.items) {
    if (!byCategory.has(item.category)) byCategory.set(item.category, []);
    byCategory.get(item.category).push(item);
  }
  return h("div", { class: screen ? "print-sheet report-sheet on-screen" : "print-sheet report-sheet" },
    h("header", { class: "report-head" },
      h("div", {},
        h("h1", {}, claim ? "Insurance claim — itemized loss" : "Asset inventory"),
        h("p", { class: "report-meta" }, `Prepared ${fmt.date(report.generated_at)} · ${total} item${total === 1 ? "" : "s"}${report.as_of ? ` · values as of ${fmt.date(report.as_of)}` : ""}`)),
      h("div", { class: "report-total" }, h("span", {}, claim ? "Total claimed (recorded value)" : "Total recorded value"), h("strong", {}, fmt.money(report.total)))
    ),
    claim ? claimHeader(claim) : null,
    report.unvalued
      ? h("p", { class: "report-note" }, `${report.valued} of ${total} items have a recorded value. Items without one are listed but add nothing to the total.`)
      : null,
    report.lost
      ? h("p", { class: "report-note" }, `Includes ${report.lost} item${report.lost === 1 ? "" : "s"} marked lost, valued as last recorded before the loss.`)
      : null,
    h("table", { class: "report-summary" },
      h("thead", {}, h("tr", {}, h("th", {}, "Category"), h("th", { class: "num" }, "Items"), h("th", { class: "num" }, "Value"))),
      h("tbody", {}, report.categories.map((c) => h("tr", {}, h("td", {}, fmt.categoryLabel(c.category)), h("td", { class: "num" }, String(c.count)), h("td", { class: "num" }, fmt.money(c.total))))),
      !report.insured_total.startsWith("0.") && !/^0 /.test(report.insured_total)
        ? h("tfoot", {}, h("tr", {}, h("td", {}, "Insured values recorded"), h("td"), h("td", { class: "num" }, fmt.money(report.insured_total))))
        : null
    ),
    [...byCategory].map(([category, items]) => [
      h("h2", { class: "report-category" }, fmt.categoryLabel(category)),
      items.map((item) => {
        const rows = [
          ["Type", item.type_label],
          ["Quantity", `${fmt.quantity(item.quantity)} ${item.quantity_unit}`],
          ...item.details.map(([k, v]) => [k, v]),
          ["Acquired", [item.acquired_date ? fmt.date(item.acquired_date) : null, item.acquired ? fmt.money(item.acquired) : null, item.acquired_from].filter(Boolean).join(" · ") || "Not recorded"],
          ["Value", item.current
            ? `${fmt.money(item.current)} — ${fmt.PROVENANCE_LABELS[item.value_source] ?? item.value_source ?? "unknown source"}, ${fmt.date(item.value_asof)}`
            : "Not recorded"],
        ];
        if (item.status === "lost") rows.unshift(["Status", `Lost${item.lost_on ? ` on ${fmt.date(item.lost_on)}` : ""}`]);
        if (item.insured) rows.push(["Insured for", fmt.money(item.insured)]);
        if (item.values_by_basis?.length > 1) {
          rows.push(["Values on record", item.values_by_basis.map((b) => `${fmt.BASIS_LABELS[b.basis] ?? b.basis} ${fmt.money(b.value)} (${fmt.date(b.asof)})`).join("; ")]);
        }
        if (item.documents?.length) rows.push(["Documents on file", item.documents.map((d) => [d.title ?? fmt.DOC_KIND_LABELS[d.kind], `${fmt.DOC_KIND_LABELS[d.kind] ?? d.kind}${d.date ? `, ${fmt.date(d.date)}` : ""}`].join(" — ")).join("; ")]);
        if (item.storage_location) rows.push(["Location", item.storage_location]);
        if (item.notes) rows.push(["Notes", item.notes]);
        return h("article", { class: "report-item" },
          h("div", { class: "report-item-text" },
            h("h3", {}, item.name),
            h("dl", {}, rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)]))
          ),
          item.photo_ids.length
            ? h("div", { class: "report-photos" }, item.photo_ids.map((id) => h("img", { src: mediaUrl(id, 1024), alt: "" })))
            : null
        );
      }),
    ]),
    h("footer", { class: "report-foot" }, report.warning)
  );
}

function printReport(report, claim = null) {
  mount(document.getElementById("print-root"), reportSheet(report, { claim }));
  runPrint(claim ? `Insurance-Claim-${claim.claim_number || fmt.todayIso()}` : `Asset-Manager-Inventory-${fmt.todayIso()}`);
}

function claimHeader(claim) {
  const rows = [
    ["Claimant", claim.claimant],
    ["Insurer", claim.insurer],
    ["Policy number", claim.policy_number],
    ["Claim number", claim.claim_number],
    ["Date of loss", claim.loss_date ? fmt.date(claim.loss_date) : null],
    ["What happened", claim.description],
  ].filter(([, v]) => v);
  return h("section", { class: "claim-header" }, h("dl", {}, rows.map(([k, v]) => [h("dt", {}, k), h("dd", {}, v)])));
}

/**
 * Prepare a claim: chosen items only, valued as they stood before the loss,
 * with the claim's details on the first page and their receipts and photos
 * exported beside it. Claim details are used for this document and not kept.
 */
export async function openClaimWorkbench(preselected = [], ctx) {
  const assets = await store.assets({ fresh: true });
  const chosen = new Set(preselected);
  const m = modal({ title: "Prepare a claim", size: "lg", body: h("div") });
  const body = m.dialog.querySelector(".modal-body");
  const claim = { claimant: "", insurer: "", policy_number: "", claim_number: "", loss_date: fmt.todayIso(), description: "" };
  const opts = { include_photos: true, include_documents: true, compare_bases: true, include_locations: false, include_notes: false };

  function chooseStep() {
    const search = h("input", { type: "search", placeholder: "Find items…", "aria-label": "Find items" });
    const list = h("div", { class: "claim-pick" });
    const count = h("strong", {});
    const drawList = () => {
      const q = search.value.trim().toLowerCase();
      const shown = assets.filter((a) => !q || a.name.toLowerCase().includes(q) || (a.attrs.serial_number ?? "").toLowerCase().includes(q));
      count.textContent = `${chosen.size} chosen`;
      mount(list, shown.slice(0, 300).map((a) => {
        const box = h("input", { type: "checkbox", checked: chosen.has(a.asset_id), onchange: () => {
          if (box.checked) chosen.add(a.asset_id); else chosen.delete(a.asset_id);
          count.textContent = `${chosen.size} chosen`;
        } });
        return h("label", { class: "check claim-item" }, box,
          h("span", {}, h("span", {}, a.name, " ", statusLabel(a.status)), h("span", { class: "check-hint" }, [a.type_label, a.current_display ? fmt.money(a.current_display) : "no value"].join(" · "))));
      }));
    };
    search.addEventListener("input", drawList);
    drawList();

    const input = (key, props = {}) => {
      const el = h(props.multiline ? "textarea" : "input", { type: props.type ?? "text", value: claim[key], rows: 3, maxlength: 2000, max: props.max, onchange: () => (claim[key] = el.value) });
      if (props.multiline) el.value = claim[key];
      el.addEventListener("input", () => (claim[key] = el.value));
      return el;
    };
    const check = (key, label, hint) => {
      const box = h("input", { type: "checkbox", checked: opts[key], onchange: () => (opts[key] = box.checked) });
      return h("label", { class: "check" }, box, h("span", {}, h("span", {}, label), hint ? h("span", { class: "check-hint" }, hint) : null));
    };
    const next = h("button", { class: "btn btn-primary" }, "Preview the claim");
    next.addEventListener("click", () => busy(next, async () => {
      if (!chosen.size) return toast("Choose the items the claim is for.", { kind: "warning" });
      await previewStep();
    }, "Preparing…"));

    mount(body,
      h("p", { class: "lede" }, "Only the items you choose appear — nothing else in your catalog is disclosed. Values are those recorded on or before the date of loss."),
      h("div", { class: "claim-layout" },
        h("section", {},
          h("h3", {}, "Items ", count),
          search,
          list
        ),
        h("section", { class: "stack" },
          h("h3", {}, "The claim"),
          h("div", { class: "form-grid" },
            field("Date of loss", input("loss_date", { type: "date", max: fmt.todayIso() })),
            field("Claim number", input("claim_number")),
            field("Insurer", input("insurer")),
            field("Policy number", input("policy_number")),
            field("Your name", input("claimant"), { span: 2 }),
            field("What happened", input("description", { multiline: true }), { span: 2 })
          ),
          h("p", { class: "field-hint" }, "These details go on the document. They are not saved in the vault."),
          h("div", { class: "check-list" },
            check("include_photos", "Photos", "Up to four per item."),
            check("include_documents", "List receipts and appraisals on file"),
            check("compare_bases", "Show every value on record", "Resale, replacement and insured values side by side."),
            check("include_locations", "Storage locations", "Rarely needed for a claim."),
            check("include_notes", "Notes")
          )
        )
      ),
      h("div", { class: "form-actions" }, h("div", { class: "btn-row" }, h("button", { class: "btn btn-ghost", onclick: () => m.close() }, "Cancel"), next))
    );
  }

  async function previewStep() {
    const report = await call("insurance_report", {
      options: { ...opts, include_lost: true, asset_ids: [...chosen], as_of: claim.loss_date || null },
    });
    const files = h("button", { class: "btn btn-secondary" }, icon("download", { size: 16 }), "Save receipts & photos…");
    files.addEventListener("click", async () => {
      const ok = await confirmDialog({
        title: "Save unencrypted copies?",
        message: [
          "The chosen items' receipts, appraisals and certificates — and their photos, if included — are decrypted into a new folder, to send with the claim.",
          "They are ordinary files, outside the vault's protection. Delete them when the claim is settled.",
        ],
        confirmLabel: "Choose a folder…",
      });
      if (!ok) return;
      const directory = await openDialog({ directory: true, title: "Where to put the claim files" });
      if (!directory) return;
      await busy(files, async () => {
        const result = await call("export_claim_files", { assetIds: [...chosen], directory, includePhotos: opts.include_photos });
        toast(`Saved ${result.files} file${result.files === 1 ? "" : "s"} to ${result.folder}.`, { kind: "success", timeout: 9000 });
      }, "Saving…");
    });
    const unvalued = report.items.filter((i) => !i.current).length;
    mount(body,
      unvalued
        ? callout("warning", `${unvalued} item${unvalued === 1 ? " has" : "s have"} no value recorded on or before ${fmt.date(report.as_of)}. Record a value with an earlier date if you have one — a receipt or appraisal — or the claim shows it as unvalued.`)
        : null,
      callout("warning", "The printed claim and any PDF you save are not encrypted."),
      reportSheet(report, { screen: true, claim }),
      h("div", { class: "form-actions" }, h("div", { class: "btn-row" },
        h("button", { class: "btn btn-ghost", onclick: () => chooseStep() }, icon("back", { size: 16 }), "Change"),
        files,
        h("button", { class: "btn btn-primary", onclick: () => printReport(report, claim) }, icon("printer", { size: 16 }), "Print or save as PDF")
      ))
    );
  }

  chooseStep();
  await m.done;
  ctx?.refresh?.();
}

function statusLabel(status) {
  return status === "active" ? null : h("span", { class: "badge badge-muted" }, fmt.STATUS_LABELS[status] ?? status);
}

export async function exportCsv() {
  try {
    const result = await call("export_csv");
    const ok = await confirmDialog({
      title: "Export is not encrypted",
      message: [result.warning, `Export ${result.row_count} asset${result.row_count === 1 ? "" : "s"}?`],
      confirmLabel: "Choose where to save…",
    });
    if (!ok) return;
    const path = await saveDialog({ defaultPath: `asset-manager-export-${fmt.todayIso()}.csv`, filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!path) return;
    await call("write_text_file", { path, contents: result.csv });
    toast(`Exported ${result.row_count} asset${result.row_count === 1 ? "" : "s"}.`, { kind: "success" });
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

export async function importCsv(onDone) {
  const path = await openDialog({ filters: [{ name: "CSV", extensions: ["csv"] }] });
  if (!path) return;
  try {
    const contents = await call("read_text_file", { path });
    // Preview first, always: it reports every row problem at once.
    const preview = await call("import_csv", { contents, apply: false });
    if (preview.errors.length) {
      modal({
        title: `${preview.errors.length} problem${preview.errors.length === 1 ? "" : "s"} — nothing was imported`,
        subtitle: "Fix these in the spreadsheet and import again.",
        body: h("ul", { class: "error-list" }, preview.errors.slice(0, 50).map((e) => h("li", {}, e)), preview.errors.length > 50 ? h("li", {}, `…and ${preview.errors.length - 50} more`) : null),
        footer: (close) => [h("button", { class: "btn btn-primary", onclick: () => close() }, "OK")],
      });
      return;
    }
    if (!preview.creates && !preview.updates) {
      toast("That file has no rows to import.", { kind: "info" });
      return;
    }
    const ok = await confirmDialog({
      title: "Import this spreadsheet?",
      message: `${preview.creates} new asset${preview.creates === 1 ? "" : "s"} will be created and ${preview.updates} updated. Every row is applied together, or none are.`,
      confirmLabel: "Import",
    });
    if (!ok) return;
    const applied = await call("import_csv", { contents, apply: true });
    store.invalidate();
    toast(`Imported: ${applied.creates} created, ${applied.updates} updated.`, { kind: "success" });
    onDone?.();
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}
