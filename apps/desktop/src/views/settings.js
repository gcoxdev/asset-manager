// Settings: security, backup, price feeds, privacy, display.

import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { call, describe } from "../lib/api.js";
import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import * as fmt from "../lib/format.js";
import * as store from "../lib/store.js";
import { busy, toast, modal, field, select, toggle, callout } from "../ui/components.js";
import { recoveryCeremony, restoreDialog, restoredMessage } from "./onboarding.js";

const CURRENCIES = [
  ["USD", "US dollar (USD)"], ["EUR", "Euro (EUR)"], ["GBP", "British pound (GBP)"], ["CAD", "Canadian dollar (CAD)"],
  ["AUD", "Australian dollar (AUD)"], ["CHF", "Swiss franc (CHF)"], ["JPY", "Japanese yen (JPY)"], ["NZD", "New Zealand dollar (NZD)"],
  ["SEK", "Swedish krona (SEK)"], ["NOK", "Norwegian krone (NOK)"], ["DKK", "Danish krone (DKK)"], ["SGD", "Singapore dollar (SGD)"],
];

export async function renderSettings(root, params, ctx) {
  const [settings, info, metals, crypto] = await Promise.all([
    store.settings({ fresh: true }),
    call("vault_info"),
    call("metals_provider_status"),
    call("crypto_provider_status"),
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
    backupCard(settings, ctx),
    feedsCard(settings, metals, crypto, save, ctx),
    privacyCard(settings, save),
    displayCard(settings, save),
    aboutCard(info)
  );

  if (params.section === "feeds") root.querySelector("#feeds")?.scrollIntoView({ block: "start" });
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

function backupCard(settings, ctx) {
  const last = settings.last_backup_at;
  const days = last ? fmt.daysSince(last) : null;
  const status = last
    ? h("span", { class: days > 30 ? "badge badge-attention" : "badge badge-fresh" }, icon(days > 30 ? "warn" : "check", { size: 12 }), `Last backup ${fmt.ago(last)}`)
    : h("span", { class: "badge badge-attention" }, icon("warn", { size: 12 }), "Never backed up");

  const backup = h("button", { class: "btn btn-primary", onclick: async () => {
    const directory = await openDialog({ directory: true, title: "Choose where to put the backup" });
    if (!directory) return;
    await busy(backup, async () => {
      const result = await call("backup_vault", { directory });
      store.setSettings(null);
      toast(`Backup saved to ${result.path} (${result.objects} photo${result.objects === 1 ? "" : "s"}).`, { kind: "success", timeout: 9000 });
      ctx.refresh();
    }, "Backing up…");
  } }, icon("archive", { size: 16 }), "Back up now…");

  const restore = h("button", { class: "btn btn-secondary", onclick: async () => {
    const restored = await restoreDialog({ replacing: true });
    if (restored?.opened) {
      // A different vault is open now; nothing cached from this one applies.
      ctx.reopen();
      toast(restoredMessage(restored), { kind: "success", timeout: 7000 });
      return;
    }
    // A restore that failed after the swap began leaves the session locked.
    const state = await call("session_state").catch(() => null);
    if (state && !state.unlocked) ctx.lock();
  } }, icon("upload", { size: 16 }), "Restore…");

  return h("section", { class: "card" },
    h("div", { class: "card-head" }, h("h2", {}, icon("archive", { size: 18 }), " Backup & restore"), status),
    h("p", { class: "card-text" }, "A backup is a complete, still-encrypted copy of the vault — records, photos and history. It is safe to keep on an external drive or in a synced folder; without your passphrase or recovery key it is unreadable."),
    h("p", { class: "card-text muted" }, "The vault closes for a moment while it is copied, so nothing changes mid-copy. You stay signed in."),
    h("div", { class: "btn-row" }, backup, restore)
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
