import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { readdir, stat } from "node:fs/promises";

const cargoRoot = fileURLToPath(new URL("../src-tauri", import.meta.url));

export function argument(args, flag) {
  let value;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === flag) {
      value = args[++i];
      if (!value || value.startsWith("--")) throw new Error(`${flag} needs a value`);
    } else if (args[i].startsWith(`${flag}=`)) value = args[i].slice(flag.length + 1);
  }
  return value;
}

/** Cargo metadata resolves environment overrides and .cargo configuration. */
export function outputDirectory(args = [], env = process.env, metadata) {
  if (!metadata) {
    const result = spawnSync("cargo", ["metadata", "--offline", "--no-deps", "--format-version", "1"],
      { cwd: cargoRoot, env, encoding: "utf8" });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`Cannot determine Cargo output directory: ${result.stderr}`);
    metadata = JSON.parse(result.stdout);
  }
  const target = argument(args, "--target") ?? env.CARGO_BUILD_TARGET;
  const profile = argument(args, "--profile") ?? (args.includes("--debug") ? "dev" : "release");
  const directory = profile === "dev" ? "debug" : profile;
  if ([target, directory].some((s) => s && (!/^[\w.-]+$/.test(s) || s === "." || s === ".."))) {
    throw new Error("Unsupported target or profile output path");
  }
  return path.join(metadata.target_directory, ...(target ? [target] : []), directory);
}

export async function artifactSnapshot(dir, ext) {
  const files = await readdir(dir, { withFileTypes: true }).catch((e) => {
    if (e.code === "ENOENT") return [];
    throw e;
  });
  return new Map(await Promise.all(files.filter((f) => f.isFile() && f.name.endsWith(ext)).map(async (f) => {
    const file = path.join(dir, f.name);
    const info = await stat(file);
    return [file, `${info.size}:${info.mtimeMs}:${info.ctimeMs}`];
  })));
}

export async function freshArtifact(dir, ext, before) {
  const after = await artifactSnapshot(dir, ext);
  const changed = [...after].filter(([file, signature]) => before.get(file) !== signature).map(([file]) => file);
  if (changed.length !== 1) throw new Error(`Expected one newly built ${ext} artifact in ${dir}; found ${changed.length}`);
  return changed[0];
}
