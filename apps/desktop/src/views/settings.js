// Settings: security, backup, price feeds, privacy, display.

import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { call, describe } from "../lib/api.js";
import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { busy, toast, modal, field, select, toggle, callout, confirmDialog } from "../ui/components.js";
import { recoveryCeremony, restoreDialog, restoredMessage, runPrint } from "./onboarding.js";

const CURRENCIES = [
  ["USD", "US dollar (USD)"], ["EUR", "Euro (EUR)"], ["GBP", "British pound (GBP)"], ["CAD", "Canadian dollar (CAD)"],
  ["AUD", "Australian dollar (AUD)"], ["CHF", "Swiss franc (CHF)"], ["JPY", "Japanese yen (JPY)"], ["NZD", "New Zealand dollar (NZD)"],
  ["SEK", "Swedish krona (SEK)"], ["NOK", "Norwegian krone (NOK)"], ["DKK", "Danish krone (DKK)"], ["SGD", "Singapore dollar (SGD)"],
];

export async function renderSettings(root, params, ctx) {
  const [settings, info, metals, crypto, trash, centre] = await Promise.all([
    store.settings({ fresh: true }),
    call("vault_info"),
    call("metals_provider_status"),
    call("crypto_provider_status"),
    call("list_trash"),
    call("backup_centre"),
  ]);

  const save = async (patch) => {
    const next = { ...settings, ...patch };
    try {
      await call("update_settings", { settings: next });
      Object.assign(settings, patch);
      store.setSettings({ ...settings });
      toast("Saved.", { kind: "success", timeout: 1800 });
    } catch (e) {
      toast(describe(e), { kind: "error" });
      ctx.refresh();
    }
  };

  mount(
    root,
    h("header", { class: "page-head" }, h("div", {}, h("h1", {}, "Settings"), h("p", { class: "page-sub" }, "Stored inside this vault, so they travel with it"))),
    securityCard(settings, info, save, ctx),
    backupCard(settings, centre, ctx),
    trashCard(trash, ctx),
    emergencyCard(info, centre),
    feedsCard(settings, metals, crypto, save, ctx),
    privacyCard(settings, save),
    displayCard(settings, save),
    aboutCard(info)
  );

  if (params.section) root.querySelector(`#${params.section}`)?.scrollIntoView({ block: "start" });
}

function row(title, description, control) {
  return h("div", { class: "setting" },
    h("div", { class: "setting-text" }, h("strong", {}, title), description ? h("p", {}, description) : null),
    h("div", { class: "setting-control" }, control)
  );
}

// ------------------------------------------------------------ security

function securityCard(settings, info, save, ctx) {
  const autoLock = select(
    [["5", "5 minutes"], ["10", "10 minutes"], ["15", "15 minutes"], ["30", "30 minutes"], ["60", "1 hour"], ["120", "2 hours"]],
    String(settings.auto_lock_minutes)
  );
  if (autoLock.value !== String(settings.auto_lock_minutes)) {
    autoLock.append(h("option", { value: String(settings.auto_lock_minutes) }, `${settings.auto_lock_minutes} minutes`));
    autoLock.value = String(settings.auto_lock_minutes);
  }
  autoLock.addEventListener("change", () => save({ auto_lock_minutes: Number(autoLock.value) }));

  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, icon("shield", { size: 18 }), " Security")),
    row("Passphrase", "Changes what opens this vault from now on. Older backups still open with the passphrase they were made with.",
      h("button", { class: "btn btn-secondary", onclick: () => changePassphrase() }, icon("key", { size: 16 }), "Change passphrase")),
    row("Recovery key", `Current key fingerprint ${info.recovery_fingerprint}. Issue a new key if the printed sheet may have been seen.`,
      h("button", { class: "btn btn-secondary", onclick: () => rotateRecovery() }, icon("refresh", { size: 16 }), "New recovery key")),
    row("Lock after inactivity", "Everything decrypted is dropped from memory and the screen when the vault locks.", autoLock),
    row("Lock now", null, h("button", { class: "btn btn-secondary", onclick: () => ctx.lock() }, icon("lock", { size: 16 }), "Lock vault"))
  );
}

function passField(label, id, autocomplete) {
  const input = h("input", { type: "password", id, autocomplete, spellcheck: "false" });
  return { input, el: field(label, input) };
}

