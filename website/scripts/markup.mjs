// Render the app's help markup to escaped, static HTML, including real links.
export const escape = value => String(value).replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const pattern = /\*\*(.+?)\*\*|«([^»]+)»|\[\[([^\]|]+)(?:\|([^\]]+))?\]\]|\{\{([^}|]+)\|([^}]+)\}\}/g;

export function inline(text, topics) {
  let html = "", last = 0;
  for (const m of text.matchAll(pattern)) {
    html += escape(text.slice(last, m.index));
    const [, bold, ui, id, label, screen, screenLabel] = m;
    if (bold) html += `<strong>${escape(bold)}</strong>`;
    else if (ui) html += `<span class="control">${escape(ui)}</span>`;
    else if (id) {
      if (!topics.has(id)) throw new Error(`Unknown help topic: ${id}`);
      html += `<a href="/guide/${escape(id)}/">${escape(label ?? topics.get(id).title)}</a>`;
    } else if (screen) html += `<strong>${escape(screenLabel)}</strong>`;
    last = m.index + m[0].length;
  }
  return html + escape(text.slice(last));
}

export function blocks(body, topics) {
  const out = [];
  let list = null, para = [];
  const paragraph = () => { if (para.length) out.push(`<p>${inline(para.join(" "), topics)}</p>`); para = []; };
  const endList = () => { if (list) out.push(`<${list.kind}>${list.items.map(i => `<li>${inline(i, topics)}</li>`).join("")}</${list.kind}>`); list = null; };
  const flush = () => { paragraph(); endList(); };
  for (const raw of body.split("\n")) {
    const line = raw.trimEnd();
    let m;
    if (!line.trim()) { flush(); continue; }
    if ((m = line.match(/^## (.+)/))) {
      flush(); out.push(`<h2>${inline(m[1], topics)}</h2>`);
    } else if ((m = line.match(/^(\d+)\. (.+)/)) || (m = line.match(/^(-) (.+)/))) {
      paragraph();
      const kind = m[1] === "-" ? "ul" : "ol";
      if (list && list.kind !== kind) endList();
      list ??= { kind, items: [] };
      list.items.push(m[2]);
    } else if (/^\s/.test(raw) && list) {
      list.items[list.items.length - 1] += ` ${line.trim()}`;
    } else if ((m = line.match(/^([>!]) (.+)/))) {
      flush();
      out.push(`<aside class="callout${m[1] === "!" ? " warning" : ""}"><strong>${m[1] === "!" ? "Keep in mind" : "Good to know"}</strong><p>${inline(m[2], topics)}</p></aside>`);
    } else { endList(); para.push(line.trim()); }
  }
  flush();
  return out.join("\n");
}
