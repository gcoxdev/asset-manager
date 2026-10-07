#!/usr/bin/env node
// Build wrapper.
//
// Why this exists rather than `tauri build ... && node rename.mjs` in a npm
// script: npm appends forwarded arguments to the END of the whole script
// string, so on an `A && B` chain they land on B. Verified:
//
//   $ npm run chained -- --verbose
//   tauri-build --bundles appimage
//   renamer appimage --verbose        <- went to the renamer, not tauri
//
// Here, extra arguments are passed explicitly to Tauri, and renaming happens
// afterwards as a separate step.

import { argument, outputDirectory, artifactSnapshot, freshArtifact } from "./artifact-paths.mjs";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { chmod, copyFile, stat, unlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const TARGETS = {
  native: { args: [], bundles: process.platform === "win32" ? ["msi"] : process.platform === "darwin" ? ["dmg"] : ["appimage", "deb"] },
  binary: { args: ["--no-bundle"], bundles: [] },
  linux: { args: ["--bundles", "appimage,deb"], bundles: ["appimage", "deb"], noStrip: true },
  appimage: { args: ["--bundles", "appimage"], bundles: ["appimage"], noStrip: true },
  deb: { args: ["--bundles", "deb"], bundles: ["deb"] },
  msi: { args: ["--bundles", "msi"], bundles: ["msi"] },
  dmg: { args: ["--bundles", "dmg"], bundles: ["dmg"] },
  "dmg-universal": {
    args: ["--target", "universal-apple-darwin", "--bundles", "dmg"],
    bundles: ["dmg"],
  },
};

const BUNDLE_LAYOUT = {
  appimage: { dir: ["bundle", "appimage"], ext: ".AppImage" },
  deb: { dir: ["bundle", "deb"], ext: ".deb" },
  dmg: { dir: ["bundle", "dmg"], ext: ".dmg" },
  msi: { dir: ["bundle", "msi"], ext: ".msi" },
};

const [targetName, ...passthrough] = process.argv.slice(2);
const target = TARGETS[targetName];

if (!target) {
  console.error(`Unknown target "${targetName ?? ""}".`);
  console.error(`Valid targets: ${Object.keys(TARGETS).join(", ")}`);
  process.exit(1);
}

// The root package only forwards scripts; build dependencies belong to the
// desktop package. Diagnose a missing/incomplete install before invoking Cargo.
let tauriCli;
try {
  tauriCli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
} catch (error) {
  if (error.code !== "MODULE_NOT_FOUND") throw error;
  console.error(
    "The desktop Tauri CLI is missing or incomplete.\n" +
    "From the repository root, install the build dependencies with:\n" +
    "  npm --prefix apps/desktop ci --include=dev --include=optional\n" +
    "Then retry the build."
  );
  process.exit(1);
}

const env = { ...process.env };
if (target.noStrip) {
  // Without this, linuxdeploy's strip step can break the AppImage on some
  // distributions.
  env.NO_STRIP = "1";
}

const buildArgs = [...target.args, ...passthrough];
const releaseDir = outputDirectory(buildArgs, env);
const bundles = buildArgs.includes("--no-bundle") ? [] : (argument(buildArgs, "--bundles")?.split(",") ?? target.bundles);
const before = new Map();
for (const bundle of bundles) {
  const layout = BUNDLE_LAYOUT[bundle];
  if (!layout) throw new Error(`Unsupported artifact format: ${bundle}`);
  before.set(bundle, await artifactSnapshot(path.join(releaseDir, ...layout.dir), layout.ext));
}

const result = spawnSync(
  process.execPath,
  [tauriCli, "build", ...buildArgs],
  { stdio: "inherit", env, cwd: fileURLToPath(new URL("..", import.meta.url)) }
);

if (result.status !== 0) {
  process.exit(result.status ?? 1);
}

const artifacts = await renameOutputs(bundles);
for (const file of artifacts) await writeChecksum(file);

/**
 * `<artifact>.sha256`, in the format `sha256sum -c` reads. Publish these
 * beside the downloads (and sign them — see docs/releasing.md) so a
 * download can be checked against what was built.
 */
async function writeChecksum(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  const line = `${hash.digest("hex")}  ${path.basename(file)}\n`;
  await writeFile(`${file}.sha256`, line);
  console.log(`→ ${file}.sha256`);
}

/** Give artifacts a stable name so CI and release scripts need not glob. */
async function renameOutputs(bundles) {
  const outputName = validateName(process.env.AM_OUTPUT_NAME) ?? "AssetManager";
  const written = [];
  for (const bundle of bundles) {
    const layout = BUNDLE_LAYOUT[bundle];
    if (!layout) continue;

    const dir = path.join(releaseDir, ...layout.dir);
    const destination = path.join(dir, `${outputName}${layout.ext}`);
    const source = await freshArtifact(dir, layout.ext, before.get(bundle));
    if (path.resolve(source) !== path.resolve(destination)) {
      const mode = (await stat(source)).mode;
      await copyFile(source, destination);
      await chmod(destination, mode);
      await unlink(source);
    }
    console.log(`→ ${destination}`);
    written.push(destination);
  }
  return written;
}

function validateName(value) {
  if (value === undefined) return null;
  const name = value.trim();
  if (!name) throw new Error("AM_OUTPUT_NAME cannot be empty.");
  if (name.length > 120) throw new Error("AM_OUTPUT_NAME cannot exceed 120 characters.");
  if (!/^[A-Za-z0-9][A-Za-z0-9._ -]*[A-Za-z0-9_-]$|^[A-Za-z0-9]$/.test(name)) {
    throw new Error(
      "AM_OUTPUT_NAME must start with a letter or number and contain only letters, " +
        "numbers, spaces, dots, underscores, or hyphens."
    );
  }
  if ([".appimage", ".deb", ".dmg", ".msi", ".exe"].some((e) => name.toLowerCase().endsWith(e))) {
    throw new Error("Set AM_OUTPUT_NAME without a file extension; the build adds it.");
  }
  return name;
}
