// Portable mode is spread across the Rust launcher check, the packaging
// scripts, the release workflow and the help. These keep them agreeing.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { MARKER } from "../scripts/portable-marker.mjs";

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const rust = read("../src-tauri/src/portable.rs");
const constant = (name) => rust.match(new RegExp(`pub const ${name}: &str = "([^"]+)"`))[1];

test("the scripts and the app agree on the marker and the data folder", () => {
  assert.equal(constant("MARKER_FILE"), MARKER);
  const help = read("../src/help/topics.js");
  for (const name of [MARKER, constant("DATA_DIRECTORY")]) assert.match(help, new RegExp(name), `help names ${name}`);
  assert.match(read("../src/views/settings.js"), new RegExp(`${constant("DATA_DIRECTORY")}.*${MARKER}`));
});

test("the release workflow builds, checks and collects both portable archives", () => {
  const workflow = read("../../../.github/workflows/release.yml");
  for (const mode of ["appimage", "windows"]) assert.match(workflow, new RegExp(`portable: ${mode}`));
  assert.match(workflow, /node scripts\/enable-portable\.mjs \$\{\{ matrix\.portable \}\}/);
  assert.match(workflow, /node scripts\/package-portable\.mjs \$\{\{ matrix\.portable \}\}/);
  assert.match(workflow, /_portable\.tar\.gz/);
  assert.match(workflow, /_portable\.zip/);
  assert.match(workflow, /-name 'AssetManager\*'/, "collects the AssetManager_… archives too");
});

test("npm scripts build each portable target", () => {
  const scripts = JSON.parse(read("../package.json")).scripts;
  assert.match(scripts["build:linux-appimage-portable"], /enable-portable\.mjs appimage && node scripts\/package-portable\.mjs appimage/);
  assert.match(scripts["build:windows-portable"], /enable-portable\.mjs windows && node scripts\/package-portable\.mjs windows/);
  const root = JSON.parse(read("../../../package.json")).scripts;
  assert.ok(root["build:linux-appimage-portable"] && root["build:windows-portable"]);
});
