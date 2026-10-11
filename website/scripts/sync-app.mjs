// Export reviewed, public app content. The standalone website build never
// reads outside website/. No vault, fixture responses or credentials are copied.
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { resolve, dirname } from "node:path";
import { SECTIONS, TOPICS } from "../../apps/desktop/src/help/topics.js";

const root = fileURLToPath(new URL("../", import.meta.url));
const check = process.argv.includes("--check");
const files = new Map([
  ["content/help.json", Buffer.from(JSON.stringify({ sections: SECTIONS, topics: TOPICS }, null, 2) + "\n")],
]);
for (const [from, to] of [
  ["apps/desktop/src/help/search.js", "public/assets/help-search.js"],
  ["apps/desktop/src-tauri/icons/icon.svg", "public/assets/icon.svg"],
  ...["overview", "holdings", "unlock"].map(name => [`docs/screenshots/${name}.png`, `public/assets/screenshots/${name}.png`]),
]) files.set(to, await readFile(resolve(root, "..", from)));

for (const [name, content] of files) {
  const destination = resolve(root, name);
  if (check) {
    const current = await readFile(destination).catch(() => null);
    if (!current?.equals(content)) throw new Error(`${name} is out of date. Run npm --prefix website run sync:app.`);
  } else {
    await mkdir(dirname(destination), { recursive: true });
    await writeFile(destination, content);
  }
}
console.log(`${check ? "Verified" : "Synced"} ${TOPICS.length} help topics, search, icon, and 3 dummy-catalog screenshots.`);
