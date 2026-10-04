// Welcome, unlock, restore, and the recovery-key ceremony.

import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import QRCode from "qrcode";

import { call, describe } from "../lib/api.js";
import { h, mount, clear } from "../lib/dom.js";
import { icon, brandMark } from "../lib/icons.js";
import { busy, callout, confirmDialog, toast } from "../ui/components.js";
import { date as fmtDate } from "../lib/format.js";

const MIN_CHARS = 12;

export function showOnboarding(root, { mode, onUnlocked, notice }) {
  const panel = h("div", { class: "onboard-card" });
  const page = h(
    "div",
    { class: "onboard" },
    h(
      "section",
      { class: "onboard-brand" },
      h("div", { class: "onboard-brand-inner" },
        brandMark(56),
        h("h1", {}, "Asset Manager"),
        h("p", { class: "onboard-tagline" }, "Catalog and value everything you own — metals, coins, cards, comics, crypto and more."),
        h("ul", { class: "onboard-points" },
          point("shield", "Encrypted on this computer", "Photos and records are unreadable without your passphrase."),
          point("markets", "Honest valuations", "Market prices where they exist, your own figures everywhere else."),
          point("archive", "Yours, offline", "No account, no cloud, no tracking. Price feeds are optional.")
        )
      )
    ),
    h("section", { class: "onboard-form" }, panel)
  );
  mount(root, page);

  const go = {
    welcome: () => renderWelcome(panel, go),
    create: () => renderCreate(panel, go, onUnlocked),
    unlock: (message) => renderUnlock(panel, go, onUnlocked, message),
  };
  (go[mode] ?? go.unlock)(notice);
}

function point(glyph, title, body) {
  return h("li", {}, h("span", { class: "onboard-point-icon" }, icon(glyph, { size: 18 })), h("div", {}, h("strong", {}, title), h("span", {}, body)));
}

function renderWelcome(panel, go) {
  mount(
    panel,
    h("h2", {}, "Welcome"),
    h("p", { class: "lede" }, "Set up a vault to hold your catalog. It lives in one folder on this computer and is encrypted with a passphrase you choose."),
    h("div", { class: "choice-list" },
      h("button", { class: "choice", onclick: go.create },
        h("span", { class: "choice-icon" }, icon("plus", { size: 20 })),
        h("span", { class: "choice-text" }, h("strong", {}, "Create a new vault"), h("span", {}, "Start an empty, encrypted catalog.")),
        icon("forward")
      ),
      h("button", { class: "choice", onclick: () => restoreFlow(go) },
        h("span", { class: "choice-icon" }, icon("archive", { size: 20 })),
        h("span", { class: "choice-text" }, h("strong", {}, "Restore from a backup"), h("span", {}, "Moving to a new computer, or recovering a vault.")),
        icon("forward")
      )
    )
  );
}

/** Rough strength guidance. Not a guarantee — length does most of the work. */
function strength(passphrase) {
  const chars = [...passphrase].length;
  const words = passphrase.trim().split(/\s+/).filter((w) => w.length >= 3).length;
  if (chars === 0) return { score: 0, label: "", hint: `At least ${MIN_CHARS} characters.` };
  if (chars < MIN_CHARS) return { score: 1, label: "Too short", hint: `${MIN_CHARS - chars} more character${MIN_CHARS - chars === 1 ? "" : "s"} needed.` };
  if (/^(.)\1+$/.test(passphrase)) return { score: 1, label: "Too simple", hint: "Repeating one character is easy to guess." };
  if (chars >= 24 || words >= 5) return { score: 4, label: "Strong", hint: "This will hold up well." };
  if (chars >= 18 || words >= 4) return { score: 3, label: "Good", hint: "A few more characters would make it stronger." };
  return { score: 2, label: "Fair", hint: "Longer is stronger — several unrelated words work well." };
}