function changePassphrase() {
  const current = passField("Current passphrase", "cp-current", "current-password");
  const next = passField("New passphrase", "cp-new", "new-password");
  const again = passField("Confirm new passphrase", "cp-again", "new-password");
  const error = h("p", { class: "form-error", role: "alert" });
  const submit = h("button", { class: "btn btn-primary", type: "submit", form: "cp-form" }, "Change passphrase");
  const m = modal({
    title: "Change passphrase",
    size: "sm",
    body: h("form", { id: "cp-form", class: "stack", onsubmit: async (e) => {
      e.preventDefault();
      error.textContent = "";
      if ([...next.input.value].length < 12) return (error.textContent = "Use at least 12 characters.");
      if (next.input.value !== again.input.value) return (error.textContent = "The new passphrases do not match.");
      await busy(submit, async () => {
        try {
          await call("change_passphrase", { current: current.input.value, newPassphrase: next.input.value });
          for (const f of [current, next, again]) f.input.value = "";
          m.close(true);
          toast("Passphrase changed. Make a fresh backup so it opens with the new one too.", { kind: "success", timeout: 7000 });
        } catch (err) {
          error.textContent = describe(err);
        }
      }, "Re-wrapping key…");
    } },
      current.el, next.el, again.el,
      callout("info", "Your photos and records are not re-encrypted — only the key that unlocks them is re-wrapped, so this takes a second, not an hour."),
      error
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), submit],
  });
}

export function rotateRecovery() {
  const pass = passField("Your passphrase", "rr-pass", "current-password");
  const error = h("p", { class: "form-error", role: "alert" });
  const submit = h("button", { class: "btn btn-primary", type: "submit", form: "rr-form" }, "Issue new key");
  const m = modal({
    title: "New recovery key",
    size: "md",
    body: h("form", { id: "rr-form", class: "stack", onsubmit: async (e) => {
      e.preventDefault();
      error.textContent = "";
      await busy(submit, async () => {
        try {
          const created = await call("rotate_recovery_key", { passphrase: pass.input.value });
          pass.input.value = "";
          m.close(true);
          // The ceremony cannot be dismissed by accident: the old key is
          // already gone, so this one must be saved.
          const ceremony = modal({ title: "Save your new recovery key", size: "md", dismissable: false, body: (close) => recoveryCeremony(created, { context: "rotate", onDone: () => close(true) }) });
          await ceremony.done;
          toast("New recovery key in place. Destroy the old sheet.", { kind: "success" });
        } catch (err) {
          error.textContent = describe(err);
        }
      }, "Generating…");
    } },
      h("p", {}, "The current recovery key stops opening this vault immediately. You will be shown the new key once, and must confirm you have saved it."),
      callout("warning", "Backups made before now still open with the old key. Make a fresh backup afterwards, and destroy old backups you no longer trust."),
      pass.el,
      error
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), submit],
  });
}

// ------------------------------------------------------------ backup

/**
 * The backup centre: where backups go, how many to keep, when to be
 * reminded, and — the line that matters — when a backup was last proven to
 * restore.
 */
