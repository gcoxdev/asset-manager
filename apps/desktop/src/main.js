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
  if (!presetCache.length) await loadPresets();
  if (!coinCache.length) await loadCoins();
  await refreshSpotPrices();
  await refreshCryptoStatus();
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

      for (const cell of [
        row.name,
        row.type_id,
        `${row.quantity} ${row.quantity_unit}`,
        row.storage_location ?? "",
      ]) {
        const td = document.createElement("td");
        td.textContent = cell;   // textContent, never innerHTML: names are user data
        td.addEventListener("click", () => showDetail(row));
        tr.append(td);
      }

      // Value is editable in place: setting prices one dialog at a time does
      // not scale to a few hundred collectibles.
      const valueCell = document.createElement("td");
      valueCell.className = "value-cell";
      valueCell.textContent = row.current_display ?? "—";
      valueCell.title = "Click to edit";
      valueCell.addEventListener("click", (event) => {
        event.stopPropagation();
        beginPriceEdit(valueCell, row);
      });
      tr.append(valueCell);

      const detailCell = document.createElement("td");
      const openButton = document.createElement("button");
      openButton.textContent = "Details";
      openButton.addEventListener("click", (event) => {
        event.stopPropagation();
        showDetail(row);
      });
      detailCell.append(openButton);
      tr.append(detailCell);

      tbody.append(tr);
    }
    await refreshPortfolio();
    await refreshChart();
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
  await showValuationHistory();
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
 * Edit a price in place.
 *
 * The entered text is sent to the backend as a string and parsed there as an
 * exact decimal. Parsing it to a JS number first would lose precision.
 */
function beginPriceEdit(cell, row) {
  if (cell.querySelector("input")) return;

  const previous = cell.textContent;
  const currency = row.current_currency ?? "USD";
  const startingValue =
    row.current_amount_minor == null ? "" : stripCurrency(previous);

  cell.textContent = "";
  const input = document.createElement("input");
  input.type = "text";
  input.value = startingValue;
  input.placeholder = "0.00";
  cell.append(input);
  input.focus();
  input.select();

  let settled = false;
  const commit = async () => {
    if (settled) return;
    settled = true;
    const typed = input.value.trim();

    if (typed === "" || typed === startingValue) {
      cell.textContent = previous;   // nothing to do
      return;
    }

    try {
      const [result] = await invoke("set_prices", {
        entries: [{ asset_id: row.asset_id, amount: typed, currency }],
      });
      if (!result.ok) {
        cell.textContent = previous;
        return status(result.error ?? "Could not save that price.");
      }
      await refresh();
    } catch (e) {
      cell.textContent = previous;
      status(describe(e));
    }
  };

  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") { event.preventDefault(); commit(); }
    if (event.key === "Escape") { settled = true; cell.textContent = previous; }
  });
  input.addEventListener("blur", commit);
}

/** "1234.50 USD" -> "1234.50", so editing starts from the number alone. */
function stripCurrency(text) {
  const match = text.match(/^-?[\d.]+/);
  return match ? match[0] : "";
}

let presetCache = [];

/**
 * Spot prices, each with its own entry field.
 *
 * Staleness is shown as words, not colour alone: a coloured dot means nothing
 * to a colour-blind user and nothing at all when printed.
 */
