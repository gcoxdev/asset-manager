// The help's markup, turned into DOM nodes (see topics.js for the syntax).
// Everything becomes elements and text nodes — nothing is parsed as HTML.

import { h } from "../lib/dom.js";
import { callout } from "../ui/components.js";

const INLINE = /\*\*(.+?)\*\*|«([^»]+)»|\[\[([^\]|]+)(?:\|([^\]]+))?\]\]|\{\{([^}|]+)\|([^}]+)\}\}/g;

/**
 * Inline markup to nodes. `links.topic(id, text)` and `links.screen(view,
 * text)` make the link elements; `links.title(id)` names a topic.
 */
export function inline(text, links) {
  const out = [];
  let last = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const [, bold, ui, topic, topicText, screen, screenText] = m;
    if (bold) out.push(h("strong", {}, bold));
    else if (ui) out.push(h("span", { class: "help-ui" }, ui));
    else if (topic) out.push(links.topic(topic, topicText ?? links.title(topic)));
    else if (screen) out.push(links.screen(screen, screenText));
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** A topic body to block elements. */
export function blocks(body, links) {
  const out = [];
  let list = null; // { kind: "ol" | "ul", items: [string] }
  let para = [];
  const flushPara = () => {
    if (para.length) out.push(h("p", {}, inline(para.join(" "), links)));
    para = [];
  };
  const flushList = () => {
    if (list) out.push(h(list.kind, {}, list.items.map((item) => h("li", {}, inline(item, links)))));
    list = null;
  };
  const flush = () => { flushPara(); flushList(); };

  for (const raw of body.split("\n")) {
    const line = raw.trimEnd();
    if (!line.trim()) { flush(); continue; }
    let m;
    if ((m = line.match(/^## (.+)/))) {
      flush();
      out.push(h("h2", {}, inline(m[1], links)));
    } else if ((m = line.match(/^(\d+)\. (.+)/)) || (m = line.match(/^(-) (.+)/))) {
      flushPara();
      const kind = m[1] === "-" ? "ul" : "ol";
      if (list && list.kind !== kind) flushList();
      list ??= { kind, items: [] };
      list.items.push(m[2]);
    } else if (/^\s/.test(raw) && list) {
      // An indented line continues the item above it.
      list.items[list.items.length - 1] += ` ${line.trim()}`;
    } else if ((m = line.match(/^([>!]) (.+)/))) {
      flush();
      out.push(callout(m[1] === "!" ? "warning" : "info", ...inline(m[2], links)));
    } else {
      flushList();
      para.push(line.trim());
    }
  }
  flush();
  return out;
}

/**
 * Wrap every match of `pattern` in the text under `root` in <mark>. Works on
 * text nodes only, so the structure around them is untouched.
 */
export function highlight(root, pattern) {
  if (!pattern) return [];
  const marks = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes = [];
  while (walker.nextNode()) nodes.push(walker.currentNode);
  for (const node of nodes) {
    const text = node.nodeValue;
    const folded = text.toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "");
    // Folding can change length (rare: "ß"); only mark when it did not.
    if (folded.length !== text.length) continue;
    pattern.lastIndex = 0;
    const parts = [];
    let last = 0;
    for (const m of folded.matchAll(pattern)) {
      if (m.index > last) parts.push(document.createTextNode(text.slice(last, m.index)));
      const mark = h("mark", { class: "help-hit" }, text.slice(m.index, m.index + m[0].length));
      parts.push(mark);
      marks.push(mark);
      last = m.index + m[0].length;
    }
    if (!parts.length) continue;
    if (last < text.length) parts.push(document.createTextNode(text.slice(last)));
    node.replaceWith(...parts);
  }
  return marks;
}