function backupCard(settings, centre, ctx) {
  const last = centre.last_backup_at;
  const days = last ? fmt.daysSince(last) : null;
  const overdue = centre.reminder_days > 0 && (days === null || days > centre.reminder_days);
  const verifiedDays = centre.last_verified_at ? fmt.daysSince(centre.last_verified_at) : null;

  const runBackup = async (button, directory) => {
    await busy(button, async () => {
      const result = await call("backup_vault", { directory: directory ?? null });
      store.setSettings(null);
      const pruned = result.pruned.length ? ` Removed ${result.pruned.length} older backup${result.pruned.length === 1 ? "" : "s"} under your keep rule.` : "";
      toast(`Backup saved to ${result.path} (${result.objects} file${result.objects === 1 ? "" : "s"}).${pruned} Verify it to be sure it restores.`, { kind: "success", timeout: 10000 });
      ctx.refresh();
    }, "Backing up…");
  };
  const chooseFolder = async () => openDialog({ directory: true, title: "Choose where to put backups" });

  const backupNow = h("button", { class: "btn btn-primary", onclick: async () => {
    if (centre.folder && centre.folder_available) return runBackup(backupNow, null);
    const directory = await chooseFolder();
    if (directory) runBackup(backupNow, directory);
  } }, icon("archive", { size: 16 }), centre.folder && centre.folder_available ? "Back up now" : "Back up now…");
  const backupElsewhere = h("button", { class: "btn btn-secondary", onclick: async () => {
    const directory = await chooseFolder();
    if (directory) runBackup(backupElsewhere, directory);
  } }, "Back up to another folder…");
  const verify = h("button", { class: "btn btn-secondary", onclick: () => verifyDialog(centre, ctx) }, icon("check", { size: 16 }), "Verify a backup…");

  const restore = h("button", { class: "btn btn-ghost", onclick: async () => {
    const restored = await restoreDialog({ replacing: true });
    if (restored?.opened) {
      ctx.reopen();
      toast(restoredMessage(restored), { kind: "success", timeout: 7000 });
      return;
    }
    const state = await call("session_state").catch(() => null);
    if (state && !state.unlocked) ctx.lock();
  } }, icon("upload", { size: 16 }), "Restore…");

  // Preferences.
  const savePrefs = async (patch) => {
    try {
      await call("update_backup_preferences", { preferences: { folder: centre.folder ?? null, keep: centre.keep, reminder_days: centre.reminder_days, ...patch } });
      store.setSettings(null);
      toast("Saved.", { kind: "success", timeout: 1800 });
      ctx.refresh();
    } catch (e) {
      toast(describe(e), { kind: "error" });
    }
  };
  const keep = select([["0", "Keep every backup"], ["3", "The newest 3"], ["5", "The newest 5"], ["10", "The newest 10"], ["20", "The newest 20"]], String(centre.keep));
  if (keep.value !== String(centre.keep)) {
    keep.append(h("option", { value: String(centre.keep) }, `The newest ${centre.keep}`));
    keep.value = String(centre.keep);
  }
  keep.addEventListener("change", () => savePrefs({ keep: Number(keep.value) }));
  const reminder = select([["0", "Never"], ["7", "After a week"], ["14", "After two weeks"], ["30", "After a month"], ["90", "After three months"]], String(centre.reminder_days));
  if (reminder.value !== String(centre.reminder_days)) {
    reminder.append(h("option", { value: String(centre.reminder_days) }, `After ${centre.reminder_days} days`));
    reminder.value = String(centre.reminder_days);
  }
  reminder.addEventListener("change", () => savePrefs({ reminder_days: Number(reminder.value) }));
  const changeFolder = h("button", { class: "btn btn-secondary btn-sm", onclick: async () => {
    const directory = await chooseFolder();
    if (directory) savePrefs({ folder: directory });
  } }, centre.folder ? "Change…" : "Choose…");

  const history = centre.history.slice(0, 12).map((b) => {
    const state = b.pruned ? h("span", { class: "badge badge-muted" }, "Removed (keep rule)")
      : !b.present ? h("span", { class: "badge badge-muted" }, "Not found — drive unplugged?")
      : b.verified_ok === true ? h("span", { class: "badge badge-fresh" }, icon("check", { size: 12 }), `Verified ${fmt.ago(b.verified_at)}`)
      : b.verified_ok === false ? h("span", { class: "badge badge-attention", title: b.verify_note ?? "" }, icon("warn", { size: 12 }), `Failed: ${b.verify_note ?? "did not restore"}`)
      : h("span", { class: "badge badge-muted" }, "Not verified");
    return h("tr", {},
      h("td", {}, fmt.date(b.created_at), h("div", { class: "muted small path-cell", title: b.path }, b.path)),
      h("td", { class: "num muted small" }, `${b.objects} file${b.objects === 1 ? "" : "s"}`),
      h("td", {}, state),
      h("td", { class: "row-action" }, b.present && !b.pruned ? h("button", { class: "btn btn-ghost btn-sm", onclick: () => verifyDialog(centre, ctx, b.path) }, "Verify") : null)
    );
  });

  return h("section", { class: "card", id: "backup" },
    h("div", { class: "card-head" },
      h("h2", {}, icon("archive", { size: 18 }), " Backups"),
      h("div", { class: "btn-row" },
        last
          ? h("span", { class: overdue ? "badge badge-attention" : "badge badge-fresh" }, icon(overdue ? "warn" : "check", { size: 12 }), `Last backup ${fmt.ago(last)}`)
          : h("span", { class: "badge badge-attention" }, icon("warn", { size: 12 }), "Never backed up"),
        centre.last_verified_at
          ? h("span", { class: verifiedDays > 90 ? "badge badge-attention" : "badge badge-fresh" }, `Last verified ${fmt.ago(centre.last_verified_at)}`)
          : h("span", { class: "badge badge-muted" }, "Never verified")
      )
    ),
    h("p", { class: "card-text" }, "A backup is a complete, still-encrypted copy of the vault — records, photos, documents and history. It is safe on an external drive or in a synced folder: without your passphrase or recovery key it is unreadable. Verifying one restores it into a scratch folder, decrypts every file, and throws the copy away — the only proof it will open on a new computer."),
    h("div", { class: "btn-row" }, backupNow, backupElsewhere, verify, restore),
    h("div", { class: "setting" },
      h("div", { class: "setting-text" }, h("strong", {}, "Backup folder"), h("p", {}, centre.folder ? h("code", { class: "path" }, centre.folder, centre.folder_available ? "" : " — not available now") : "Not chosen yet. An external drive or a synced folder, away from this computer, is best.")),
      h("div", { class: "setting-control" }, changeFolder)
    ),
    row("Keep in that folder", "Older backups of this vault are removed after each new one. Nothing else in the folder is touched.", keep),
    row("Remind me to back up", "Shown on the overview while the app is open. Nothing runs when the app is closed.", reminder),
    history.length
      ? h("table", { class: "table compact backup-history" }, h("tbody", {}, history))
      : null,
    h("details", { class: "disclosure" },
      h("summary", {}, "Moving to a new computer"),
      h("ol", { class: "rules" },
        h("li", {}, "Make a backup, and verify it."),
        h("li", {}, "Copy the backup folder to the new computer, or plug in the drive it is on."),
        h("li", {}, "Install Asset Manager there and choose “Restore from a backup”."),
        h("li", {}, "Give the passphrase or recovery key the backup was made with. The restore checks every file before the vault opens.")
      )
    )
  );
}