async function refreshSpotPrices() {
  try {
    const prices = await invoke("spot_prices");
    const container = el("spot-prices");
    container.innerHTML = "";

    for (const price of prices) {
      const card = document.createElement("div");
      card.className = "spot";

      const heading = document.createElement("h4");
      heading.textContent = price.metal_name;
      card.append(heading);

      const value = document.createElement("div");
      value.className = "price";
      value.textContent = price.price_per_troy_oz
        ? `${price.price_per_troy_oz} ${price.currency}/oz`
        : "not set";
      card.append(value);

      const freshness = document.createElement("div");
      freshness.className = price.needs_caveat ? "freshness warn" : "freshness";
      if (price.freshness) {
        const days = Math.floor((price.age_hours ?? 0) / 24);
        const age = days >= 1 ? `${days}d old` : "today";
        freshness.textContent = `${price.freshness} — ${age}, from ${price.source}`;
      } else {
        freshness.textContent = "enter a price to value holdings";
      }
      card.append(freshness);

      // Manual entry is always available: it never spends API quota and works
      // with the provider down.
      const input = document.createElement("input");
      input.type = "text";
      input.placeholder = "price/oz";
      input.setAttribute("aria-label", `${price.metal_name} spot price per troy ounce`);
      input.addEventListener("keydown", async (event) => {
        if (event.key !== "Enter") return;
        const typed = input.value.trim();
        if (!typed) return;
        try {
          await invoke("set_spot_price", { metal: price.metal, price: typed });
          input.value = "";
          await refreshSpotPrices();
          status("");
        } catch (e) {
          status(describe(e));
        }
      });
      card.append(input);

      container.append(card);
    }

    const provider = await invoke("metals_provider_status");
    const quota = provider.quota;
    el("quota-note").textContent = provider.configured
      ? `Price feed: ${quota.used_this_month} of ${quota.monthly_limit} requests used this month. ` +
        `Typing a price by hand costs nothing.`
      : "No price feed configured — enter prices by hand, or add a key below.";

    // The button stays enabled without a key so the error explains what to
    // do; a disabled control with no explanation is its own dead end.
    el("refresh-spot").textContent = provider.configured
      ? "Update prices"
      : "Update prices (needs a key)";
  } catch (e) {
    // A locked vault is the usual reason; nothing to report.
  }
}

el("refresh-spot").addEventListener("click", async () => {
  const button = el("refresh-spot");
  button.disabled = true;
  status("Fetching prices…");
  try {
    const result = await invoke("refresh_spot_prices", { automatic: false });
    await refreshSpotPrices();
    status(
      result.updated.length
        ? `Updated ${result.updated.join(", ")}. ` +
          `${result.quota.remaining} of ${result.quota.monthly_limit} requests left this month.`
        : "The feed returned no usable prices."
    );
  } catch (e) {
    status(describe(e));
  } finally {
    button.disabled = false;
  }
});

el("save-metals-key").addEventListener("click", async () => {
  const input = el("metals-key");
  try {
    await invoke("set_metals_api_key", { key: input.value });
    input.value = "";   // never leave a key sitting in the DOM
    await refreshSpotPrices();
    status("Price feed key saved to your system keyring.");
  } catch (e) {
    status(describe(e));
  }
});

el("clear-metals-key").addEventListener("click", async () => {
  try {
    await invoke("set_metals_api_key", { key: "" });
    el("metals-key").value = "";
    await refreshSpotPrices();
    status("Price feed key removed.");
  } catch (e) {
    status(describe(e));
  }
});

let coinCache = [];

async function loadCoins() {
  try {
    coinCache = await invoke("common_coins");
    const select = el("crypto-coin");
    for (const coin of coinCache) {
      const option = document.createElement("option");
      option.value = coin.coin_id;
      option.textContent = `${coin.name} (${coin.symbol})`;
      select.append(option);
    }
  } catch (e) {
    // The bundled list is static; a failure just leaves manual entry.
  }
}

// Picking a coin fills the ID field, which stays visible and editable: the
// bundled list is short by design, and anything not on it needs a real coin
// ID rather than a guessed symbol.
el("crypto-coin").addEventListener("change", () => {
  const chosen = el("crypto-coin").value;
  el("crypto-coin-id").value = chosen;
});

async function refreshCryptoStatus() {
  try {
    const status = await invoke("crypto_provider_status");
    // Attribution is required by CoinGecko's branding guidelines wherever
    // their data appears.
    el("crypto-attribution").textContent = status.configured
      ? status.attribution
      : "No price feed configured — add a key below, or enter values by hand.";
    el("refresh-crypto").textContent = status.configured
      ? "Update prices"
      : "Update prices (needs a key)";
  } catch (e) {
    // Locked vault; nothing to show.
  }
}

