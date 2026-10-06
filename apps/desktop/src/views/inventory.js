// Physical inventory checks and QR labels.
//
// A check walks a place and confirms each thing is there. It is saved as it
// goes and never changes the catalog by itself: at the end, each discrepancy
// is shown with what to do about it, and nothing happens unless chosen.
//
// Labels encode only the asset's random ID. A USB barcode scanner types
// what it reads, so scanning a label into the check's box marks it present
// — no camera, no network.

import QRCode from "qrcode";

import { call, describe } from "../lib/api.js";
import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { modal, field, select, busy, toast, confirmDialog, callout, toggle } from "../ui/components.js";
import { runPrint } from "./onboarding.js";

/** Print QR labels for assets: the opaque code, a short code, and optionally the name. */
export async function printLabels(assetIds) {
  let withNames = true;
  const m = modal({
    title: `Print ${assetIds.length} label${assetIds.length === 1 ? "" : "s"}`,
    size: "sm",
    body: h("div", { class: "stack" },
      h("p", {}, "Each label's code identifies the item only to this vault — it holds no value, location or serial number. Scan it during an inventory check, or type the short code."),
      toggle("Print the item's name under the code", true, (on) => (withNames = on))
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Cancel"), h("button", { class: "btn btn-primary", onclick: () => close(true) }, icon("printer", { size: 16 }), "Print")],
  });
  if (!(await m.done)) return;
  try {
    const labels = await call("label_data", { assetIds });
    const cells = await Promise.all(labels.map(async (l) => {
      const canvas = h("canvas");
      await QRCode.toCanvas(canvas, l.payload, { errorCorrectionLevel: "M", margin: 1, width: 120 });
      return h("div", { class: "label-cell" },
        h("img", { src: canvas.toDataURL("image/png"), alt: "" }),
        h("div", { class: "label-text" }, h("strong", {}, l.short_code), withNames ? h("span", {}, l.name) : null));
    }));
    mount(document.getElementById("print-root"), h("div", { class: "print-sheet label-sheet" }, cells));
    runPrint(`Asset-Manager-Labels-${fmt.todayIso()}`);
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

/** The list of checks: start one, resume an open one, review a finished one. */
export async function openInventory(ctx) {
  const m = modal({ title: "Inventory checks", size: "lg", body: h("div") });
  const body = m.dialog.querySelector(".modal-body");

  async function listStep() {
    const [checks, locations] = await Promise.all([call("list_checks"), call("list_locations")]);
    const where = select([["", "Everything held"], ...locations.map((l) => [l.name, l.name])], "");
    const name = h("input", { type: "text", maxlength: 120, value: `Check ${fmt.date(fmt.todayIso())}` });
    const start = h("button", { class: "btn btn-primary" }, "Start");
    start.addEventListener("click", () => busy(start, async () => {
      const id = await call("start_check", { name: name.value, location: where.value || null });
      await checkStep(id);
    }));
    mount(body,
      h("p", { class: "lede" }, "Walk a room, a safe or a storage unit and confirm each thing is there. A check is saved as you go; finish it any time."),
      h("div", { class: "form-grid" }, field("Where", where, { hint: "A place includes the places inside it." }), field("Name", name)),
      h("div", { class: "btn-row", style: { marginTop: "10px" } }, start),
      checks.length
        ? h("table", { class: "table compact", style: { marginTop: "18px" } }, h("tbody", {}, checks.map((c) => h("tr", {},
            h("td", {}, h("div", { class: "name-cell" }, h("span", {}, c.name), h("span", { class: "sub" }, `${c.scope_location ?? "Everything"} · started ${fmt.date(c.started_at)}`))),
            h("td", { class: "num muted small" }, `${c.marked} of ${c.expected}`),
            h("td", {}, c.finished_at ? h("span", { class: "badge badge-muted" }, `Finished ${fmt.date(c.finished_at)}`) : h("span", { class: "badge badge-attention" }, "Open")),
            h("td", { class: "row-action" },
              h("button", { class: "btn btn-ghost btn-sm", onclick: () => (c.finished_at ? reviewStep(c.check_id) : checkStep(c.check_id)) }, c.finished_at ? "Review" : "Resume"),
              h("button", { class: "icon-btn", "aria-label": `Delete ${c.name}`, onclick: async () => {
                if (!(await confirmDialog({ title: "Delete this check?", message: "Its marks — and the “last seen” dates they set — are removed. Nothing else changes.", confirmLabel: "Delete", danger: true }))) return;
                await call("delete_check", { checkId: c.check_id });
                listStep();
              } }, icon("trash", { size: 14 })))
          )))) : null
    );
  }

  async function checkStep(checkId) {
    const items = await call("check_items", { checkId });
    const done = items.filter((i) => i.result).length;
    const scan = h("input", { type: "text", class: "input-lg", placeholder: "Scan a label, or type its short code, then Enter", autocomplete: "off", "aria-label": "Scan a label" });
    const feedback = h("p", { class: "field-hint", "aria-live": "polite" });
    const mark = async (assetId, result, counted = null) => {
      try {
        await call("mark_item", { checkId, assetId, result, counted });
        await checkStep(checkId);
      } catch (e) { toast(describe(e), { kind: "error" }); }
    };
    scan.addEventListener("keydown", async (e) => {
      if (e.key !== "Enter" || !scan.value.trim()) return;
      e.preventDefault();
      const code = scan.value;
      scan.value = "";
      const id = await call("resolve_label", { code }).catch(() => null);
      const item = items.find((i) => i.asset_id === id);
      if (!id) return void (feedback.textContent = `No item has the code “${code.trim()}”.`);
      if (!item) return void (feedback.textContent = "That item is not part of this check — it is recorded somewhere else.");
      await mark(id, "present");
    });
    const rows = items.map((i) => {
      const count = h("input", { type: "text", inputmode: "decimal", class: "input-sm", value: i.counted ?? "", placeholder: fmt.quantity(i.quantity), "aria-label": `Count of ${i.name}` });
      const btn = (result, label, extra) => h("button", { class: i.result === result ? "btn btn-primary btn-sm" : "btn btn-secondary btn-sm", "aria-pressed": String(i.result === result), onclick: () => mark(i.asset_id, i.result === result ? null : result, extra?.()) }, label);
      return h("tr", { class: i.result ? "checked-row" : null },
        h("td", {}, h("div", { class: "name-cell" },
          h("span", {}, i.name),
          h("span", { class: "sub" }, [i.storage_location, `${fmt.quantity(i.quantity)} ${i.quantity_unit}`, i.away ? `${fmt.CUSTODY_LABELS[i.away]} — not expected here` : null].filter(Boolean).join(" · ")))),
        h("td", { class: "row-action" }, h("div", { class: "btn-row" },
          btn("present", "Here"),
          btn("missing", "Missing"),
          h("span", { class: "count-mark" }, count, btn("count", "Count", () => count.value))
        ))
      );
    });
    mount(body,
      h("div", { class: "check-progress" }, h("strong", {}, `${done} of ${items.length} checked`), h("progress", { max: items.length || 1, value: done })),
      scan, feedback,
      h("table", { class: "table compact" }, h("tbody", {}, rows)),
      h("div", { class: "form-actions" }, h("div", { class: "btn-row" },
        h("button", { class: "btn btn-ghost", onclick: () => listStep() }, icon("back", { size: 16 }), "All checks"),
        h("button", { class: "btn btn-primary", onclick: async () => {
          const unchecked = items.length - done;
          if (unchecked && !(await confirmDialog({ title: "Finish with items unchecked?", message: `${unchecked} item${unchecked === 1 ? " was" : "s were"} not checked. They are listed in the review as not checked — not as missing.`, confirmLabel: "Finish" }))) return;
          await call("finish_check", { checkId });
          reviewStep(checkId);
        } }, "Finish and review")
      ))
    );
    requestAnimationFrame(() => scan.focus());
  }

  /** What the check found that differs from the catalog, and what to do. */
  async function reviewStep(checkId) {
    const items = await call("check_items", { checkId });
    const missing = items.filter((i) => i.result === "missing");
    const counts = items.filter((i) => i.result === "count" && i.counted !== i.quantity);
    const unchecked = items.filter((i) => !i.result);
    const present = items.length - missing.length - counts.length - unchecked.length;
    const open = (i) => { m.close(); ctx.navigate("asset", { id: i.asset_id }); };
    const correct = async (i, button) => busy(button, async () => {
      await call("reconcile_check_count", { checkId, assetId: i.asset_id });
      store.invalidate();
      toast(`${i.name}: count corrected to ${fmt.quantity(i.counted)}.`, { kind: "success" });
      reviewStep(checkId);
    });
    const section = (title, list, action) => list.length
      ? h("section", { class: "stack" }, h("h3", {}, `${title} · ${list.length}`),
          h("table", { class: "table compact" }, h("tbody", {}, list.map((i) => h("tr", {},
            h("td", {}, h("div", { class: "name-cell" }, h("span", {}, i.name), h("span", { class: "sub" }, [i.storage_location, i.result === "count" ? `recorded ${fmt.quantity(i.quantity)}, counted ${fmt.quantity(i.counted)}` : null].filter(Boolean).join(" · ")))),
            h("td", { class: "row-action" }, action(i))
          )))))
      : null;
    mount(body,
      callout(missing.length || counts.length ? "warning" : "success",
        `${present} found as recorded. `, missing.length ? `${missing.length} missing. ` : "", counts.length ? `${counts.length} with a different count. ` : "", unchecked.length ? `${unchecked.length} not checked.` : ""),
      h("p", { class: "field-hint" }, "These are the observations saved with this check. A missing item may only be misplaced."),
      section("Missing", missing, (i) => h("button", { class: "btn btn-secondary btn-sm", onclick: () => open(i) }, "Open — mark lost or note it")),
      section("Different count", counts, (i) => { if (!i.can_correct) return h("span", { class: "muted" }, i.reconciled ? "Reconciled" : "Holding changed or original baseline unavailable — start a new check"); const b = h("button", { class: "btn btn-secondary btn-sm" }, `Correct to ${fmt.quantity(i.counted)}`); b.addEventListener("click", () => correct(i, b)); return b; }),
      section("Not checked", unchecked, (i) => h("button", { class: "btn btn-ghost btn-sm", onclick: () => open(i) }, "Open")),
      h("div", { class: "form-actions" }, h("div", { class: "btn-row" }, h("button", { class: "btn btn-ghost", onclick: () => listStep() }, icon("back", { size: 16 }), "All checks")))
    );
  }

  listStep().catch((e) => toast(describe(e), { kind: "error" }));
  await m.done;
  store.invalidate();
  ctx.refresh();
}
