import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog, save as saveDialog, message, confirm } from "@tauri-apps/plugin-dialog";
import QRCode from "qrcode";

// The vault path is resolved in the backend, never sent from here: accepting
// a path over IPC would let the UI point vault operations anywhere.
const el = (id) => document.getElementById(id);
const show = (id) => {
  for (const s of document.querySelectorAll("main > section")) s.hidden = true;
  el(id).hidden = false;
};
const status = (msg) => { el("status").textContent = msg ?? ""; };

const ASSET_TYPES = [
  ["generic", "Generic Item"],
  ["gold_bullion", "Gold Bullion"],
  ["silver_bullion", "Silver Bullion"],
  ["platinum_bullion", "Platinum Bullion"],
  ["junk_silver", "Junk / Constitutional Silver"],
  ["sovereign_coin", "Sovereign Coin"],
];

let pendingRecovery = null;

async function boot() {
  try {
    const state = await invoke("vault_status");
    if (state.unlocked) return openCatalog();
    show(state.exists ? "vault-unlock" : "vault-setup");
  } catch (e) {
    status(describe(e));
  }
}

el("create-vault").addEventListener("click", async () => {
  const passphrase = el("new-passphrase").value;
  // Mirrors the backend check for a faster response; the backend enforces it.
  if ([...passphrase].length < 12) {
    return status("Use at least 12 characters — this is the only thing protecting the vault.");
  }
  if (passphrase !== el("confirm-passphrase").value) {
    return status("Passphrases do not match.");
  }

  try {
    status("Creating vault… this takes a moment while the key is derived.");
    const result = await invoke("create_vault", { passphrase });
    await beginRecoveryCeremony(result);
  } catch (e) {
    status(describe(e));
  }
});

el("unlock").addEventListener("click", async () => {
  try {
    status("Unlocking…");
    await invoke("unlock_vault", {
      secret: el("passphrase").value,
      useRecoveryKey: el("use-recovery").checked,
    });
    await openCatalog();
  } catch (e) {
    status(describe(e));
  }
});

/**
 * The ceremony deliberately refuses to continue until the key is acknowledged
 * AND its last 6 characters are retyped. Without that, people click through
 * and discover years later that they never saved it.
 */
async function beginRecoveryCeremony({ recovery_key, fingerprint }) {
  pendingRecovery = recovery_key;
  el("recovery-key").textContent = recovery_key;
  el("recovery-fingerprint").textContent = fingerprint;

  await QRCode.toCanvas(el("recovery-qr"), recovery_key, {
    errorCorrectionLevel: "Q",  // survives ~25% damage; these sit in safes for years
    margin: 2,
    width: 220,
  });

  el("recovery-ack").checked = false;
  el("recovery-verify").value = "";
  el("finish-recovery").disabled = true;
  show("recovery-ceremony");
  status("");
}

const checkCeremony = () => {
  const expected = pendingRecovery?.replace(/-/g, "").slice(-6).toUpperCase();
  const typed = el("recovery-verify").value.replace(/[^A-Za-z0-9]/g, "").toUpperCase();
  el("finish-recovery").disabled = !(el("recovery-ack").checked && typed === expected);
};
el("recovery-ack").addEventListener("change", checkCeremony);
el("recovery-verify").addEventListener("input", checkCeremony);

el("print-recovery").addEventListener("click", () => {
  // The PDF filename comes from document.title, so "Asset Manager" produced
  // output.pdf. A dated, descriptive name is what a user needs to find this
  // again in a folder of saved files.
  const stamp = new Date().toISOString().slice(0, 10);
  const previousTitle = document.title;
  document.title = `Asset-Manager-Recovery-Key-${stamp}`;

  el("print-date").textContent = new Date().toLocaleString();

  // Restore afterwards so the window title is not left changed. Printing is
  // synchronous in the WebView, but afterprint is the documented hook.
  const restore = () => {
    document.title = previousTitle;
    window.removeEventListener("afterprint", restore);
  };
  window.addEventListener("afterprint", restore);

  window.print();
  // Belt and braces: if afterprint never fires, do not strand the title.
  setTimeout(restore, 1000);
});