el("refresh-crypto").addEventListener("click", async () => {
  const button = el("refresh-crypto");

  // Only ask for what is actually held: every id costs credits, and asking
  // for the whole bundled list would spend them on coins nobody owns.
  const wanted = [...new Set(
    [el("crypto-coin-id").value.trim().toLowerCase()].filter(Boolean)
  )];
  if (!wanted.length) {
    return status("Enter a coin ID first, so we only fetch what you hold.");
  }

  button.disabled = true;
  status("Fetching prices…");
  try {
    const result = await invoke("refresh_crypto_prices", { coinIds: wanted });
    const parts = [];
    if (result.updated.length) parts.push(`Updated ${result.updated.join(", ")}.`);
    // Naming what came back empty distinguishes a wrong ID from an outage.
    if (result.missing.length) {
      parts.push(`No price for ${result.missing.join(", ")} — check the coin ID.`);
    }
    status(parts.join(" ") || "Nothing to update.");
    await refreshCryptoStatus();
  } catch (e) {
    status(describe(e));
  } finally {
    button.disabled = false;
  }
});

el("crypto-calc").addEventListener("click", async () => {
  const coinId = el("crypto-coin-id").value.trim();
  if (!coinId) return status("Enter a coin ID.");

  const known = coinCache.find((c) => c.coin_id === coinId);
  try {
    const result = await invoke("value_crypto_holding", {
      holding: {
        coin_id: coinId,
        symbol: known ? known.symbol : coinId.toUpperCase(),
        quantity: el("crypto-qty").value || "0",
        custody: el("crypto-custody").value,
        location: el("crypto-where").value || null,
        chain: null,
        contract: null,
      },
    });
    renderCryptoResult(result);
    status("");
  } catch (e) {
    el("crypto-result").textContent = "";
    status(describe(e));
  }
});

function renderCryptoResult(result) {
  const container = el("crypto-result");
  container.innerHTML = "";

  if (result.unpriced_reason) {
    // Say why there is no number rather than showing a blank.
    const note = document.createElement("p");
    note.className = "unpriced";
    note.textContent = `${result.label}: ${result.unpriced_reason}`;
    container.append(note);
    return;
  }

  const list = document.createElement("dl");
  for (const [label, value] of [
    ["Holding", `${result.quantity} ${result.label}`],
    ["Unit price", `${result.unit_price} ${result.currency}`],
    ["Value", result.value],
    ["Priced at", result.source_asof ?? "unknown"],
  ]) {
    const dt = document.createElement("dt");
    dt.textContent = label;
    const dd = document.createElement("dd");
    dd.textContent = value;
    list.append(dt, dd);
  }
  container.append(list);

  const credit = document.createElement("p");
  credit.className = "coverage";
  credit.textContent = result.attribution;
  container.append(credit);
}

el("save-crypto-key").addEventListener("click", async () => {
  const input = el("crypto-key");
  try {
    await invoke("set_crypto_api_key", { key: input.value });
    input.value = "";   // never leave a key in the DOM
    await refreshCryptoStatus();
    status("Price feed key saved to your system keyring.");
  } catch (e) {
    status(describe(e));
  }
});

el("clear-crypto-key").addEventListener("click", async () => {
  try {
    await invoke("set_crypto_api_key", { key: "" });
    el("crypto-key").value = "";
    await refreshCryptoStatus();
    status("Price feed key removed.");
  } catch (e) {
    status(describe(e));
  }
});

async function loadPresets() {
  try {
    presetCache = await invoke("bullion_presets");
    const select = el("metal-preset");
    for (const preset of presetCache) {
      const option = document.createElement("option");
      option.value = preset.id;
      option.textContent = preset.label;
      select.append(option);
    }
  } catch (e) {
    // Presets are static; a failure here just leaves the custom path.
  }
}