/** Prove a backup restores: the credential it opens with, then a full check. */
function verifyDialog(centre, ctx, presetPath = null) {
  let directory = presetPath ?? centre.history.find((b) => b.present && !b.pruned)?.path ?? null;
  let useRecovery = false;
  const folderLabel = h("code", { class: "path" }, directory ?? "No folder chosen");
  const choose = h("button", { type: "button", class: "btn btn-secondary btn-sm", onclick: async () => {
    const picked = await openDialog({ directory: true, title: "Choose a backup folder" });
    if (picked) { directory = picked; folderLabel.textContent = picked; }
  } }, "Choose another…");
  const secret = passField("Passphrase the backup opens with", "verify-secret", "off");
  const label = secret.el.querySelector("label");
  const switcher = h("button", { type: "button", class: "link", onclick: () => {
    useRecovery = !useRecovery;
    label.textContent = useRecovery ? "Recovery key the backup opens with" : "Passphrase the backup opens with";
    switcher.textContent = useRecovery ? "Use the passphrase instead" : "Use the recovery key instead";
    secret.input.value = "";
  } }, "Use the recovery key instead");
  const result = h("div", { "aria-live": "polite" });
  const run = h("button", { class: "btn btn-primary", type: "submit", form: "verify-form" }, "Verify");
  modal({
    title: "Verify a backup",
    size: "md",
    body: h("form", { id: "verify-form", class: "stack", onsubmit: async (e) => {
      e.preventDefault();
      if (!directory) return mount(result, h("p", { class: "form-error" }, "Choose the backup folder."));
      mount(result);
      await busy(run, async () => {
        try {
          const v = await call("verify_backup", { directory, secret: secret.input.value, useRecoveryKey: useRecovery });
          secret.input.value = "";
          mount(result, callout("success",
            h("strong", {}, "This backup restores. "),
            `Made ${fmt.date(v.created_at)}: ${v.assets} asset${v.assets === 1 ? "" : "s"}, ${v.decrypted} of ${v.objects} files decrypted and intact.`,
            v.this_vault ? null : " It is a backup of a different vault than the one open now."
          ));
        } catch (err) {
          mount(result, callout("danger", h("strong", {}, "This backup did not verify. "), describe(err)));
        }
        ctx.refresh();
      }, "Restoring into a scratch folder…");
    } },
      h("p", {}, "The backup is restored into a scratch folder, every photo and document is decrypted, and the copy is removed. Your vault is not touched."),
      h("div", { class: "field" }, h("label", {}, "Backup"), h("div", { class: "btn-row" }, folderLabel, choose)),
      secret.el,
      h("div", { class: "form-links" }, switcher),
      result
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", type: "button", onclick: () => close() }, "Close"), run],
  });
}

// ------------------------------------------------------------ emergency access

/**
 * A sheet for someone you trust: what this is, where the vault and its
 * backups are, and how to open it — but never the passphrase or recovery
 * key. Possession of those is what opens the vault, so they stay on their
 * own sheet, kept elsewhere; this one says where.
 */
function emergencyCard(info, centre) {
  return h("section", { class: "card", id: "emergency" },
    h("div", { class: "card-head" }, h("h2", {}, icon("key", { size: 18 }), " Emergency access")),
    h("p", { class: "card-text" }, "If something happened to you, could someone you trust find and open this catalog? Print instructions for them: where the vault and its backups are, how to restore it on another computer, and where you keep the recovery key. The sheet holds no passphrase or key — on its own it opens nothing."),
    h("div", { class: "btn-row" }, h("button", { class: "btn btn-secondary", onclick: () => emergencyDialog(info, centre) }, icon("printer", { size: 16 }), "Print an access sheet…"))
  );
}

function emergencyDialog(info, centre) {
  const fields = {
    for_whom: h("input", { type: "text", maxlength: 200, placeholder: "e.g. Sam, my executor" }),
    recovery_kept: h("input", { type: "text", maxlength: 500, placeholder: "e.g. in the fire safe, envelope marked “Asset Manager”" }),
    backups_kept: h("input", { type: "text", maxlength: 500, value: centre.folder ? `On the drive or folder: ${centre.folder}` : "" }),
    contact: h("input", { type: "text", maxlength: 500, placeholder: "e.g. my attorney, Jane Doe, 555-0100" }),
    notes: h("textarea", { rows: 3, maxlength: 2000, placeholder: "Anything else they should know" }),
  };
  const print = h("button", { class: "btn btn-primary" }, icon("printer", { size: 16 }), "Print");
  const m = modal({
    title: "Emergency access sheet",
    size: "md",
    body: h("div", { class: "stack" },
      callout("info", "Keep this sheet apart from your recovery key sheet. Together they open the vault; this one alone does not. What you type here is printed, not saved."),
      h("div", { class: "form-grid" },
        field("For", fields.for_whom, { span: 2 }),
        field("Where the recovery key sheet is kept", fields.recovery_kept, { span: 2 }),
        field("Where backups are kept", fields.backups_kept, { span: 2 }),
        field("Who else can help", fields.contact, { span: 2 }),
        field("Anything else", fields.notes, { span: 2 })
      )
    ),
    footer: (close) => [h("button", { class: "btn btn-ghost", onclick: () => close() }, "Cancel"), print],
  });
  print.addEventListener("click", () => {
    const v = Object.fromEntries(Object.entries(fields).map(([k, el]) => [k, el.value.trim()]));
    printEmergencySheet(info, centre, v);
    m.close();
  });
}

function printEmergencySheet(info, centre, v) {
  const line = (label, value) => (value ? [h("dt", {}, label), h("dd", {}, value)] : null);
  mount(document.getElementById("print-root"),
    h("div", { class: "print-sheet emergency-sheet" },
      h("h1", {}, "Asset Manager — emergency access"),
      v.for_whom ? h("p", {}, h("strong", {}, "For: "), v.for_whom) : null,
      h("p", {}, "This explains how to open an encrypted catalog of valuables kept in the Asset Manager desktop app — what is owned, what it is worth, and the receipts and photos that prove it. It is useful for insurance, an estate, or replacing what was lost."),
      h("h2", {}, "What you need"),
      h("ol", {},
        h("li", {}, "The Asset Manager app, installed on any computer."),
        h("li", {}, "The vault — either this computer, or a backup (below)."),
        h("li", {}, "The ", h("strong", {}, "recovery key"), ", a printed sheet of letters and numbers. ", v.recovery_kept ? `It is kept: ${v.recovery_kept}.` : "Its location is not written here."),
      ),
      h("p", {}, "This sheet does not contain the recovery key or the passphrase. Without one of them the catalog cannot be opened by anyone — there is no reset and no company that can help."),
      h("h2", {}, "Where things are"),
      h("dl", { class: "kv" },
        line("Vault on this computer", info.location),
        line("Created", fmt.date(info.created_at)),
        line("Recovery key fingerprint", info.recovery_fingerprint),
        line("Backups", v.backups_kept || centre.folder),
        line("Last backup", centre.last_backup_at ? fmt.date(centre.last_backup_at) : "none recorded"),
        line("Last backup proven to restore", centre.last_verified_at ? fmt.date(centre.last_verified_at) : "none recorded"),
        line("Who else can help", v.contact)
      ),
      h("p", { class: "muted" }, "The fingerprint identifies which recovery key sheet belongs to this vault. It is printed on that sheet too, and reveals nothing about the key."),
      h("h2", {}, "How to open it"),
      h("ol", {},
        h("li", {}, "On this computer: open Asset Manager and choose “Use the recovery key instead”. Type the key from the recovery sheet."),
        h("li", {}, "On another computer: install Asset Manager, choose “Restore from a backup”, pick the backup folder, and give the recovery key. Every file is checked before the catalog opens."),
        h("li", {}, "Once open, Reports → “Prepare a claim” or “Insurance inventory” produce documents for an insurer. Settings → Backups can make a fresh copy.")
      ),
      v.notes ? [h("h2", {}, "Also"), h("p", { class: "pre-wrap" }, v.notes)] : null,
      h("p", { class: "print-meta" }, `Prepared ${new Date().toLocaleDateString()}. Reprint it after moving the vault or its backups.`)
    )
  );
  runPrint(`Asset-Manager-Emergency-Access-${fmt.todayIso()}`);
}

// ------------------------------------------------------------ trash

function trashCard(items, ctx) {
  const purge = async (assetId, name) => {
    const ok = await confirmDialog({
      title: assetId ? `Delete “${name}” for good?` : "Empty the trash?",
      message: assetId
        ? "Its record, history, and any photos or documents nothing else uses are removed from the vault. This cannot be undone — though backups made before now still contain it."
        : `All ${items.length} item${items.length === 1 ? "" : "s"} in the trash are removed from the vault for good. This cannot be undone — though backups made before now still contain them.`,
      confirmLabel: assetId ? "Delete for good" : "Empty trash",
      danger: true,
    });
    if (!ok) return;
    try {
      await call("purge_trash", { assetId: assetId ?? null });
      toast("Deleted for good.", { kind: "success" });
      ctx.refresh();
    } catch (e) {
      toast(describe(e), { kind: "error" });
    }
  };
  const restore = async (item) => {
    try {
      await call("restore_asset", { assetId: item.asset_id });
      store.invalidate();
      toast(`Restored ${item.name}.`, { kind: "success" });
      ctx.refresh();
    } catch (e) {
      toast(describe(e), { kind: "error" });
    }
  };
  return h("section", { class: "card", id: "trash" },
    h("div", { class: "card-head" },
      h("h2", {}, icon("trash", { size: 18 }), " Trash"),
      items.length ? h("button", { class: "btn btn-ghost btn-sm", onclick: () => purge(null) }, "Empty trash") : null
    ),
    h("p", { class: "card-text" }, "Deleted assets wait here for 30 days with their history, photos and documents, then are removed for good. Until then they are still inside the encrypted vault."),
    items.length
      ? h("table", { class: "table compact" },
          h("tbody", {}, items.map((item) => h("tr", {},
            h("td", {}, h("div", { class: "name-cell" }, h("span", {}, item.name), h("span", { class: "sub" }, item.type_label))),
            h("td", { class: "muted small" }, `Deleted ${fmt.ago(item.deleted_at)}${item.purge_on ? ` · removed ${fmt.date(item.purge_on)}` : ""}`),
            h("td", { class: "num" }, h("div", { class: "btn-row", style: { justifyContent: "flex-end" } },
              h("button", { class: "btn btn-secondary btn-sm", onclick: () => restore(item) }, "Restore"),
              h("button", { class: "btn btn-ghost btn-sm", onclick: () => purge(item.asset_id, item.name) }, "Delete for good")
            ))
          )))
        )
      : h("p", { class: "muted small" }, "The trash is empty.")
  );
}

// ------------------------------------------------------------ feeds

function keyRow(title, description, configured, command, onSaved) {
  const input = h("input", { type: "password", autocomplete: "off", spellcheck: "false", placeholder: configured ? "Key saved — enter a new one to replace it" : "Paste API key", class: "input-key" });
  const saveBtn = h("button", { class: "btn btn-secondary", onclick: () => busy(saveBtn, async () => {
    if (!input.value.trim()) return;
    await call(command, { key: input.value });
    input.value = ""; // never leave a key in the DOM
    toast("Key saved to your system keyring.", { kind: "success" });
    onSaved();
  }) }, "Save");
  const remove = configured ? h("button", { class: "btn btn-ghost", onclick: () => busy(remove, async () => {
    await call(command, { key: "" });
    toast("Key removed.", { kind: "success" });
    onSaved();
  }) }, "Remove") : null;
  return h("div", { class: "setting setting-stack" },
    h("div", { class: "setting-text" },
      h("strong", {}, title, " ", configured ? h("span", { class: "badge badge-fresh" }, icon("check", { size: 12 }), "Configured") : h("span", { class: "badge badge-muted" }, "Not set")),
      h("p", {}, description)
    ),
    h("div", { class: "key-form" }, input, saveBtn, remove)
  );
}

function feedsCard(settings, metals, crypto, save, ctx) {
  return h("section", { class: "card", id: "feeds" },
    h("div", { class: "card-head" }, h("h2", {}, icon("markets", { size: 18 }), " Price feeds")),
    h("p", { class: "card-text" }, "Optional. Everything works with prices typed by hand. Keys are kept in your system keyring, never in the vault or a file. Asking a provider for prices tells it which metals or coins you hold."),
    keyRow("metals.dev", "Gold, silver, platinum and palladium spot. The free plan allows 100 requests a month; one request updates all four.", metals.configured, "set_metals_api_key", () => ctx.refresh()),
    row("Refresh metals on unlock", "At most every 30 hours, from a small share of the monthly allowance kept for automatic use.",
      toggle("", settings.metals_auto_refresh, (on) => save({ metals_auto_refresh: on }))),
    keyRow("CoinGecko", "Coin prices. The free Demo plan allows 10,000 calls a month; all your coins are priced in one request.", crypto.configured, "set_crypto_api_key", () => ctx.refresh())
  );
}

function privacyCard(settings, save) {
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, icon("eye", { size: 18 }), " Privacy")),
    row("Watch-only balance lookup",
      "Checking a crypto address's balance sends it to a public block explorer (blockstream.info), which learns that someone at your IP address is interested in it. The balance was already public; the link to you is not. Separate from price fetching, and off by default.",
      toggle("", settings.balance_lookup, (on) => save({ balance_lookup: on })))
  );
}

