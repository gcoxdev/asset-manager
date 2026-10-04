// Asset Manager — application shell.
//
// The vault path is resolved in the backend, never sent from here: accepting
// a path over IPC would let the UI point vault operations anywhere.
//
// Locking is a backend state transition. This file's job on lock is to make
// the screen match: drop every rendered view and cached record so nothing
// decrypted stays on screen or in memory.
//
// The session has three phases, and every one but "locked" has something to
// clear when the backend locks:
//
//   locked    the unlock or welcome screen; nothing decrypted is held
//   setup     a vault was just created and its recovery key is on screen —
//             the backend session is already open, so it can auto-lock here
//   unlocked  the app proper

import { listen } from "@tauri-apps/api/event";

import { call, endSession, setLockedHandler } from "./lib/api.js";
import { h, mount, clear } from "./lib/dom.js";
import { icon, brandMark } from "./lib/icons.js";
import * as store from "./lib/store.js";
import { toast, closeAllModals, clearToasts, modal } from "./ui/components.js";
import { showOnboarding } from "./views/onboarding.js";
import { renderOverview } from "./views/overview.js";
import { renderHoldings } from "./views/holdings.js";
import { renderAsset } from "./views/asset.js";
import { renderMarkets } from "./views/markets.js";
import { renderReports } from "./views/reports.js";
import { renderSettings, rotateRecovery } from "./views/settings.js";

const app = document.getElementById("app");

const NAV = [
  ["overview", "Overview", "overview"],
  ["holdings", "Holdings", "holdings"],
  ["markets", "Markets", "markets"],
  ["reports", "Reports", "reports"],
  ["settings", "Settings", "settings"],
];

const VIEWS = {
  overview: renderOverview,
  holdings: renderHoldings,
  asset: renderAsset,
  markets: renderMarkets,
  reports: renderReports,
  settings: renderSettings,
};

let current = { view: "overview", params: {} };
let main = null;
let navButtons = new Map();
let cleanup = null;
/** "locked" | "setup" | "unlocked" — see the top of this file. */
let phase = "locked";

/** Navigate to a view. Views receive (container, params, ctx). */
export function navigate(view, params = {}) {
  if (phase !== "unlocked") return;
  current = { view, params };
  renderCurrent();
}

async function renderCurrent({ keepScroll = false } = {}) {
  if (!main) return;
  const scroll = main.scrollTop;
  cleanup?.();
  cleanup = null;

  const section = current.view === "asset" ? "holdings" : current.view;
  for (const [id, button] of navButtons) {
    button.classList.toggle("active", id === section);
    if (id === section) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  }

  const container = h("div", { class: "view" });
  mount(main, container);
  try {
    cleanup = (await VIEWS[current.view](container, current.params, ctx)) ?? null;
  } catch (error) {
    if (error?.kind === "locked") return;
    mount(container, h("div", { class: "view-error" }, h("h2", {}, "This page could not load"), h("p", {}, String(error?.message ?? error))));
  }
  if (keepScroll) main.scrollTop = scroll;
  else main.scrollTop = 0;
}

const ctx = {
  navigate,
  /** Re-render the current view after data changed, keeping the scroll. */
  refresh: () => renderCurrent({ keepScroll: true }),
  lock: () => lockNow(),
  /**
   * The backend now has a different vault open (after a restore): drop
   * everything belonging to the old one and start over in the new one.
   */
  reopen: () => {
    clearSession();
    enterApp();
  },
};

function buildShell() {
  navButtons = new Map();
  const nav = h(
    "nav",
    { class: "nav", "aria-label": "Main" },
    NAV.map(([id, label, glyph]) => {
      const button = h("button", { class: "nav-item", onclick: () => navigate(id) }, icon(glyph, { size: 19 }), h("span", {}, label));
      navButtons.set(id, button);
      return button;
    })
  );

  const lockButton = h("button", { class: "nav-item nav-lock", onclick: () => lockNow() }, icon("lock", { size: 19 }), h("span", {}, "Lock vault"));

  const sidebar = h(
    "aside",
    { class: "sidebar" },
    h("div", { class: "brand" }, brandMark(30), h("div", { class: "brand-text" }, h("strong", {}, "Asset Manager"), h("span", {}, "Encrypted catalog"))),
    nav,
    h("div", { class: "sidebar-foot" }, idleNotice, lockButton)
  );

  main = h("main", { class: "main", id: "main" });
  mount(app, h("div", { class: "shell" }, sidebar, main));
}

// ------------------------------------------------------------ lock & idle

const idleNotice = h("div", { class: "idle-notice", hidden: true });
let idleTimer = null;
let lastKeepAlive = 0;

