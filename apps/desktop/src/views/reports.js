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
import { busy, toast, confirmDialog, modal, callout } from "../ui/components.js";
import { runPrint } from "./onboarding.js";

export async function renderReports(root, _params, ctx) {
  const options = { include_locations: false, include_notes: false, include_photos: true, include_lost: false };
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
        check("include_locations", "Include storage locations", "Usually unnecessary for a claim — and a list of where valuables are kept is exactly what should not leak."),
        check("include_notes", "Include notes", "Your free-text notes, as written."),
        check("include_lost", "Include items marked lost", "For a claim: each lost item with the date it was lost and its value from before.")
      ),
      callout("warning", "The printed report and any PDF you save are not encrypted. Treat the file as you would the items themselves."),
      h("div", { class: "btn-row" }, build)
    ),
    preview,
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

function reportSheet(report, { screen } = {}) {
  const total = report.valued + report.unvalued;
  const byCategory = new Map();
  for (const item of report.items) {
    if (!byCategory.has(item.category)) byCategory.set(item.category, []);
    byCategory.get(item.category).push(item);
  }
  return h("div", { class: screen ? "print-sheet report-sheet on-screen" : "print-sheet report-sheet" },
    h("header", { class: "report-head" },
      h("div", {}, h("h1", {}, "Asset inventory"), h("p", { class: "report-meta" }, `Prepared ${fmt.date(report.generated_at)} · ${total} item${total === 1 ? "" : "s"}`)),
      h("div", { class: "report-total" }, h("span", {}, "Total recorded value"), h("strong", {}, fmt.money(report.total)))
    ),
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

function printReport(report) {
  mount(document.getElementById("print-root"), reportSheet(report));
  runPrint(`Asset-Manager-Inventory-${fmt.todayIso()}`);
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
