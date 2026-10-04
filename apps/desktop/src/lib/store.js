// Session-scoped cache of what the views share.
//
// Everything here is decrypted catalog data, so reset() must run on every
// lock — a locked app must not keep a copy of the collection in JS memory
// for the next screen to find.

import { call } from "./api.js";

const state = {
  assets: null,
  types: null,
  settings: null,
  collectibleTypes: null,
  graders: null,
  presets: null,
  coins: null,
  ui: null,
};

const listeners = new Set();

export function onChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function emit(what) {
  for (const fn of listeners) fn(what);
}

export function reset() {
  for (const key of Object.keys(state)) state[key] = null;
}

export async function assets({ fresh = false } = {}) {
  if (fresh || !state.assets) state.assets = await call("list_assets");
  return state.assets;
}

/** Drop cached asset data after a write, and tell open views. */
export function invalidate() {
  state.assets = null;
  emit("assets");
}

export async function types() {
  state.types ??= await call("asset_types");
  return state.types;
}

export async function settings({ fresh = false } = {}) {
  if (fresh || !state.settings) state.settings = await call("get_settings");
  return state.settings;
}

export function setSettings(value) {
  state.settings = value;
}

export async function collectibleTypes() {
  state.collectibleTypes ??= await call("collectible_types");
  return state.collectibleTypes;
}

export async function graders() {
  state.graders ??= await call("graders");
  return state.graders;
}

export async function presets() {
  state.presets ??= await call("bullion_presets");
  return state.presets;
}

export async function coins() {
  state.coins ??= await call("common_coins");
  return state.coins;
}

/**
 * View preferences for this session: search, filters, sort. Cleared on lock
 * with everything else — a search query can itself be sensitive.
 */
export function ui() {
  state.ui ??= { query: "", category: "all", status: "active", sort: "updated", layout: "list", tag: "", location: "", missing: "" };
  return state.ui;
}
