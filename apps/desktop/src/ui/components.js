// Shared UI pieces: toasts, modals, confirm, menus, form fields, badges.

import { h, clear, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import { describe } from "../lib/api.js";

// ------------------------------------------------------------------ toasts

export function toast(message, { kind = "info", timeout = 4500 } = {}) {
  const host = document.getElementById("toasts");
  const glyph = { success: "check", error: "warn", warning: "warn", info: "info" }[kind];
  const el = h(
    "div",
    { class: `toast toast-${kind}`, role: kind === "error" ? "alert" : "status" },
    icon(glyph, { size: 18 }),
    h("div", { class: "toast-text" }, message),
    h("button", { class: "icon-btn toast-close", "aria-label": "Dismiss", onclick: () => close() }, icon("x", { size: 16 }))
  );
  const close = () => {
    el.classList.add("leaving");
    setTimeout(() => el.remove(), 180);
  };
  host.append(el);
  if (timeout) setTimeout(close, kind === "error" ? timeout * 1.6 : timeout);
  return close;
}

export function toastError(error) {
  // Locking already took the user to the unlock screen; a toast saying so
  // would only land there late.
  if (error?.kind === "locked") return () => {};
  return toast(describe(error), { kind: "error" });
}

export function clearToasts() {
  clear(document.getElementById("toasts"));
}

// ------------------------------------------------------------------ modals

/** Open modals, in stacking order, each with the close() that disposes it. */
const openModals = new Map();

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]):not([type=hidden]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Only the topmost dialog can be reached: the page and any dialog beneath
 * are inert — no focus, no clicks, hidden from assistive technology —
 * until it closes. `aria-modal` alone only asks screen readers to behave.
 */
function syncInert() {
  const stack = [...openModals.keys()];
  const top = stack.at(-1);
  const app = document.getElementById("app");
  if (app) app.inert = stack.length > 0;
  for (const backdrop of stack) backdrop.inert = backdrop !== top;
}

/**
 * Open a modal. `render(close)` returns its body; `footer` is an array of
 * buttons. Resolves when closed, with whatever close() was given.
 */
let modalSeq = 0;

export function modal({ title, subtitle, body, footer = [], size = "md", onClose, dismissable = true }) {
  let resolve;
  const done = new Promise((r) => (resolve = r));
  const previousFocus = document.activeElement;

  // One disposer for every way a modal ends — a button, Escape, or a lock —
  // so listeners are always removed and whoever awaits `done` is answered.
  const close = (value, { immediate = false } = {}) => {
    if (!openModals.has(backdrop)) return;
    openModals.delete(backdrop);
    syncInert();
    document.removeEventListener("keydown", onKey, true);
    if (immediate) {
      backdrop.remove();
    } else {
      backdrop.classList.add("leaving");
      setTimeout(() => backdrop.remove(), 160);
    }
    onClose?.(value);
    resolve(value);
    if (!immediate && previousFocus && document.contains(previousFocus)) previousFocus.focus();
  };

  const onKey = (event) => {
    if (!isTopmost()) return;
    if (event.key === "Escape" && dismissable) {
      event.stopPropagation();
      close(undefined);
    }
    // Tab stays inside the dialog, wrapping at either end.
    if (event.key === "Tab") {
      const items = [...dialog.querySelectorAll(FOCUSABLE)].filter((el) => el.offsetParent !== null || el === document.activeElement);
      if (!items.length) return event.preventDefault();
      const first = items[0];
      const last = items.at(-1);
      const inside = dialog.contains(document.activeElement);
      if (event.shiftKey && (document.activeElement === first || !inside)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && (document.activeElement === last || !inside)) {
        event.preventDefault();
        first.focus();
      }
    }
  };
  const isTopmost = () => [...openModals.keys()].at(-1) === backdrop;

  const content = typeof body === "function" ? body(close) : body;
  const buttons = typeof footer === "function" ? footer(close) : footer;

  const titleId = `modal-title-${++modalSeq}`;
  const dialog = h(
    "div",
    { class: `modal modal-${size}`, role: "dialog", "aria-modal": "true", "aria-labelledby": titleId },
    h(
      "header",
      { class: "modal-head" },
      h("div", {}, h("h2", { class: "modal-title", id: titleId }, title), subtitle ? h("p", { class: "modal-subtitle" }, subtitle) : null),
      dismissable
        ? h("button", { class: "icon-btn", "aria-label": "Close", onclick: () => close(undefined) }, icon("x"))
        : null
    ),
    h("div", { class: "modal-body" }, content),
    buttons.length ? h("footer", { class: "modal-foot" }, buttons) : null
  );

  const backdrop = h("div", { class: "modal-backdrop" }, dialog);
  backdrop.addEventListener("mousedown", (event) => {
    if (event.target === backdrop && dismissable) close(undefined);
  });
  document.addEventListener("keydown", onKey, true);
  openModals.set(backdrop, close);
  document.body.append(backdrop);
  syncInert();

  // Focus the first field, or the primary button.
  requestAnimationFrame(() => {
    const target =
      dialog.querySelector("[autofocus]") ||
      dialog.querySelector(".modal-body input:not([type=hidden]):not([disabled]), .modal-body select, .modal-body textarea") ||
      dialog.querySelector(".modal-foot .btn-primary");
    target?.focus();
  });

  return { close, done, dialog };
}

/** Close every modal at once, through each one's own disposer. */
export function closeAllModals() {
  for (const close of [...openModals.values()].reverse()) close(undefined, { immediate: true });
}

/** A yes/no question. Resolves true only on confirmation. */
export function confirmDialog({ title, message, confirmLabel = "Continue", danger = false, details }) {
  const { done } = modal({
    title,
    size: "sm",
    body: h(
      "div",
      { class: "confirm-body" },
      Array.isArray(message) ? message.map((m) => h("p", {}, m)) : h("p", {}, message),
      details ?? null
    ),
    footer: (close) => [
      h("button", { class: "btn btn-ghost", onclick: () => close(false) }, "Cancel"),
      h(
        "button",
        { class: danger ? "btn btn-danger" : "btn btn-primary", onclick: () => close(true) },
        confirmLabel
      ),
    ],
  });
  return done.then((v) => v === true);
}

/**
 * Run an async action from a button: disable it, show progress, report
 * errors. Returns the action's result, or undefined on failure.
 */
export async function busy(button, action, busyLabel) {
  const original = [...button.childNodes];
  button.disabled = true;
  button.classList.add("is-busy");
  if (busyLabel) {
    clear(button);
    button.append(h("span", { class: "spinner", "aria-hidden": "true" }), busyLabel);
  }
  try {
    return await action();
  } catch (error) {
    toastError(error);
    return undefined;
  } finally {
    if (button.isConnected) {
      button.disabled = false;
      button.classList.remove("is-busy");
      if (busyLabel) {
        clear(button);
        button.append(...original);
      }
    }
  }
}

// ------------------------------------------------------------------ menus

/** A dropdown menu anchored to a button. items: {label, icon, onSelect, danger} */
export function menuButton(trigger, items) {
  trigger.setAttribute("aria-haspopup", "menu");
  trigger.addEventListener("click", (event) => {
    event.stopPropagation();
    const existing = document.querySelector(".menu");
    if (existing) {
      existing.remove();
      if (existing.dataset.owner === trigger.dataset.menuId) return;
    }
    trigger.dataset.menuId ??= String(Math.random());
    const menu = h(
      "div",
      { class: "menu", role: "menu", dataset: { owner: trigger.dataset.menuId } },
      items
        .filter(Boolean)
        .map((item) =>
          item === "divider"
            ? h("div", { class: "menu-divider" })
            : h(
                "button",
                {
                  class: item.danger ? "menu-item danger" : "menu-item",
                  role: "menuitem",
                  onclick: () => {
                    menu.remove();
                    item.onSelect();
                  },
                },
                item.icon ? icon(item.icon, { size: 16 }) : null,
                item.label
              )
        )
    );
    document.body.append(menu);
    const rect = trigger.getBoundingClientRect();
    const width = menu.offsetWidth;
    menu.style.top = `${rect.bottom + 6 + window.scrollY}px`;
    menu.style.left = `${Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8))}px`;
    menu.querySelector("button")?.focus();
    const dismiss = (e) => {
      if (!menu.isConnected || !menu.contains(e.target)) {
        menu.remove();
        document.removeEventListener("mousedown", dismiss);
      }
    };
    setTimeout(() => document.addEventListener("mousedown", dismiss));
    menu.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        menu.remove();
        trigger.focus();
      }
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const buttons = [...menu.querySelectorAll("button")];
        const i = buttons.indexOf(document.activeElement);
        const next = e.key === "ArrowDown" ? (i + 1) % buttons.length : (i - 1 + buttons.length) % buttons.length;
        buttons[next].focus();
      }
    });
  });
  return trigger;
}