function passwordField(label, props = {}) {
  const input = h("input", { type: "password", class: "input-lg", spellcheck: "false", ...props });
  const reveal = h("button", {
    type: "button",
    class: "icon-btn reveal",
    "aria-label": "Show passphrase",
    onclick: () => {
      const showing = input.type === "text";
      input.type = showing ? "password" : "text";
      mount(reveal, icon(showing ? "eye" : "eyeOff"));
      reveal.setAttribute("aria-label", showing ? "Show passphrase" : "Hide passphrase");
    },
  }, icon("eye"));
  const wrap = h("div", { class: "field" }, h("label", { for: props.id }, label), h("div", { class: "input-with-button" }, input, reveal));
  return { wrap, input };
}

function renderCreate(panel, go, onUnlocked) {
  const pass = passwordField("Passphrase", { id: "new-pass", autocomplete: "new-password", autofocus: true });
  const confirm = passwordField("Confirm passphrase", { id: "confirm-pass", autocomplete: "new-password" });
  const meter = h("div", { class: "meter", "aria-hidden": "true" }, [1, 2, 3, 4].map(() => h("span")));
  const meterText = h("p", { class: "field-hint" }, `At least ${MIN_CHARS} characters.`);
  const error = h("p", { class: "form-error", role: "alert" });
  const submit = h("button", { class: "btn btn-primary btn-lg btn-block", type: "submit" }, "Create vault");

  pass.input.addEventListener("input", () => {
    const s = strength(pass.input.value);
    meter.dataset.score = s.score;
    meterText.textContent = s.label ? `${s.label} — ${s.hint}` : s.hint;
  });

  const form = h(
    "form",
    {
      class: "stack",
      onsubmit: async (event) => {
        event.preventDefault();
        error.textContent = "";
        const passphrase = pass.input.value;
        // Mirrors the backend check for a faster answer; the backend enforces it.
        if ([...passphrase].length < MIN_CHARS) {
          error.textContent = `Use at least ${MIN_CHARS} characters — this is the only thing protecting the vault.`;
          return;
        }
        if (passphrase !== confirm.input.value) {
          error.textContent = "The two passphrases do not match.";
          return;
        }
        await busy(submit, async () => {
          try {
            const created = await call("create_vault", { passphrase });
            pass.input.value = "";
            confirm.input.value = "";
            mount(panel, recoveryCeremony(created, { context: "create", onDone: onUnlocked }));
          } catch (e) {
            error.textContent = describe(e);
          }
        }, "Deriving key…");
      },
    },
    pass.wrap,
    h("div", { class: "meter-row" }, meter, meterText),
    confirm.wrap,
    callout("warning", h("strong", {}, "There is no password reset. "), "If you lose both this passphrase and the recovery key you are about to receive, the catalog cannot be recovered — by anyone."),
    error,
    submit
  );

  mount(
    panel,
    h("button", { class: "link-back", onclick: go.welcome }, icon("back", { size: 16 }), "Back"),
    h("h2", {}, "Create your vault"),
    h("p", { class: "lede" }, "Choose a passphrase you can remember. A few unrelated words is easier to recall than symbols, and far harder to guess."),
    form
  );
}

function renderUnlock(panel, go, onUnlocked, notice) {
  let useRecovery = false;
  const pass = passwordField("Passphrase", { id: "unlock-pass", autocomplete: "current-password", autofocus: true });
  const error = h("p", { class: "form-error", role: "alert" });
  const submit = h("button", { class: "btn btn-primary btn-lg btn-block", type: "submit" }, icon("unlock"), "Unlock");
  const label = pass.wrap.querySelector("label");

  const switcher = h("button", {
    type: "button",
    class: "link",
    onclick: () => {
      useRecovery = !useRecovery;
      label.textContent = useRecovery ? "Recovery key" : "Passphrase";
      switcher.textContent = useRecovery ? "Use my passphrase instead" : "Use the recovery key instead";
      pass.input.value = "";
      pass.input.placeholder = useRecovery ? "The key from your printed sheet" : "";
      pass.input.focus();
    },
  }, "Use the recovery key instead");

  const form = h(
    "form",
    {
      class: "stack",
      onsubmit: async (event) => {
        event.preventDefault();
        error.textContent = "";
        if (!pass.input.value) {
          error.textContent = useRecovery ? "Enter the recovery key." : "Enter your passphrase.";
          return;
        }
        await busy(submit, async () => {
          try {
            await call("unlock_vault", { secret: pass.input.value, useRecoveryKey: useRecovery });
            pass.input.value = "";
            onUnlocked();
          } catch (e) {
            // Deliberately one message for a wrong credential and a damaged
            // header: the difference must not leak.
            error.textContent = describe(e);
            pass.input.select();
          }
        }, "Unlocking…");
      },
    },
    pass.wrap,
    error,
    submit,
    h("div", { class: "form-links" }, switcher, h("button", { type: "button", class: "link", onclick: () => restoreFlow(go) }, "Restore from a backup"))
  );

  mount(
    panel,
    h("div", { class: "unlock-lockmark" }, icon("lock", { size: 26 })),
    h("h2", {}, "Unlock your vault"),
    notice ? h("p", { class: "notice" }, notice) : h("p", { class: "lede" }, "Enter your passphrase to open the catalog."),
    form
  );
}