function onActivity() {
  if (phase === "locked") return;
  const now = Date.now();
  // Reading and scrolling count as presence, but reach the backend at most
  // once every 30 seconds.
  if (now - lastKeepAlive > 30_000) {
    lastKeepAlive = now;
    call("keep_alive").catch(() => {});
  }
  if (!idleNotice.hidden) {
    idleNotice.hidden = true;
  }
}

for (const type of ["pointerdown", "keydown", "wheel", "pointermove"]) {
  window.addEventListener(type, onActivity, { passive: true });
}

/** Warn a minute before the backend's idle lock, rather than dropping the user mid-edit. */
function startIdleWatch() {
  stopIdleWatch();
  idleTimer = setInterval(async () => {
    try {
      const s = await call("session_state");
      if (!s.unlocked) return toLocked("Locked after inactivity.");
      const remaining = s.auto_lock_seconds - (s.idle_seconds ?? 0);
      if (remaining <= 60) {
        idleNotice.hidden = false;
        mount(idleNotice, icon("clock", { size: 16 }), `Locking in ${Math.max(0, remaining)}s — move the mouse to stay`);
      } else {
        idleNotice.hidden = true;
      }
    } catch {
      // Handled by the locked handler.
    }
  }, 10_000);
}

function stopIdleWatch() {
  clearInterval(idleTimer);
  idleTimer = null;
}

async function lockNow() {
  try {
    await call("lock_vault");
  } finally {
    toLocked("Vault locked.");
  }
}

/**
 * Drop everything that belongs to the current session: views, dialogs,
 * caches, print content, and any response still in flight.
 */
function clearSession() {
  phase = "locked";
  endSession();
  stopIdleWatch();
  cleanup?.();
  cleanup = null;
  closeAllModals();
  clearToasts();
  document.querySelector(".menu")?.remove();
  clear(document.getElementById("print-root"));
  store.reset();
  main = null;
  clear(app);
}

/** Leave the app: nothing decrypted may survive on screen or in memory. */
function toLocked(message) {
  // Includes the setup phase: a recovery key on screen when the vault
  // auto-locks must go with it.
  if (phase === "locked") return;
  clearSession();
  showOnboarding(app, { mode: "unlock", ...onboardingHooks, notice: message });
}

setLockedHandler(() => toLocked("The vault locked."));

listen("vault-auto-locked", () => toLocked("Locked after inactivity."));

listen("prices-updated", () => {
  if (phase !== "unlocked") return;
  store.invalidate();
  toast("Spot prices updated in the background.", { kind: "info" });
  if (["overview", "markets", "holdings"].includes(current.view)) ctx.refresh();
});

/** A vault was created and its session opened, before the app is entered. */
function enterSetup() {
  phase = "setup";
  lastKeepAlive = Date.now();
  startIdleWatch();
}

function enterApp() {
  phase = "unlocked";
  lastKeepAlive = Date.now();
  buildShell();
  startIdleWatch();
  navigate("overview");
  remindUnsavedRecoveryKey();
}

const onboardingHooks = { onUnlocked: enterApp, onSessionOpened: enterSetup };

/**
 * The last recovery key issued was never confirmed as saved — the vault
 * locked or the app closed during the ceremony. That key cannot be shown
 * again, so the only fix is a new one.
 */
async function remindUnsavedRecoveryKey() {
  let settings;
  try {
    settings = await store.settings();
  } catch {
    return;
  }
  if (!settings?.recovery_unconfirmed || phase !== "unlocked") return;
  const m = modal({
    title: "Your recovery key was not saved",
    size: "sm",
    body: h("div", { class: "stack" },
      h("p", {}, "The vault locked or closed while your recovery key was on screen, before you confirmed you had saved it. That key cannot be shown again."),
      h("p", {}, "Issue a new one now. Without it, a forgotten passphrase means the catalog is lost.")
    ),
    footer: (close) => [
      h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Later"),
      h("button", { class: "btn btn-primary", onclick: () => close(true) }, "Issue a new key"),
    ],
  });
  if (await m.done) rotateRecovery();
}

// Keyboard: "/" focuses search anywhere it exists.
window.addEventListener("keydown", (event) => {
  if (phase !== "unlocked") return;
  const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName ?? "");
  if (event.key === "/" && !typing && !document.querySelector(".modal-backdrop")) {
    const search = document.querySelector("[data-search]");
    if (search) {
      event.preventDefault();
      search.focus();
    } else {
      navigate("holdings", { focusSearch: true });
    }
  }
});

async function boot() {
  try {
    const status = await call("vault_status");
    if (status.unlocked) return enterApp();
    showOnboarding(app, { mode: status.exists ? "unlock" : "welcome", ...onboardingHooks });
  } catch (error) {
    mount(app, h("div", { class: "fatal" }, h("h1", {}, "Asset Manager could not start"), h("p", {}, String(error?.message ?? error))));
  }
}

boot();