el("finish-recovery").addEventListener("click", async () => {
  pendingRecovery = null;
  el("recovery-key").textContent = "";
  await openCatalog();
});

el("export-csv").addEventListener("click", async () => {
  try {
    const result = await invoke("export_csv");
    // Warn before writing, not after: once the file exists the catalog is
    // outside the vault's protection.
    const proceed = await confirm(`${result.warning}\n\nExport ${result.row_count} assets anyway?`, {
      title: "Export is not encrypted",
      kind: "warning",
    });
    if (!proceed) return;

    const path = await saveDialog({
      defaultPath: "asset-manager-export.csv",
      filters: [{ name: "CSV", extensions: ["csv"] }],
    });
    if (!path) return;

    await invoke("write_text_file", { path, contents: result.csv });
    status(`Exported ${result.row_count} assets.`);
  } catch (e) {
    status(describe(e));
  }
});

el("import-csv").addEventListener("click", async () => {
  try {
    const path = await openDialog({ filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (!path) return;

    const contents = await invoke("read_text_file", { path });

    // Preview first, always: it reports every row problem at once.
    const preview = await invoke("import_csv", { contents, apply: false });
    if (preview.errors.length) {
      await message(preview.errors.slice(0, 20).join("\n"), {
        title: `${preview.errors.length} problem(s) — nothing was imported`,
        kind: "error",
      });
      return;
    }

    const proceed = await confirm(
      `Create ${preview.creates} and update ${preview.updates} asset(s)?`,
      { title: "Confirm import" }
    );
    if (!proceed) return;

    const applied = await invoke("import_csv", { contents, apply: true });
    await refresh();
    status(`Imported: ${applied.creates} created, ${applied.updates} updated.`);
  } catch (e) {
    status(describe(e));
  }
});

el("lock").addEventListener("click", lockUi);

async function lockUi() {
  await invoke("lock_vault");
  toUnlockScreen();
}

function toUnlockScreen() {
  el("passphrase").value = "";
  el("search").value = "";
  el("assets").querySelector("tbody").innerHTML = "";  // don't leave data on screen
  currentAsset = null;
  el("photos").innerHTML = "";  // decrypted images must not survive a lock
  el("detail").hidden = true;
  show("vault-unlock");
}

// The backend locks on idle whether or not the UI reacts; this just moves the
// user off the catalog screen and says why.
listen("vault-auto-locked", () => {
  toUnlockScreen();
  status("Locked after inactivity.");
});

el("new-asset").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    await invoke("create_asset", {
      asset: {
        type_id: el("asset-type").value,
        name: el("asset-name").value,
        quantity: el("asset-quantity").value || "1",
        quantity_unit: "item",
        storage_location: el("asset-location").value || null,
        notes: null,
      },
    });
    el("asset-name").value = "";
    el("asset-location").value = "";
    await refresh();
  } catch (e) {
    status(describe(e));
  }
});

let searchTimer;
el("search").addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(refresh, 150);
});

async function openCatalog() {
  const select = el("asset-type");
  select.innerHTML = "";
  for (const [value, label] of ASSET_TYPES) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    select.append(option);
  }
  show("catalog");
  await refresh();
}

async function refresh() {
  try {
    const query = el("search").value.trim();
    const rows = query
      ? await invoke("search_assets", { query })
      : await invoke("list_assets");

    const tbody = el("assets").querySelector("tbody");
    tbody.innerHTML = "";
    for (const row of rows) {
      const tr = document.createElement("tr");
      tr.addEventListener("click", () => showDetail(row));
      for (const cell of [
        row.name,
        row.type_id,
        `${row.quantity} ${row.quantity_unit}`,
        row.storage_location ?? "",
        formatMoney(row.current_amount_minor, row.current_currency),
      ]) {
        const td = document.createElement("td");
        td.textContent = cell;   // textContent, never innerHTML: names are user data
        tr.append(td);
      }
      tbody.append(tr);
    }
    status("");
  } catch (e) {
    status(describe(e));
  }
}