async function restoreFlow(go) {
  const ok = await confirmDialog({
    title: "Restore from a backup",
    message: [
      "Choose the backup folder — the one containing manifest.json and vault.header.",
      "If a vault already exists on this computer it is set aside as “vault.pre-restore”, not deleted. You will then unlock the restored vault with the passphrase or recovery key it had when the backup was made.",
    ],
    confirmLabel: "Choose folder…",
  });
  if (!ok) return;
  const directory = await openDialog({ directory: true, title: "Choose a backup folder" });
  if (!directory) return;
  try {
    const restored = await call("restore_vault", { directory });
    toast(`Restored a backup from ${fmtDate(restored.created_at)} with ${restored.objects} photo${restored.objects === 1 ? "" : "s"}.`, { kind: "success" });
    go.unlock("Backup restored. Unlock it with its passphrase or recovery key.");
  } catch (e) {
    toast(describe(e), { kind: "error" });
  }
}

// ------------------------------------------------------------ ceremony

/**
 * The recovery-key ceremony.
 *
 * Refuses to continue until the key is acknowledged AND its last 6
 * characters are retyped. Without that, people click through and discover
 * years later that they never saved it.
 */
export function recoveryCeremony({ recovery_key, fingerprint }, { context, onDone }) {
  const key = recovery_key;
  const groups = keyGroups(key);
  const canvas = h("canvas", { class: "qr", "aria-label": "Recovery key as a QR code" });
  QRCode.toCanvas(canvas, key, { errorCorrectionLevel: "Q", margin: 1, width: 168 }).catch(() => {});

  const ack = h("input", { type: "checkbox", id: "rk-ack" });
  const verify = h("input", { type: "text", class: "input-mono", id: "rk-verify", maxlength: 12, autocomplete: "off", spellcheck: "false", placeholder: "••••••" });
  const finish = h("button", { class: "btn btn-primary btn-lg btn-block", disabled: true }, context === "create" ? "Open my vault" : "Done");

  const expected = key.replace(/[^A-Za-z0-9]/g, "").slice(-6).toUpperCase();
  const check = () => {
    const typed = verify.value.replace(/[^A-Za-z0-9]/g, "").toUpperCase();
    finish.disabled = !(ack.checked && typed === expected);
  };
  ack.addEventListener("change", check);
  verify.addEventListener("input", check);

  finish.addEventListener("click", () => {
    clear(canvas.parentElement ?? canvas);
    onDone();
  });

  const copy = h("button", { class: "btn btn-secondary", onclick: async () => {
    try {
      await navigator.clipboard.writeText(key);
      toast("Copied. Paste it somewhere safe, then clear your clipboard.", { kind: "info" });
    } catch {
      toast("Copying is not available here — use Save or Print.", { kind: "warning" });
    }
  } }, icon("copy", { size: 16 }), "Copy");

  const saveText = h("button", { class: "btn btn-secondary", onclick: async () => {
    const stamp = new Date().toISOString().slice(0, 10);
    const path = await saveDialog({ defaultPath: `Asset-Manager-Recovery-Key-${stamp}.txt`, filters: [{ name: "Text", extensions: ["txt"] }] });
    if (!path) return;
    try {
      await call("write_text_file", { path, contents: recoveryText(key, fingerprint) });
      toast("Saved. Move it somewhere offline — a USB stick in a drawer, not this computer.", { kind: "success" });
    } catch (e) {
      toast(describe(e), { kind: "error" });
    }
  } }, icon("download", { size: 16 }), "Save as text");

  const print = h("button", { class: "btn btn-secondary", onclick: () => printRecoverySheet(key, fingerprint) }, icon("printer", { size: 16 }), "Print");

  return h(
    "div",
    { class: "ceremony" },
    h("h2", {}, context === "create" ? "Save your recovery key" : "Your new recovery key"),
    h("p", { class: "lede" }, "This key opens the vault if you forget your passphrase. It is shown once. ", context === "rotate" ? "The previous key no longer opens this vault — but it still opens any backup made before today." : null),
    h("div", { class: "key-display" },
      h("div", { class: "key-groups", "aria-label": "Recovery key" }, groups.map((g) => h("span", {}, g))),
      canvas
    ),
    h("p", { class: "fingerprint" }, "Fingerprint ", h("code", {}, fingerprint), h("span", {}, " — identifies this key on a printed sheet; reveals nothing about it.")),
    h("div", { class: "btn-row" }, print, saveText, copy),
    h("div", { class: "ceremony-verify" },
      h("label", { class: "check" }, ack, h("span", {}, "I have stored this key somewhere safe and offline")),
      h("div", { class: "field" }, h("label", { for: "rk-verify" }, "Type the last 6 characters to confirm"), verify)
    ),
    finish
  );
}

