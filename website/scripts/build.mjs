import { readFile, writeFile, mkdir, rm, cp } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { resolve, dirname } from "node:path";
import { layout, home, guideHome, guideArticle, privacy, notFound } from "./pages.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const out = resolve(root, "dist");
const help = JSON.parse(await readFile(resolve(root, "content/help.json"), "utf8"));
const release = JSON.parse(await readFile(resolve(root, "content/release.json"), "utf8"));
if (!/^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(release.version)) throw new Error("Invalid release version");
if (release.repository !== "https://github.com/gcoxdev/asset-manager") throw new Error("Unexpected download repository");
const ids = new Set();
for (const t of help.topics) {
  if (!/^[a-z0-9-]+$/.test(t.id) || ids.has(t.id)) throw new Error(`Invalid or duplicate topic ID: ${t.id}`);
  ids.add(t.id);
}
await rm(out, { recursive: true, force: true });
await mkdir(out, { recursive: true });
await cp(resolve(root, "public"), out, { recursive: true });
const write = async (path, title, description, body, active = "") => {
  const file = resolve(out, path);
  await mkdir(dirname(file), { recursive: true });
  await writeFile(file, layout({ title, description, body, active }));
};
await write("index.html", "Asset Manager — A place for everything you value", "A private desktop catalog for your collections, valuables and everyday things. Track values, keep receipts, and back up your encrypted vault.", home(release), "product");
await write("guide/index.html", "User guide — Asset Manager", "The complete Asset Manager help guide. Learn about your vault, collections, values, photos, reports, backups and recovery.", guideHome(help), "guide");
for (const topic of help.topics) {
  await write(`guide/${topic.id}/index.html`, `${topic.title} — Asset Manager`, topic.summary, guideArticle(topic, help), "guide");
}
await write("privacy/index.html", "Privacy & your data — Asset Manager", "Understand how Asset Manager stores your catalog, encrypts attachments, handles optional network requests, and keeps recovery in your hands.", privacy(), "privacy");
await write("404.html", "Page not found — Asset Manager", "Find your way back to Asset Manager or search the user guide.", notFound());
await writeFile(resolve(out, "assets/help-index.json"), JSON.stringify(help.topics));
console.log(`Built ${help.topics.length + 4} static pages in website/dist. No runtime server or external API required.`);
