#!/usr/bin/env node
// Put the portable-mode marker beside a built launcher.
//
//   node scripts/enable-portable.mjs appimage   beside AssetManager.AppImage
//   node scripts/enable-portable.mjs windows    beside the standalone .exe
//
// With the marker there, the app keeps its data in AssetManagerData beside
// the launcher instead of the user profile (see src-tauri/src/portable.rs).

import { access, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { outputDirectory } from "./artifact-paths.mjs";
import { MARKER } from "./portable-marker.mjs";

const releaseDir = outputDirectory(process.argv.slice(3));

async function requireFile(file) {
  try {
    await access(file);
  } catch {
    throw new Error(`Expected build output was not found: ${file}`);
  }
}

const mode = process.argv[2];
let dir;
let launchers;
if (mode === "appimage") {
  dir = path.join(releaseDir, "bundle", "appimage");
  launchers = (await readdir(dir).catch(() => [])).filter((f) => f.endsWith(".AppImage"));
  if (!launchers.length) throw new Error(`No AppImage was found in ${dir}. Run npm run build:linux-appimage first.`);
} else if (mode === "windows") {
  dir = releaseDir;
  launchers = ["asset-manager-desktop.exe"];
  await requireFile(path.join(dir, launchers[0]));
} else {
  console.error("Usage: node scripts/enable-portable.mjs appimage|windows");
  process.exit(1);
}

const marker = path.join(dir, MARKER);
await writeFile(marker, "Asset Manager portable mode marker. Keep this file beside the app and move them together.\n", { mode: 0o600 });
console.log(`Portable mode enabled for ${launchers.join(", ")}`);
console.log(`Marker: ${marker}`);
console.log("At first launch, Asset Manager creates AssetManagerData beside it.");