/**
 * The key arrives already grouped ("ABCD-EFGH-…"). Unlock ignores anything
 * but letters and digits, so it can be typed back with or without the dashes.
 */
function keyGroups(key) {
  const parts = key.split("-").filter(Boolean);
  return parts.length > 1 ? parts : key.match(/.{1,4}/g) ?? [key];
}

function recoveryText(key, fingerprint) {
  return [
    "Asset Manager — Recovery Key",
    "",
    key,
    "",
    `Fingerprint: ${fingerprint}`,
    `Generated: ${new Date().toLocaleString()}`,
    "",
    "This key unlocks an Asset Manager vault without its passphrase.",
    "Anyone holding it can open that vault. Keep it offline and private.",
    "There is no password reset: if both the passphrase and this key are",
    "lost, the catalog cannot be recovered by anyone.",
    "",
  ].join("\n");
}

async function printRecoverySheet(key, fingerprint) {
  const root = document.getElementById("print-root");
  const canvas = h("canvas");
  await QRCode.toCanvas(canvas, key, { errorCorrectionLevel: "Q", margin: 2, width: 260 });
  const img = h("img", { src: canvas.toDataURL("image/png"), alt: "", class: "print-qr" });
  mount(
    root,
    h("div", { class: "print-sheet recovery-sheet" },
      h("h1", {}, "Asset Manager — Recovery Key"),
      h("p", {}, "This key unlocks an Asset Manager vault without its passphrase. Anyone holding this sheet can open that vault. Store it somewhere private and offline."),
      h("pre", { class: "print-key" }, keyGroups(key).join(" ")),
      img,
      h("p", {}, "Fingerprint: ", h("code", {}, fingerprint), " — identifies this key, so two printed sheets can be told apart. It is not a vault name and reveals nothing about the key."),
      h("p", {}, h("strong", {}, "There is no password reset. "), "If both the passphrase and this key are lost, the catalog cannot be recovered by anyone, including the software's author."),
      h("p", { class: "print-meta" }, `Generated ${new Date().toLocaleString()}`)
    )
  );
  runPrint(`Asset-Manager-Recovery-Key-${new Date().toISOString().slice(0, 10)}`);
}

/**
 * Print whatever is in #print-root. The PDF filename comes from the document
 * title, so it is set to something a person can find in a folder later.
 */
export function runPrint(title) {
  const previous = document.title;
  document.title = title;
  document.body.classList.add("printing");
  const restore = () => {
    document.title = previous;
    document.body.classList.remove("printing");
    // Plaintext must not linger in the DOM after printing.
    clear(document.getElementById("print-root"));
    window.removeEventListener("afterprint", restore);
  };
  window.addEventListener("afterprint", restore);
  // Let images decode before the print snapshot.
  setTimeout(() => {
    window.print();
    setTimeout(restore, 1500);
  }, 150);
}
