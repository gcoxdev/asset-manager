import test from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir, stat } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { blocks, inline } from "../scripts/markup.mjs";
import { buildIndex, search } from "../public/assets/help-search.js";

// Building first also verifies all help references and topic IDs.
await import("../scripts/build.mjs");
const root = fileURLToPath(new URL("../", import.meta.url));
const dist = resolve(root, "dist");
const help = JSON.parse(await readFile(resolve(root, "content/help.json"), "utf8"));
const topics = new Map(help.topics.map(t => [t.id, t]));
const release = JSON.parse(await readFile(resolve(root, "content/release.json"), "utf8"));
const pages = [];
async function walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) await walk(path);
    else if (path.endsWith(".html")) pages.push(path);
  }
}
await walk(dist);

test("help renderer escapes HTML, including markup labels", () => {
  const sample = '**<script>alert(1)</script>** «<img src=x onerror=alert(1)>» [[backups|<b>Backup</b>]] {{settings|<i>Settings</i>}}';
  const html = inline(sample, topics);
  assert.ok(!html.includes("<script>"));
  assert.ok(!html.includes("<img"));
  assert.ok(html.includes('&lt;b&gt;Backup&lt;/b&gt;'));
  assert.ok(html.includes('href="/guide/backups/"'));
  assert.throws(() => inline("[[missing-topic]]", topics), /Unknown help topic/);
});

test("help renderer preserves list continuation, warning and paragraph boundaries", () => {
  const html = blocks("## Heading\n\n1. First step\n   continues here\n2. Second step\n\n! Keep the key\n\nA paragraph.", topics);
  assert.ok(html.includes("<li>First step continues here</li>"));
  assert.ok(html.includes("</ol>\n<aside"));
  assert.ok(html.includes('class="callout warning"'));
  assert.ok(html.endsWith("<p>A paragraph.</p>"));
});

test("every help topic has readable HTML with its full rendered body", async () => {
  assert.equal(pages.length, help.topics.length + 4);
  for (const topic of help.topics) {
    const html = await readFile(resolve(dist, "guide", topic.id, "index.html"), "utf8");
    assert.ok(html.includes(blocks(topic.body, topics)), topic.id);
    assert.equal((html.match(/<h1[ >]/g) ?? []).length, 1, topic.id);
  }
});

test("every local link, fragment, script and image resolves in the isolated output", async () => {
  for (const page of pages) {
    const html = await readFile(page, "utf8");
    for (const [, raw] of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
      if (/^https?:/.test(raw)) continue;
      const [pathname, fragment] = raw.split("#");
      let target = pathname.startsWith("/") ? resolve(dist, "." + pathname) : resolve(dirname(page), pathname || "index.html");
      if ((await stat(target)).isDirectory()) target = resolve(target, "index.html");
      assert.ok((await stat(target)).isFile(), `${page}: ${raw}`);
      if (fragment) {
        const destination = await readFile(target, "utf8");
        assert.ok(destination.includes(`id="${fragment}"`), `${page}: missing ${raw}`);
      }
    }
    assert.ok(!/<(?:script|link)[^>]+(?:src|href)="https?:/.test(html), "No third-party scripts or styles");
  }
});

test("downloads use the selected published release, including portable archives and checksums", async () => {
  const html = await readFile(resolve(dist, "index.html"), "utf8");
  for (const asset of Object.values(release.assets)) {
    assert.ok(html.includes(`${release.repository}/releases/download/v${release.version}/${asset}`), asset);
  }
  assert.ok(!html.includes("/releases/latest"));
  assert.ok(html.includes(release.prerelease ? "Preview release" : "Release"));
});

test("search preserves app synonyms and handles unusual queries", () => {
  const index = buildIndex(help.topics);
  assert.ok(search(index, "forgot password").results.some(r => r.topic.id === "forgot-passphrase"));
  assert.ok(search(index, "backup").results.some(r => r.topic.id === "backups"));
  assert.deepEqual(search(index, "zzzznonexistentword").results, []);
  assert.doesNotThrow(() => search(index, '<img src=x onerror="alert(1)">'));
});
