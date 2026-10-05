// An in-app viewer for PDF documents in the vault.
//
// The bytes come over IPC — decrypted in memory, never written to disk —
// and pdf.js draws each page onto a canvas. Nothing runs
// from the document: no scripts, no forms, no eval, no WebAssembly. The
// library loads on first use so it costs nothing until a PDF is opened.

import { h } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import { call } from "../lib/api.js";
import { modal } from "./components.js";

const ZOOMS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3];

let library = null;
async function pdfjs() {
  if (!library) {
    library = Promise.all([import("pdfjs-dist"), import("pdfjs-dist/build/pdf.worker.min.mjs?url")]).then(([lib, worker]) => {
      lib.GlobalWorkerOptions.workerSrc = worker.default;
      return lib;
    });
  }
  return library;
}

/**
 * Open a PDF attached to an asset. `title` heads the viewer; `actions` are
 * extra header buttons (e.g. "Save a copy…").
 */
export async function viewPdf(assetId, objectId, { title, actions = [] } = {}) {
  const pages = h("div", { class: "pdf-pages", tabindex: "0", "aria-label": `${title}, document` });
  const status = h("span", { class: "pdf-status", "aria-live": "polite" }, "Opening…");
  const zoomLabel = h("span", { class: "pdf-zoom" }, "Fit");
  const toolbar = h("div", { class: "pdf-toolbar" },
    h("button", { class: "icon-btn", "aria-label": "Previous page", onclick: () => go(-1) }, icon("chevron_up", { size: 16 })),
    h("button", { class: "icon-btn", "aria-label": "Next page", onclick: () => go(1) }, icon("down", { size: 16 })),
    status,
    h("span", { class: "pdf-spacer" }),
    h("button", { class: "icon-btn", "aria-label": "Zoom out", onclick: () => zoom(-1) }, "−"),
    zoomLabel,
    h("button", { class: "icon-btn", "aria-label": "Zoom in", onclick: () => zoom(1) }, "+"),
    h("button", { class: "btn btn-ghost btn-sm", onclick: () => setScale(null) }, "Fit width"),
    ...actions
  );
  const m = modal({ title, size: "full", body: h("div", { class: "pdf-viewer" }, toolbar, pages) });

  let doc = null;
  let task = null;
  let closed = false;
  let scale = null; // null: fit to width
  let effective = 1; // the scale pages are laid out at
  let slots = [];
  const rendering = new Map();
  const observer = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) draw(Number(e.target.dataset.page));
    current();
  }, { root: pages, rootMargin: "400px 0px" });

  m.done.then(() => {
    closed = true;
    observer.disconnect();
    // Decrypted pages must not outlive the viewer.
    for (const s of slots) s.canvas.width = s.canvas.height = 0;
    task?.destroy();
  });

  pages.addEventListener("scroll", () => current(), { passive: true });
  pages.addEventListener("keydown", (e) => {
    if (e.key === "PageDown" || (e.key === "ArrowRight" && !e.altKey)) { e.preventDefault(); go(1); }
    if (e.key === "PageUp" || (e.key === "ArrowLeft" && !e.altKey)) { e.preventDefault(); go(-1); }
    if ((e.ctrlKey || e.metaKey) && (e.key === "=" || e.key === "+")) { e.preventDefault(); zoom(1); }
    if ((e.ctrlKey || e.metaKey) && e.key === "-") { e.preventDefault(); zoom(-1); }
  });

  try {
    // Not fetch() from the asset:// handler: to WebKit that is another
    // origin, and a script may not read it.
    const [lib, bytes] = await Promise.all([pdfjs(), call("read_attachment", { assetId, objectId })]);
    const data = new Uint8Array(bytes);
    if (closed) return;
    const base = new URL("pdfjs/", document.baseURI).href;
    task = lib.getDocument({
      data,
      cMapUrl: `${base}cmaps/`,
      cMapPacked: true,
      standardFontDataUrl: `${base}standard_fonts/`,
      wasmUrl: `${base}wasm/`,
      useWasm: false,
      isEvalSupported: false,
      enableXfa: false,
    });
    doc = await task.promise;
    if (closed) return;
    await layout();
    pages.focus();
  } catch (e) {
    if (closed) return;
    status.textContent = "";
    pages.replaceChildren(h("p", { class: "pdf-error" }, e?.name === "PasswordException"
      ? "This PDF is password-protected. Save a copy and open it in a PDF reader to enter the password."
      : `This PDF could not be shown (${e?.message ?? e}). Save a copy to open it in another app.`));
  }

  /** One sized placeholder per page; pages draw as they scroll into view. */
  async function layout() {
    const first = await doc.getPage(1);
    const natural = first.getViewport({ scale: 1 });
    const fit = Math.max(0.25, (pages.clientWidth - 32) / natural.width);
    const s = scale ?? fit;
    effective = s;
    zoomLabel.textContent = scale == null ? "Fit" : `${Math.round(scale * 100)}%`;
    observer.disconnect();
    rendering.clear();
    for (const old of slots) old.canvas.width = old.canvas.height = 0;
    slots = [];
    const frag = [];
    for (let n = 1; n <= doc.numPages; n++) {
      // Pages share the first page's size until drawn; most documents are uniform.
      const canvas = h("canvas", { "aria-hidden": "true" });
      const el = h("div", { class: "pdf-page", "data-page": String(n), role: "img", "aria-label": `Page ${n} of ${doc.numPages}`, style: { width: `${natural.width * s}px`, height: `${natural.height * s}px` } }, canvas);
      slots.push({ el, canvas, drawnAt: null });
      frag.push(el);
    }
    pages.replaceChildren(...frag);
    for (const s of slots) observer.observe(s.el);
    current();
  }

  async function draw(n) {
    const slot = slots[n - 1];
    const s = effective;
    if (!slot || slot.drawnAt === s || rendering.has(n) || closed) return;
    const page = await doc.getPage(n);
    if (s !== effective || closed) return; // zoomed or closed meanwhile
    const viewport = page.getViewport({ scale: s });
    const ratio = window.devicePixelRatio || 1;
    const canvas = slot.canvas;
    canvas.width = Math.floor(viewport.width * ratio);
    canvas.height = Math.floor(viewport.height * ratio);
    slot.el.style.width = `${viewport.width}px`;
    slot.el.style.height = `${viewport.height}px`;
    const job = page.render({ canvas, viewport, transform: ratio !== 1 ? [ratio, 0, 0, ratio, 0, 0] : null });
    rendering.set(n, job);
    try {
      await job.promise;
      slot.drawnAt = s;
    } catch {
      // Cancelled by a zoom or by closing.
    } finally {
      rendering.delete(n);
    }
  }

  function visiblePage() {
    const top = pages.scrollTop + pages.clientHeight / 3;
    const i = slots.findIndex((s) => s.el.offsetTop + s.el.offsetHeight > top);
    return i < 0 ? slots.length : i + 1;
  }

  function current() {
    if (doc) status.textContent = `Page ${visiblePage()} of ${doc.numPages}`;
  }

  function go(d) {
    if (!doc) return;
    const n = Math.min(doc.numPages, Math.max(1, visiblePage() + d));
    pages.scrollTo({ top: slots[n - 1].el.offsetTop - 12, behavior: "smooth" });
  }

  function setScale(next) {
    if (!doc) return;
    const page = visiblePage();
    for (const job of rendering.values()) job.cancel();
    scale = next;
    layout().then(() => pages.scrollTo({ top: slots[page - 1].el.offsetTop - 12 }));
  }

  function zoom(d) {
    if (!doc) return;
    const now = effective;
    const next = d > 0 ? ZOOMS.find((z) => z > now + 0.01) : [...ZOOMS].reverse().find((z) => z < now - 0.01);
    if (next) setScale(next);
  }
}