let currentAsset = null;

async function showDetail(row) {
  currentAsset = row;
  el("detail-name").textContent = row.name;
  el("detail").hidden = false;
  await refreshPhotos();
}

el("close-detail").addEventListener("click", () => {
  currentAsset = null;
  el("detail").hidden = true;
  el("photos").innerHTML = "";
});

el("add-photo").addEventListener("click", async () => {
  if (!currentAsset) return;
  try {
    const selected = await openDialog({
      multiple: true,
      filters: [{ name: "Images and PDFs", extensions: ["jpg", "jpeg", "png", "webp", "pdf"] }],
    });
    if (!selected) return;

    const paths = Array.isArray(selected) ? selected : [selected];
    let deduped = 0;
    for (const path of paths) {
      const result = await invoke("import_photo", { assetId: currentAsset.asset_id, path });
      if (result.deduplicated) deduped += 1;
    }
    await refreshPhotos();
    status(deduped ? `Added ${paths.length} file(s); ${deduped} already in the vault.` : "");
  } catch (e) {
    status(describe(e));
  }
});

async function refreshPhotos() {
  if (!currentAsset) return;
  try {
    const photos = await invoke("list_photos", { assetId: currentAsset.asset_id });
    const container = el("photos");
    container.innerHTML = "";

    for (const photo of photos) {
      const wrap = document.createElement("div");
      wrap.className = "photo";

      if (photo.media_type.startsWith("image/")) {
        const img = document.createElement("img");
        // Served by the asset:// handler, which decrypts in-process. The
        // variant query asks for a thumbnail; the handler falls back to the
        // original if that variant has not been generated.
        img.src = `asset://localhost/media/${photo.object_id}?variant=256`;
        img.alt = "";
        img.loading = "lazy";
        wrap.append(img);
      } else {
        const placeholder = document.createElement("div");
        placeholder.className = "photo-placeholder";
        placeholder.textContent = photo.media_type;
        wrap.append(placeholder);
      }

      if (photo.is_primary) {
        const badge = document.createElement("span");
        badge.className = "primary-badge";
        badge.textContent = "Primary";
        wrap.append(badge);
      }

      const remove = document.createElement("button");
      remove.textContent = "Remove";
      remove.addEventListener("click", async (event) => {
        event.stopPropagation();
        try {
          await invoke("remove_photo", {
            assetId: currentAsset.asset_id,
            objectId: photo.object_id,
          });
          await refreshPhotos();
        } catch (e) {
          status(describe(e));
        }
      });
      wrap.append(remove);

      container.append(wrap);
    }
  } catch (e) {
    status(describe(e));
  }
}

/**
 * Money arrives as a string of minor units. Parsing it into a JS number would
 * be wrong for large values, so format by string manipulation.
 */
function formatMoney(amountMinor, currency) {
  if (amountMinor == null || currency == null) return "—";
  const negative = amountMinor.startsWith("-");
  const digits = (negative ? amountMinor.slice(1) : amountMinor).padStart(3, "0");
  const major = digits.slice(0, -2);
  const minor = digits.slice(-2);
  return `${negative ? "-" : ""}${major}.${minor} ${currency}`;
}

function describe(error) {
  if (error && typeof error === "object" && "message" in error) {
    if (error.kind === "another_instance") {
      return "Another Asset Manager window already has this vault open.";
    }
    if (error.kind === "pairing_mismatch") {
      return error.message;  // already explains the partial-restore cause
    }
    return error.message;
  }
  return String(error);
}

boot();