// ------------------------------------------------------------------ fields

let fieldSeq = 0;

/** A labelled field wrapping an input. */
export function field(label, input, { hint, span, required, error } = {}) {
  const id = input.id || `f${++fieldSeq}`;
  input.id = id;
  return h(
    "div",
    { class: ["field", span ? `span-${span}` : null, required ? "required" : null] },
    h("label", { for: id }, label),
    input,
    hint ? h("p", { class: "field-hint" }, hint) : null,
    error ? h("p", { class: "field-error" }, error) : null
  );
}

export function textInput(props = {}) {
  return h("input", { type: "text", autocomplete: "off", spellcheck: "false", ...props });
}

export function select(options, value, props = {}) {
  const el = h(
    "select",
    props,
    options.map((o) => {
      const [v, label] = Array.isArray(o) ? o : [o.value, o.label];
      return h("option", { value: v }, label);
    })
  );
  if (value !== undefined && value !== null) el.value = value;
  return el;
}

/** An on/off switch with a text label, backed by a real checkbox. */
export function toggle(label, checked, onChange, { hint } = {}) {
  const input = h("input", { type: "checkbox", class: "switch-input", checked, onchange: () => onChange(input.checked) });
  return h(
    "label",
    { class: "switch" },
    input,
    h("span", { class: "switch-track", "aria-hidden": "true" }, h("span", { class: "switch-thumb" })),
    h("span", { class: "switch-text" }, h("span", {}, label), hint ? h("span", { class: "switch-hint" }, hint) : null)
  );
}

