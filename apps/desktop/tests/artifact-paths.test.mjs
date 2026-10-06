import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { outputDirectory, artifactSnapshot, freshArtifact } from "../scripts/artifact-paths.mjs";

test("artifact paths honor Cargo metadata, target and profile arguments", () => {
  const root = path.join(tmpdir(), "custom-cargo");
  const metadata = { target_directory: root };
  assert.equal(outputDirectory([], {}, metadata), path.join(root, "release"));
  assert.equal(outputDirectory(["--target", "aarch64-apple-darwin", "--debug"], {}, metadata), path.join(root, "aarch64-apple-darwin", "debug"));
  assert.equal(outputDirectory(["--profile=dist"], { CARGO_BUILD_TARGET: "x86_64-unknown-linux-gnu" }, metadata), path.join(root, "x86_64-unknown-linux-gnu", "dist"));
  assert.throws(() => outputDirectory(["--profile=../../bad"], {}, metadata));
});

test("stale artifacts cannot be published as a new build", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "asset-build-"));
  try {
    const stale = path.join(root, "target", "release");
    const custom = path.join(root, "custom", "release");
    await mkdir(stale, { recursive: true });
    await writeFile(path.join(stale, "old.deb"), "old default output");
    const before = await artifactSnapshot(custom, ".deb");
    await assert.rejects(freshArtifact(custom, ".deb", before));
    await mkdir(custom, { recursive: true });
    const file = path.join(custom, "new.deb");
    await writeFile(file, "new build");
    assert.equal(await freshArtifact(custom, ".deb", before), file);
    const after = await artifactSnapshot(custom, ".deb");
    await assert.rejects(freshArtifact(custom, ".deb", after));
  } finally { await rm(root, { recursive: true, force: true }); }
});
