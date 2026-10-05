// The help must describe the app as it is. Every «label» it quotes has to
// appear in the frontend source, every [[link]] has to name a topic, and
// every {{screen}} link a screen — so renaming a button without updating
// the help fails here instead of misleading someone.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { SECTIONS, TOPICS } from "../src/help/topics.js";

const SRC = new URL("../src", import.meta.url).pathname;
const files = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
  e.isDirectory() ? (e.name === "help" ? [] : files(join(dir, e.name))) : e.name.endsWith(".js") ? [join(dir, e.name)] : []);
const source = files(SRC).map((f) => readFileSync(f, "utf8")).join("\n");

// Keys and symbols the help names that are not text in the UI.
const KEYS = new Set(["/", "?", "Enter", "Escape", "Space", "Ctrl", "+", "−", "Page Up", "Page Down", " / "]);

const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

test("every topic has a unique id, a known section, and a body", () => {
  const ids = TOPICS.map((t) => t.id);
  assert.equal(new Set(ids).size, ids.length, "duplicate topic ids");
  const sections = new Set(SECTIONS.map(([id]) => id));
  for (const t of TOPICS) {
    assert.ok(sections.has(t.section), `${t.id}: unknown section ${t.section}`);
    assert.ok(t.title && t.summary && t.body.length > 80, `${t.id}: missing title, summary or body`);
  }
});

test("every «label» the help quotes is in the app", () => {
  const missing = [];
  for (const t of TOPICS) {
    for (const [, label] of t.body.matchAll(/«([^»]+)»/g)) {
      if (KEYS.has(label)) continue;
      // "…" stands for a part that varies: "Correct to …".
      const pattern = new RegExp(label.split("…").map(escape).join("[^\"`]*"));
      if (!pattern.test(source)) missing.push(`${t.id}: «${label}»`);
    }
  }
  assert.deepEqual(missing, []);
});

test("every [[link]] names a topic and every {{screen}} a screen", () => {
  const ids = new Set(TOPICS.map((t) => t.id));
  const screens = new Set(["overview", "holdings", "wishlist", "markets", "reports", "settings", "help"]);
  const broken = [];
  for (const t of TOPICS) {
    for (const [, target] of t.body.matchAll(/\[\[([^\]|]+)(?:\|[^\]]+)?\]\]/g)) if (!ids.has(target)) broken.push(`${t.id} → [[${target}]]`);
    for (const [, screen] of t.body.matchAll(/\{\{([^}|]+)\|[^}]+\}\}/g)) if (!screens.has(screen)) broken.push(`${t.id} → {{${screen}}}`);
  }
  assert.deepEqual(broken, []);
});

// --- search ----------------------------------------------------------------

import { buildIndex, search, snippet, queryTerms } from "../src/help/search.js";

const index = buildIndex(TOPICS);
const top = (q, n = 1) => search(index, q).results.slice(0, n).map((r) => r.topic.id);

test("common questions find the right topic first", () => {
  const cases = {
    "forgot password": "forgot-passphrase",
    "forgot my passphrase": "forgot-passphrase",
    backup: "backups",
    "verify backup": "verify-backup",
    "move to a new computer": "restore",
    "restore backup": "restore",
    "restore from a backup": "restore",
    "spot price": "spot-prices",
    "seed phrase": "crypto",
    "delete an item": "trash",
    "undo an edit": "edit-history",
    "insurance claim": "claim",
    "import excel": "spreadsheet-import",
    "exchange rate": "currencies",
    "qr labels": "labels",
    "change passphrase": "change-passphrase",
    "auto lock": "unlocking",
    "melt calculator": "melt-calculator",
    "wishlist": "wishlist",
    "split": "split",
  };
  const wrong = Object.entries(cases).filter(([q, id]) => top(q)[0] !== id).map(([q, id]) => `${q} → ${top(q, 3).join(", ")} (wanted ${id})`);
  assert.deepEqual(wrong, []);
});

test("synonyms and word starts both match", () => {
  assert.ok(top("stolen", 3).includes("claim"), "stolen → claim");
  assert.ok(top("back", 5).includes("backups"), "prefix: back → backups");
  assert.ok(top("pictures", 3).includes("photos"), "pictures → photos");
});

test("nonsense finds nothing; a partial match says so", () => {
  assert.deepEqual(search(index, "zzzzqqq").results, []);
  const partial = search(index, "backup zzzzqqq");
  assert.equal(partial.partial, true);
  assert.ok(partial.results.length > 0);
});

test("a snippet shows the text around the match", () => {
  const s = snippet("A long text about many things. The recovery key opens the vault. More text follows here.", queryTerms("recovery"));
  assert.match(s, /recovery key/);
});