el("metal-preset").addEventListener("change", () => {
  const preset = presetCache.find((p) => p.id === el("metal-preset").value);
  if (!preset) return;
  el("metal-which").value = preset.metal;
  el("metal-weight").value = preset.weight;
  el("metal-unit").value = preset.unit;
  el("metal-basis").value = preset.basis;
  el("metal-purity").value = preset.purity;
});

el("metal-calc").addEventListener("click", async () => {
  try {
    const result = await invoke("value_metal_holding", {
      request: {
        metal: el("metal-which").value,
        quantity: el("metal-qty").value || "1",
        weight_per_item: el("metal-weight").value,
        unit: el("metal-unit").value,
        basis: el("metal-basis").value,
        purity: el("metal-purity").value,
        premium_pct: el("metal-premium").value || null,
      },
    });
    renderMetalResult(result);
    status("");
  } catch (e) {
    el("metal-result").textContent = "";
    status(describe(e));
  }
});

/**
 * Melt and market are shown as separate lines.
 *
 * A graded coin's premium can dwarf its metal content, so collapsing them
 * into one number would hide which part is metal and which is collectibility.
 */
function renderMetalResult(result) {
  const container = el("metal-result");
  container.innerHTML = "";

  const list = document.createElement("dl");
  const rows = [
    ["Fine metal", `${result.fine_troy_oz} troy oz`],
    ["Melt value", result.melt],
    ["Premium", result.premium_amount],
    ["Market value", result.market],
    ["Spot used", `${result.spot_used} ${result.currency}/oz`],
  ];
  for (const [label, value] of rows) {
    const dt = document.createElement("dt");
    dt.textContent = label;
    const dd = document.createElement("dd");
    dd.textContent = value;
    list.append(dt, dd);
  }
  container.append(list);

  if (result.needs_caveat) {
    const caveat = document.createElement("p");
    caveat.className = "caveat";
    caveat.textContent =
      `Based on a ${result.freshness} spot price from ${result.source_asof}. ` +
      `Enter a current price for an up-to-date figure.`;
    container.append(caveat);
  }
}

const SVG_NS = "http://www.w3.org/2000/svg";

function svg(tag, attrs = {}) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, value);
  return node;
}

/**
 * Draw the portfolio value series.
 *
 * Semantics live in docs/chart-semantics.md. Two rules are visible here:
 * segments where any holding was unpriced are dashed, so a partial total
 * never looks complete; and dates carrying a quantity event are marked, so a
 * step can be attributed to a purchase rather than read as appreciation.
 *
 * Deliberately labelled "value", never "return" — the app does not keep the
 * per-flow data needed to separate buying more from gaining value.
 */
