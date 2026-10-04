// Importing someone's own spreadsheet: say what each column is, see every
// row as it will be stored, then import them all together.
//
// The backend guesses the columns and remembers how a file laid out the same
// way was read last time. The preview is the import itself, rolled back, so
// what it shows is exactly what importing stores.

import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import { call, describe } from "../lib/api.js";
import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { modal, field, select, busy, toast, callout } from "../ui/components.js";

const FIELD_LABELS = {
  name: "Name",
  type: "Type",
  quantity: "Quantity",
  unit: "Unit",
  acquired_date: "Date acquired",
  acquired_price: "Price paid (total)",
  acquired_from: "Acquired from",
  storage_location: "Storage location",
  notes: "Notes",
  current_value: "Current value",
  insured_value: "Insured value",
  currency: "Currency",
  tags: "Tags",
};

/** Detail keys offered for any column, beyond keeping its own name. */
const COMMON_DETAILS = ["serial_number", "brand", "model", "year", "condition", "manufacturer", "cert_number", "grade", "grader"];

const DATE_ORDERS = [
  ["month_day_year", "Month first — 03/15/2024"],
  ["day_month_year", "Day first — 15/03/2024"],
  ["year_month_day", "Year first — 2024-03-15"],
];

/** A template people can fill in, with headers the importer recognizes. */
export async function saveTemplate() {
  const path = await saveDialog({ defaultPath: "Asset Manager import template.csv", filters: [{ name: "CSV", extensions: ["csv"] }] });
  if (!path) return;
  const rows = [
    ["Item", "Type", "Quantity", "Purchase price", "Date bought", "Bought from", "Location", "Value", "Insured value", "Serial number", "Brand", "Model", "Tags", "Notes"],
    ["Omega Speedmaster", "Watch", "1", "4100.50", "2019-11-02", "Jeweller on Main St", "Safe / Top shelf", "5200", "5500", "OM-12345", "Omega", "311.30.42.30.01.005", "insured rider", "Box and papers"],
    ["American Silver Eagle", "Silver", "20", "640", "2023-05-10", "Coin shop", "Safe", "", "", "", "", "", "", "Bullion coins"],
  ];
  const csv = rows.map((r) => r.map((c) => (/[",\n]/.test(c) ? `"${c.replace(/"/g, '""')}"` : c)).join(",")).join("\n") + "\n";
  try {
    await call("write_text_file", { path, contents: csv });
    toast("Template saved. Fill it in, then import it.", { kind: "success" });
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

export async function importSpreadsheet(onDone) {
  const path = await openDialog({ filters: [{ name: "Spreadsheet (CSV)", extensions: ["csv", "txt", "tsv"] }] });
  if (!path) return;
  let contents;
  let inspected;
  try {
    contents = await call("read_text_file", { path });
    inspected = await call("inspect_spreadsheet", { contents });
  } catch (e) {
    return toast(describe(e), { kind: "error" });
  }
  if (!inspected.row_count) return toast("That file has a header row but nothing under it.", { kind: "info" });

  const types = await store.types();
  const settings = await store.settings();
  const m = modal({ title: "Import a spreadsheet", subtitle: String(path).split(/[\\/]/).pop(), size: "lg", body: h("div") });
  const body = m.dialog.querySelector(".modal-body");
  let mapping = [...inspected.mapping];
  let options = { ...inspected.options };
  const skip = new Set();

  // ---------------------------------------------------------------- columns
  function columnsStep() {
    const rows = inspected.headers.map((header, i) => {
      const current = mapping[i];
      const ownKey = current.startsWith("detail:") ? current.slice(7) : null;
      const detailKeys = [...new Set([ownKey, ...COMMON_DETAILS].filter(Boolean))];
      const choice = select([
        ["ignore", "Leave out"],
        ...inspected.fields.map((f) => [f, FIELD_LABELS[f] ?? f]),
        ...detailKeys.map((k) => [`detail:${k}`, `Detail: ${fmt.fieldLabel(k)}`]),
      ], current, { "aria-label": `What “${header}” holds` });
      choice.addEventListener("change", () => (mapping[i] = choice.value));
      const samples = inspected.samples.map((r) => r[i]).filter((v) => v && v.trim()).slice(0, 3);
      return h("tr", {},
        h("td", {}, h("strong", {}, header || h("span", { class: "muted" }, "(no header)"))),
        h("td", { class: "muted small sample-cell" }, samples.length ? samples.join(" · ") : "—"),
        h("td", {}, choice)
      );
    });

    const dateOrder = select(DATE_ORDERS, options.date_order);
    dateOrder.addEventListener("change", () => (options.date_order = dateOrder.value));
    const decimal = select([["false", "1,234.56 — a point"], ["true", "1.234,56 — a comma"]], String(options.decimal_comma));
    decimal.addEventListener("change", () => (options.decimal_comma = decimal.value === "true"));
    const currency = select(fmt.COMMON_CURRENCIES.includes(options.currency) ? fmt.COMMON_CURRENCIES.map((c) => [c, c]) : [[options.currency, options.currency], ...fmt.COMMON_CURRENCIES.map((c) => [c, c])], options.currency ?? settings.currency);
    currency.addEventListener("change", () => (options.currency = currency.value));
    const freeTypes = types.filter((t) => !["metals", "crypto"].includes(t.category));
    const defaultType = select(freeTypes.map((t) => [t.type_id, t.label]), options.default_type);
    defaultType.addEventListener("change", () => (options.default_type = defaultType.value));

    const next = h("button", { class: "btn btn-primary" }, "Preview rows");
    next.addEventListener("click", () => busy(next, async () => {
      if (!mapping.includes("name")) {
        toast("Choose the column that holds each item's name.", { kind: "warning" });
        return;
      }
      await previewStep();
    }, "Checking every row…"));

    mount(body,
      h("p", { class: "lede" }, `${inspected.row_count} row${inspected.row_count === 1 ? "" : "s"}. Say what each column holds — columns the app does not recognize are kept as details under their own name, so nothing is lost unless you leave it out.`),
      inspected.remembered ? callout("info", "Read the same way as the last file with these columns. Change anything that is wrong.") : null,
      h("table", { class: "table compact mapping-table" },
        h("thead", {}, h("tr", {}, h("th", {}, "Column"), h("th", {}, "First values"), h("th", {}, "Import as"))),
        h("tbody", {}, rows)
      ),
      h("div", { class: "form-grid", style: { marginTop: "16px" } },
        field("Dates are written", dateOrder),
        field("Decimal mark", decimal),
        field("Currency of amounts", currency, { hint: "Unless the file has a currency column." }),
        field("Type for rows without one", defaultType)
      ),
      h("div", { class: "form-actions" }, h("div", { class: "btn-row" }, h("button", { class: "btn btn-ghost", onclick: () => m.close() }, "Cancel"), next))
    );
  }

  // ---------------------------------------------------------------- preview
  async function previewStep() {
    const report = await call("import_spreadsheet", { contents, mapping, options, skip: [...skip], apply: false });
    const included = report.rows.filter((r) => r.status !== "skipped");
    const problems = report.rows.filter((r) => r.status === "error");

    const importButton = h("button", { class: "btn btn-primary", disabled: problems.length > 0 || report.ready === 0 },
      `Import ${report.ready} asset${report.ready === 1 ? "" : "s"}`);
    importButton.addEventListener("click", () => busy(importButton, async () => {
      const applied = await call("import_spreadsheet", { contents, mapping, options, skip: [...skip], apply: true });
      if (!applied.applied) {
        toast("Some rows still have problems — nothing was imported.", { kind: "warning" });
        return previewStep();
      }
      store.invalidate();
      m.close();
      toast(`Imported ${applied.ready} asset${applied.ready === 1 ? "" : "s"}${applied.skipped ? `; ${applied.skipped} left out` : ""}.`, { kind: "success", timeout: 7000 });
      onDone?.();
    }, "Importing…"));

    const rerun = async () => {
      try { await previewStep(); } catch (e) { toast(describe(e), { kind: "error" }); }
    };
    const skipProblems = problems.length
      ? h("button", { class: "btn btn-secondary", onclick: () => { for (const r of problems) skip.add(r.line - 2); rerun(); } }, `Leave out the ${problems.length} with problems`)
      : null;

    const rows = report.rows.map((r) => {
      const index = r.line - 2;
      const include = h("input", { type: "checkbox", checked: r.status !== "skipped", "aria-label": `Import line ${r.line}`, onchange: () => {
        if (include.checked) skip.delete(index);
        else skip.add(index);
        rerun();
      } });
      const issue = r.error
        ? h("span", { class: "field-error" }, r.error)
        : r.warnings.length ? h("span", { class: "warn-text" }, r.warnings.join("; ")) : null;
      return h("tr", { class: r.status === "skipped" ? "row-skipped" : r.status === "error" ? "row-error" : null },
        h("td", { class: "col-check" }, include),
        h("td", { class: "muted small" }, String(r.line)),
        h("td", {}, h("div", { class: "name-cell" }, h("span", { class: "name" }, r.name ?? "—"), h("span", { class: "sub" }, [r.type_label, r.location].filter(Boolean).join(" · ")), issue)),
        h("td", { class: "num" }, r.quantity ? fmt.quantity(r.quantity.split(" ")[0]) : "—"),
        h("td", { class: "num" }, r.paid ? fmt.money(r.paid) : "—"),
        h("td", { class: "num" }, r.value ? fmt.money(r.value) : "—")
      );
    });

    mount(body,
      h("p", { class: "lede" },
        `${report.ready} ready`,
        report.errors ? `, ${report.errors} with problems` : "",
        report.skipped ? `, ${report.skipped} left out` : "",
        report.warnings ? `, ${report.warnings} worth a look` : "",
        ". Rows are imported together — all of them, or none."),
      problems.length ? callout("warning", "Fix these in the file, choose different columns, or leave them out.") : null,
      h("table", { class: "table compact" },
        h("thead", {}, h("tr", {}, h("th", { class: "col-check" }, h("span", { class: "sr-only" }, "Import")), h("th", {}, "Line"), h("th", {}, "As it will be stored"), h("th", { class: "num" }, "Qty"), h("th", { class: "num" }, "Paid"), h("th", { class: "num" }, "Value"))),
        h("tbody", {}, rows)
      ),
      h("div", { class: "form-actions" },
        included.length ? null : h("span", { class: "muted" }, "Every row is left out."),
        h("div", { class: "btn-row" },
          h("button", { class: "btn btn-ghost", onclick: () => columnsStep() }, icon("back", { size: 16 }), "Columns"),
          skipProblems,
          importButton
        )
      )
    );
  }

  columnsStep();
  return m.done;
}
