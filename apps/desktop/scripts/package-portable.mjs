#!/usr/bin/env node
// Archive a launcher with its portable-mode marker, for release.
//
//   node scripts/package-portable.mjs appimage
//     → target/release/bundle/portable/AssetManager_<version>_amd64_portable.tar.gz
//   node scripts/package-portable.mjs windows      (on Windows)
//     → target/release/bundle/portable/AssetManager_<version>_x64_portable.zip
//
// Each archive gets a .sha256 beside it, like every other artifact. Run
// enable-portable.mjs first. On Windows, run it after the signed MSI build:
// that build signs the executable in place, so the archive holds the signed
// copy.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { MARKER } from "./portable-marker.mjs";

const desktopRoot = fileURLToPath(new URL("..", import.meta.url));
const repoRoot = path.resolve(desktopRoot, "../..");
const releaseDir = path.join(process.env.CARGO_TARGET_DIR ? path.resolve(repoRoot, process.env.CARGO_TARGET_DIR) : path.join(repoRoot, "target"), "release");
const bundleDir = path.join(releaseDir, "bundle");
const outDir = path.join(bundleDir, "portable");
const { version } = JSON.parse(readFileSync(path.join(desktopRoot, "src-tauri", "tauri.conf.json"), "utf8"));

function requireFile(file) {
  if (!statSync(file, { throwIfNoEntry: false })?.isFile()) throw new Error(`Expected build output was not found: ${file}`);
}

function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed with exit code ${result.status}`);
}

function checksum(file) {
  const digest = createHash("sha256").update(readFileSync(file)).digest("hex");
  writeFileSync(`${file}.sha256`, `${digest}  ${path.basename(file)}\n`);
  console.log(`→ ${file}.sha256`);
}

const mode = process.argv[2];
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

let archive;
if (mode === "appimage") {
  const dir = path.join(bundleDir, "appimage");
  const launcher = readdirSync(dir).filter((f) => f.endsWith(".AppImage"))
    .map((f) => ({ f, t: statSync(path.join(dir, f)).mtimeMs })).sort((a, b) => b.t - a.t)[0]?.f;
  if (!launcher) throw new Error(`No AppImage was found in ${dir}`);
  requireFile(path.join(dir, MARKER));
  archive = path.join(outDir, `AssetManager_${version}_amd64_portable.tar.gz`);
  run("tar", ["-czf", archive, "-C", dir, launcher, MARKER]);
} else if (mode === "windows") {
  const exe = path.join(releaseDir, "asset-manager-desktop.exe");
  requireFile(exe);
  requireFile(path.join(releaseDir, MARKER));
  // Named for people, not for cargo; renaming leaves a signature intact.
  const stage = path.join(outDir, "stage");
  mkdirSync(stage);
  copyFileSync(exe, path.join(stage, "AssetManager.exe"));
  copyFileSync(path.join(releaseDir, MARKER), path.join(stage, MARKER));
  const arch = process.arch === "x64" ? "x64" : process.arch;
  archive = path.join(outDir, `AssetManager_${version}_${arch}_portable.zip`);
  // Windows' own bsdtar writes zip with -a. Named in full: in Git Bash,
  // "tar" is GNU tar, which cannot.
  const tar = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe");
  run(tar, ["-a", "-cf", archive, "-C", stage, "AssetManager.exe", MARKER]);
  rmSync(stage, { recursive: true, force: true });
} else {
  console.error("Usage: node scripts/package-portable.mjs appimage|windows");
  process.exit(1);
}

if (!existsSync(archive)) throw new Error(`The archive was not written: ${archive}`);
checksum(archive);
console.log(`Packaged portable build: ${archive}`);