/** Segmented control. Returns {el, value()}. */
export function segmented(options, value, onChange) {
  let current = value;
  const buttons = options.map(([v, label]) =>
    h(
      "button",
      {
        type: "button",
        class: v === current ? "seg active" : "seg",
        "aria-pressed": v === current ? "true" : "false",
        onclick: () => {
          current = v;
          for (const b of buttons) {
            const on = b.dataset.value === v;
            b.classList.toggle("active", on);
            b.setAttribute("aria-pressed", on ? "true" : "false");
          }
          onChange?.(v);
        },
        dataset: { value: v },
      },
      label
    )
  );
  return { el: h("div", { class: "segmented", role: "group" }, buttons), value: () => current };
}

/**
 * Tags as removable pills, with an input that adds one on Enter or comma and
 * suggests tags already in use. Returns {el, input, value()}.
 */
let tagListSeq = 0;
export function tagsInput(initial = [], suggestions = [], { label = "Tags" } = {}) {
  const tags = [...initial];
  const listId = `tag-suggestions-${++tagListSeq}`;
  const pills = h("div", { class: "tag-pills" });
  const input = h("input", { type: "text", list: listId, maxlength: 60, autocomplete: "off", placeholder: tags.length ? "Add another…" : "Type a tag, then Enter", "aria-label": `Add to ${label.toLowerCase()}` });
  const datalist = h("datalist", { id: listId }, suggestions.map((s) => h("option", { value: s })));
  const has = (name) => tags.some((t) => t.toLowerCase() === name.toLowerCase());
  const draw = () => mount(pills, tags.map((t, i) =>
    h("span", { class: "tag-pill" }, t,
      h("button", { type: "button", class: "tag-remove", "aria-label": `Remove tag ${t}`, onclick: () => { tags.splice(i, 1); draw(); } }, icon("x", { size: 12 })))
  ));
  const commit = () => {
    for (const part of input.value.split(",")) {
      const name = part.trim().replace(/\s+/g, " ");
      if (name && !has(name)) tags.push(name);
    }
    input.value = "";
    draw();
  };
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      commit();
    } else if (e.key === "Backspace" && !input.value && tags.length) {
      tags.pop();
      draw();
    }
  });
  input.addEventListener("change", () => { if (input.value.includes(",") || suggestions.includes(input.value)) commit(); });
  draw();
  return {
    el: h("div", { class: "tag-input" }, pills, input, datalist),
    input,
    // A tag typed but not yet committed still counts.
    value: () => { commit(); return [...tags]; },
  };
}

/** A text input that suggests values already in use. */
let suggestSeq = 0;
export function suggestInput(props, suggestions) {
  const id = `suggest-${++suggestSeq}`;
  const input = h("input", { type: "text", autocomplete: "off", list: id, ...props });
  return { input, el: h("span", { class: "suggest" }, input, h("datalist", { id }, suggestions.map((s) => h("option", { value: s })))) };
}

// ------------------------------------------------------------------ badges

/**
 * Where a value came from. Manual is shown neutrally — a price the owner
 * looked up is often more accurate than a thin automated match — and
 * staleness is what gets flagged.
 */
export function sourceBadge(source, asof) {
  if (!source) return h("span", { class: "badge badge-muted" }, "No value");
  const label = { manual: "Manual", api: "Market", appraisal: "Appraisal" }[source] ?? source;
  return h("span", { class: `badge badge-${source}`, title: asof ? `Valued ${asof}` : "" }, label);
}

export function statusBadge(status) {
  if (status === "active") return null;
  const label = { sold: "Sold", lost: "Lost", retired: "Retired" }[status] ?? status;
  return h("span", { class: `badge badge-status-${status}` }, label);
}

export function emptyState({ glyph = "holdings", title, body, actions = [] }) {
  return h(
    "div",
    { class: "empty" },
    h("div", { class: "empty-glyph" }, icon(glyph, { size: 28 })),
    h("h3", {}, title),
    body ? h("p", {}, body) : null,
    actions.length ? h("div", { class: "empty-actions" }, actions) : null
  );
}

export function callout(kind, ...content) {
  const glyph = { warning: "warn", danger: "warn", info: "info", success: "check" }[kind] ?? "info";
  return h("div", { class: `callout callout-${kind}` }, icon(glyph, { size: 18 }), h("div", {}, ...content));
}
