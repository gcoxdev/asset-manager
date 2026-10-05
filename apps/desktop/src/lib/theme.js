// Light, dark, or whatever the computer uses.
//
// "system" leaves the page to the computer's setting (the CSS follows
// prefers-color-scheme); "light" or "dark" overrides it with a data-theme
// attribute on <html>. The choice is a preference of this computer, kept
// outside the vault, so it applies on the unlock screen too.
//
// The native window theme is deliberately left alone. On Linux, Tauri's
// setTheme(null) — "follow the system" — sets GTK's prefer-dark flag to
// false, which makes the WebView report light even on a dark desktop.
// tests/theme.test.mjs keeps it out.

import { call } from "./api.js";

export const THEMES = [["system", "System"], ["light", "Light"], ["dark", "Dark"]];

let current = "system";

export function theme() {
  return current;
}

/** Show `name` now. */
export function applyTheme(name) {
  current = THEMES.some(([id]) => id === name) ? name : "system";
  if (current === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = current;
}

/** The saved theme, applied. Never fails: the default is fine. */
export async function loadTheme() {
  try {
    applyTheme((await call("get_preferences")).theme);
  } catch {
    applyTheme("system");
  }
}

/** Apply and remember. */
export async function saveTheme(name) {
  applyTheme(name);
  await call("set_theme", { theme: current });
}