function displayCard(settings, save) {
  const currency = select(CURRENCIES.some(([c]) => c === settings.currency) ? CURRENCIES : [...CURRENCIES, [settings.currency, settings.currency]], settings.currency);
  currency.addEventListener("change", () => save({ currency: currency.value }));
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, icon("tag", { size: 18 }), " Currency")),
    row("Base currency", "New values are entered in it, totals are shown in it, and price feeds are asked for it. There is no conversion: holdings already valued in another currency are listed but left out of totals.", currency)
  );
}

function aboutCard(info) {
  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, icon("info", { size: 18 }), " This vault")),
    h("dl", { class: "kv kv-wide" },
      h("dt", {}, "Location"), h("dd", {}, h("code", { class: "path" }, info.location)),
      h("dt", {}, "Created"), h("dd", {}, fmt.date(info.created_at)),
      h("dt", {}, "Assets"), h("dd", {}, String(info.asset_count)),
      h("dt", {}, "Photos"), h("dd", {}, `${info.photo_count} · ${fmt.bytes(info.photo_bytes)} encrypted`),
      h("dt", {}, "Format"), h("dd", {}, `Schema ${info.schema_version} · key epoch ${info.key_epoch}`),
      h("dt", {}, "Encryption"), h("dd", {}, "Argon2id key derivation · XChaCha20-Poly1305 photos · SQLCipher database")
    ),
    h("p", { class: "card-text muted" }, "There is no password reset. If both the passphrase and the recovery key are lost, the vault cannot be opened by anyone.")
  );
}