async function refreshChart() {
  const figure = el("chart-figure");
  try {
    const series = await invoke("portfolio_series", { maxPoints: 120 });
    const points = series.points ?? [];

    if (points.length < 2) {
      figure.hidden = true;   // a single point is not a trend
      return;
    }

    const chart = el("chart");
    chart.innerHTML = "";
    const width = 640;
    const height = 180;
    const pad = { top: 12, right: 12, bottom: 22, left: 12 };

    // Minor units are integers-as-strings; Number() is safe for plotting
    // (pixels), never for arithmetic on the stored amount.
    const values = points.map((p) => Number(p.total_minor));
    const maxValue = Math.max(...values, 1);
    const plotWidth = width - pad.left - pad.right;
    const plotHeight = height - pad.top - pad.bottom;

    const x = (i) => pad.left + (i / (points.length - 1)) * plotWidth;
    const y = (v) => pad.top + plotHeight - (v / maxValue) * plotHeight;

    // Baseline.
    chart.append(svg("line", {
      class: "axis",
      x1: pad.left, y1: pad.top + plotHeight,
      x2: pad.left + plotWidth, y2: pad.top + plotHeight,
    }));

    // Split into runs of equal coverage so partial stretches can be dashed.
    let run = [];
    let runPartial = points[0].unvalued > 0;

    const flush = () => {
      if (run.length < 2) return;
      chart.append(svg("polyline", {
        class: runPartial ? "line line-partial" : "line",
        points: run.join(" "),
      }));
    };

    points.forEach((point, i) => {
      const partial = point.unvalued > 0;
      const coord = `${x(i)},${y(values[i])}`;

      if (partial !== runPartial && run.length) {
        run.push(coord);       // bridge the join so there is no gap
        flush();
        run = [coord];
        runPartial = partial;
      } else {
        run.push(coord);
      }
    });
    flush();

    // Mark dates where a quantity event took effect.
    points.forEach((point, i) => {
      if (!point.quantity_event) return;
      const marker = svg("circle", {
        class: "event-marker", cx: x(i), cy: y(values[i]), r: 3.5,
      });
      const title = document.createElementNS(SVG_NS, "title");
      title.textContent = `${point.date}: holding changed`;
      marker.append(title);
      chart.append(marker);
    });

    // End labels only: a dense axis on a 180px-tall chart is noise.
    const first = svg("text", {
      class: "axis-label", x: pad.left, y: height - 6,
    });
    first.textContent = points[0].date;
    const last = svg("text", {
      class: "axis-label", x: pad.left + plotWidth, y: height - 6, "text-anchor": "end",
    });
    last.textContent = points[points.length - 1].date;
    chart.append(first, last);

    const peak = svg("text", { class: "axis-label", x: pad.left, y: pad.top - 2 });
    peak.textContent = points.reduce(
      (best, p) => (Number(p.total_minor) >= Number(best.total_minor) ? p : best),
      points[0]
    ).total;
    chart.append(peak);

    // State the caveat rather than letting a dashed line speak for itself.
    const notes = [];
    if (!series.complete) {
      notes.push("Dashed where some holdings had no price on that date.");
    }
    if (series.skipped_currencies.length) {
      notes.push(`Excludes ${series.skipped_currencies.join(", ")} — no conversion yet.`);
    }
    notes.push("Dots mark dates when a holding changed; a step there is a purchase or sale, not a gain.");
    el("chart-note").textContent = notes.join(" ");

    figure.hidden = false;
  } catch (e) {
    figure.hidden = true;   // a broken chart is worse than none
  }
}

async function refreshPortfolio() {
  try {
    const view = await invoke("portfolio_total");
    el("portfolio-total").textContent = view.total;

    // Coverage is shown alongside the total: an unpriced holding must be
    // visible, or a partial figure looks complete.
    const parts = [];
    if (view.unvalued > 0) {
      parts.push(`${view.valued} of ${view.valued + view.unvalued} priced`);
    }
    if (view.skipped_currencies.length) {
      parts.push(`excludes ${view.skipped_currencies.join(", ")} — no conversion yet`);
    }
    el("portfolio-coverage").textContent = parts.length ? `(${parts.join("; ")})` : "";
  } catch (e) {
    el("portfolio-total").textContent = "—";
    el("portfolio-coverage").textContent = "";
  }
}

el("qty-apply").addEventListener("click", async () => {
  if (!currentAsset) return;
  const amount = el("qty-amount").value.trim();
  if (!amount) return status("Enter a quantity.");

  try {
    await invoke("change_quantity", {
      change: {
        asset_id: currentAsset.asset_id,
        kind: el("qty-kind").value,
        quantity: amount,
        effective_date: el("qty-date").value || null,
        note: null,
      },
    });
    el("qty-amount").value = "";
    await refresh();
    await showValuationHistory();
    status("");
  } catch (e) {
    status(describe(e));
  }
});

async function showValuationHistory() {
  if (!currentAsset) return;
  try {
    const points = await invoke("valuation_history", { assetId: currentAsset.asset_id });
    const tbody = el("valuation-history").querySelector("tbody");
    tbody.innerHTML = "";

    for (const point of points) {
      const tr = document.createElement("tr");
      for (const cell of [
        point.asof,
        point.amount,
        point.quantity_at_time,
        point.basis,
        point.provenance,
      ]) {
        const td = document.createElement("td");
        td.textContent = cell;
        tr.append(td);
      }
      tbody.append(tr);
    }
  } catch (e) {
    status(describe(e));
  }
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
