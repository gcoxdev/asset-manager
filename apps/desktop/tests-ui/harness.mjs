// The stand-in backend and the page setup every browser test shares.

import { readFileSync } from "node:fs";
import { test as base, expect } from "@playwright/test";

// The recorded responses: the test catalog, or the demo one for screenshots.
const fixturesPath = new URL(`./.fixtures/${process.env.UI_FIXTURES ?? "fixtures.json"}`, import.meta.url);
const pdfPath = new URL("./.fixtures/sample.pdf", import.meta.url);
const conf = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));

/** The CSP the app ships with, applied to every page. */
export const CSP = conf.app.security.csp;

let cached = null;
export function fixtures() {
  if (!cached) {
    try {
      cached = JSON.parse(readFileSync(fixturesPath, "utf8"));
    } catch {
      throw new Error("No recorded fixtures. Run `npm run test:ui`, which records them first.");
    }
  }
  return cached;
}

/**
 * Installed before any app script runs. Answers invoke() from the recorded
 * responses — by asset or set ID when the call names one — or from a test's
 * overrides; logs every call; and lists any command it has no answer for.
 */
function installBackend({ calls }) {
  const callbacks = new Map();
  let next = 1;
  const listeners = new Map();
  const state = { calls: [], missing: [], csp: [] };
  window.__backend = state;
  document.addEventListener("securitypolicyviolation", (e) => state.csp.push(`${e.violatedDirective} ${e.blockedURI}`));

  const pick = (cmd, args) => {
    const entries = calls[cmd];
    if (!entries) return undefined;
    const keyed = entries.find((e) => ["assetId", "setId"].some((k) => args?.[k] != null && e.args[k] === args[k]));
    return structuredClone((keyed ?? entries[0]).result);
  };

  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback(cb, once) {
      const id = next++;
      callbacks.set(id, (data) => { if (once) callbacks.delete(id); return cb(data); });
      return id;
    },
    unregisterCallback(id) { callbacks.delete(id); },
    runCallback(id, data) { callbacks.get(id)?.(data); },
    convertFileSrc: (path) => path,
    async invoke(cmd, args = {}) {
      if (cmd === "plugin:event|listen") {
        listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
        return args.handler;
      }
      if (cmd.startsWith("plugin:")) return null;
      state.calls.push({ cmd, args: structuredClone(args) });
      // Raw-byte commands answer with an ArrayBuffer, as Tauri's do.
      if (cmd === "read_attachment") {
        const response = await fetch(`/__test-bytes/${args.assetId}/${args.objectId}`);
        if (!response.ok) throw { kind: "internal", message: "that file is not attached to this asset" };
        return response.arrayBuffer();
      }
      // A test's own answer, worked out on the test side (no eval here:
      // the page runs under the app's CSP).
      const o = await window.__override(cmd, args);
      if (o.handled) {
        if (o.error) throw o.error;
        return o.value;
      }
      const recorded = pick(cmd, args);
      if (recorded !== undefined) return recorded;
      if (["keep_alive"].includes(cmd)) return null;
      state.missing.push(cmd);
      throw { kind: "internal", message: `the test backend has no answer for ${cmd}` };
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  /** Deliver a backend event, as the real app would. */
  window.__emit = (event, payload) => {
    for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id: 0, payload });
  };
}

export const test = base.extend({
  /**
   * Command → value, or `(args) => value`, in place of the recording. A
   * value of `{ error: {...} }` is thrown as the command's error. Set for a
   * group with `test.use({ overrides })`.
   */
  overrides: [{}, { option: true }],

  /** This test's own copy of the overrides, to change while it runs. */
  answers: async ({ overrides }, use) => use({ ...overrides }),

  /** Attachment bytes by `(assetId, objectId)`; null when not attached. */
  media: async ({}, use) =>
    use((assetId, objectId) => (assetId === fixtures().ids.comic && objectId === fixtures().ids.pdf ? readFileSync(pdfPath) : null)),

  page: async ({ page, answers, media }, use) => {
    const data = fixtures();
    await page.exposeFunction("__override", async (cmd, args) => {
      if (!(cmd in answers)) return { handled: false };
      const o = answers[cmd];
      const value = typeof o === "function" ? await o(args) : o;
      return value && typeof value === "object" && "error" in value && Object.keys(value).length === 1
        ? { handled: true, error: value.error }
        : { handled: true, value: value ?? null };
    });
    await page.addInitScript(installBackend, { calls: data.calls });
    // The production CSP on the document, so a feature that needs more than
    // the app allows fails here too.
    await page.route("http://127.0.0.1:4173/", async (route) => {
      const response = await route.fetch();
      await route.fulfill({ response, headers: { ...response.headers(), "content-security-policy": CSP } });
    });
    // Vault media, as the asset:// handler would serve it — to images
    // only. The handler is another origin to the page and sends no CORS
    // headers, so WebKit refuses a script that tries to read it; refuse the
    // same here rather than let such code pass.
    await page.route("http://asset.localhost/media/**", async (route) => {
      if (["fetch", "xhr"].includes(route.request().resourceType())) return route.abort("accessdenied");
      return route.fulfill({ status: 404 });
    });
    // What read_attachment returns, by asset and object.
    await page.route("http://127.0.0.1:4173/__test-bytes/**", async (route) => {
      const [, , assetId, objectId] = new URL(route.request().url()).pathname.split("/");
      const body = media(assetId, objectId);
      return body ? route.fulfill({ status: 200, body }) : route.fulfill({ status: 404 });
    });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await use(page);
    const backend = await page.evaluate(() => window.__backend).catch(() => null);
    expect(errors, "uncaught errors in the page").toEqual([]);
    if (backend) {
      expect(backend.missing, "commands with no recorded answer").toEqual([]);
      expect(backend.csp, "content security policy violations").toEqual([]);
    }
  },
});

export { expect };

/** Calls the page made to a command, with their arguments. */
export async function callsTo(page, cmd) {
  return page.evaluate((c) => window.__backend.calls.filter((x) => x.cmd === c).map((x) => x.args), cmd);
}
