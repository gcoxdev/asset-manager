// Dark mode is defined twice in style.css — once for "follow the computer"
// (inside a prefers-color-scheme query) and once for "always dark". CSS has
// no way to share one block between the two, so this keeps them identical.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";

const css = readFileSync(new URL("../src/style.css", import.meta.url), "utf8");

/** The declarations inside the block that starts at `selector {`. */
function declarations(selector) {
  const at = css.indexOf(`${selector} {`);
  assert.ok(at >= 0, `no block for ${selector}`);
  const body = css.slice(css.indexOf("{", at) + 1, css.indexOf("}", at));
  return body.split(";").map((d) => d.replace(/\s+/g, " ").trim()).filter(Boolean);
}

test("the system-dark and chosen-dark palettes are the same", () => {
  const system = declarations(':root:not([data-theme="light"])');
  const chosen = declarations(':root[data-theme="dark"]');
  assert.ok(system.length > 20, "found the palette");
  assert.deepEqual(chosen, system);
});

test("the system-dark palette applies only inside the dark-scheme query", () => {
  const query = css.indexOf("@media (prefers-color-scheme: dark)");
  const block = css.indexOf(':root:not([data-theme="light"])');
  assert.ok(query >= 0 && block > query && block - query < 80, "the :not(light) block sits inside the query");
  assert.equal(css.match(/prefers-color-scheme/g).length, 1, "one query; anything else would escape the choice");
});

test("the app never sets the native window theme", () => {
  // On Linux, setTheme(null) ("follow the system") switches GTK's
  // prefer-dark flag off, and the WebView then reports light on a dark
  // desktop. The CSS handles every choice on its own.
  const src = new URL("../src", import.meta.url).pathname;
  const files = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? files(`${dir}/${e.name}`) : e.name.endsWith(".js") ? [`${dir}/${e.name}`] : []);
  const offenders = files(src).filter((f) => /\.setTheme\(/.test(readFileSync(f, "utf8")));
  assert.deepEqual(offenders, []);
});
