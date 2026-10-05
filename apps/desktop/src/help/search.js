// Searching the help: pure functions, no DOM, so they can be tested alone.
//
// A query matches a topic when every word in it is found — as the start of
// a word, after trimming endings like -ing and -s — in the topic's title,
// keywords, summary or text, or a synonym of it is. "back" finds "backup",
// "delete" finds "Deleting", "password" finds "passphrase". Title hits weigh
// most (more when they cover more of the title), then keywords, the
// summary, and the text.

/** Words people use that the help (and the app) call something else. */
const SYNONYMS = {
  password: ["passphrase"],
  pass: ["passphrase"],
  login: ["unlock"],
  log: ["unlock", "lock"],
  signin: ["unlock"],
  logout: ["lock"],
  delete: ["trash"],
  remove: ["trash"],
  bin: ["trash"],
  undelete: ["restore", "trash"],
  undo: ["history", "restore"],
  revert: ["history", "restore"],
  price: ["value", "spot"],
  prices: ["value", "spot"],
  worth: ["value"],
  appraise: ["value", "appraisal"],
  valuation: ["value"],
  picture: ["photo"],
  pictures: ["photo"],
  image: ["photo"],
  images: ["photo"],
  receipt: ["document"],
  receipts: ["document"],
  paperwork: ["document"],
  file: ["document"],
  files: ["document"],
  excel: ["spreadsheet", "csv"],
  sheets: ["spreadsheet", "csv"],
  export: ["csv", "export"],
  stolen: ["lost", "claim"],
  theft: ["lost", "claim"],
  fire: ["claim"],
  flood: ["claim"],
  damage: ["claim"],
  damaged: ["claim"],
  insurer: ["insurance"],
  sell: ["sold"],
  sale: ["sold"],
  buy: ["bought"],
  purchase: ["bought", "purchase"],
  gold: ["metal"],
  silver: ["metal"],
  bullion: ["metal"],
  bitcoin: ["crypto"],
  btc: ["crypto"],
  ethereum: ["crypto"],
  car: ["vehicle"],
  house: ["property"],
  loan: ["custody"],
  lend: ["custody", "lent"],
  borrowed: ["custody"],
  warranty: ["care"],
  repair: ["care", "custody"],
  barcode: ["label"],
  qr: ["label"],
  audit: ["inventory"],
  stocktake: ["inventory"],
  migrate: ["restore", "import"],
  computer: ["restore"],
  money: ["currency"],
  fx: ["exchange"],
  currency: ["currency", "exchange"],
  profit: ["gain"],
  loss: ["gain", "claim"],
  key: ["recovery", "key"],
  forgot: ["forgot", "recovery"],
  forgotten: ["forgot", "recovery"],
  graph: ["chart"],
  dashboard: ["overview"],
  collection: ["set", "collection"],
  group: ["set"],
  hotkey: ["shortcut"],
  hotkeys: ["shortcut"],
  keyboard: ["shortcut", "keyboard"],
  privacy: ["privacy", "network"],
  online: ["network", "internet"],
};

const WEIGHTS = { title: 12, keywords: 8, summary: 5, text: 1 };

export function normalize(s) {
  return s.toLowerCase().normalize("NFD").replace(/[̀-ͯ]/g, "");
}

/** Words that say nothing about what is wanted. */
const STOP = new Set("a an and are as at be by can do does for from how i if in is it its me my of on or the to what when where which with you your".split(" "));

/** A light stemmer: enough that "deleting", "deleted" and "delete" meet. */
export function stem(w) {
  if (w.length > 5 && w.endsWith("ing")) w = w.slice(0, -3);
  else if (w.length > 4 && w.endsWith("ed")) w = w.slice(0, -2);
  else if (w.length > 4 && w.endsWith("es")) w = w.slice(0, -2);
  else if (w.length > 3 && w.endsWith("s") && !w.endsWith("ss")) w = w.slice(0, -1);
  if (w.length > 4 && w.endsWith("e")) w = w.slice(0, -1);
  return w;
}

function words(s) {
  return normalize(s).split(/[^a-z0-9]+/).filter((w) => w && !STOP.has(w)).map(stem);
}

/** The text of a topic body with its markup removed. */
export function plainText(body) {
  return body
    .replace(/\[\[([^\]|]+)\|([^\]]+)\]\]/g, "$2")
    .replace(/\[\[([^\]]+)\]\]/g, "$1")
    .replace(/\{\{[^}|]+\|([^}]+)\}\}/g, "$1")
    .replace(/[«»]|\*\*/g, "")
    .replace(/^(#+|>|!|-|\d+\.)\s+/gm, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** Build once: each topic's searchable fields as word lists. */
export function buildIndex(topics) {
  return topics.map((topic) => {
    const text = plainText(topic.body);
    return {
      topic,
      text,
      fields: {
        title: words(topic.title),
        keywords: words(topic.keywords ?? ""),
        summary: words(topic.summary ?? ""),
        text: words(text),
      },
    };
  });
}

/** The words of a query, each with the forms it may match. */
export function queryTerms(query) {
  return normalize(query)
    .split(/[^a-z0-9]+/)
    .filter((w) => w && !STOP.has(w) && (w.length > 1 || /\d/.test(w)))
    .map((w) => ({ word: stem(w), forms: [...new Set([stem(w), ...(SYNONYMS[w] ?? []).map(stem)])] }));
}

function termScore(entry, term) {
  // The word as typed counts fully; the best synonym adds half its weight,
  // so a topic that uses the reader's own word ranks above one that only
  // uses ours.
  let direct = 0;
  let synonym = 0;
  for (const form of term.forms) {
    let score = 0;
    for (const [field, list] of Object.entries(entry.fields)) {
      let hits = 0;
      for (const w of list) {
        if (w === form) hits += 1.2;
        else if (w.startsWith(form)) hits += 1;
        if (field !== "text" && hits) break;
      }
      let weight = WEIGHTS[field];
      // A short title the word fills says more than a long one it is in.
      if (field === "title" && list.length) weight *= 0.5 + 0.5 / list.length;
      score += weight * Math.min(hits, field === "text" ? 3 : 1.2);
    }
    if (form === term.word) direct = score;
    else synonym = Math.max(synonym, score);
  }
  return direct + synonym / 2;
}

/**
 * Topics matching `query`, best first: `{ topic, score, snippet, terms }`.
 * When no topic has every word, topics with some of them are returned,
 * flagged `partial`.
 */
export function search(index, query) {
  const terms = queryTerms(query);
  if (!terms.length) return { results: [], partial: false, terms };
  const scored = index.map((entry) => {
    const scores = terms.map((t) => termScore(entry, t));
    return { entry, scores, total: scores.reduce((a, b) => a + b, 0) };
  });
  let matches = scored.filter((s) => s.scores.every((x) => x > 0));
  let partial = false;
  if (!matches.length) {
    matches = scored.filter((s) => s.total > 0);
    partial = matches.length > 0;
  }
  matches.sort((a, b) => b.total - a.total || a.entry.topic.title.localeCompare(b.entry.topic.title));
  return {
    results: matches.map((s) => ({ topic: s.entry.topic, score: s.total, snippet: snippet(s.entry.text, terms) })),
    partial,
    terms,
  };
}

/** A pattern finding any of the terms' forms at the start of a word. */
export function termPattern(terms) {
  const forms = [...new Set(terms.flatMap((t) => t.forms))].sort((a, b) => b.length - a.length);
  if (!forms.length) return null;
  const escaped = forms.map((f) => f.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  return new RegExp(`(?<![\\p{L}\\p{N}])(${escaped.join("|")})[\\p{L}\\p{N}]*`, "giu");
}

/** A stretch of text around the first match, cut at word boundaries. */
export function snippet(text, terms, length = 180) {
  const pattern = termPattern(terms);
  const folded = normalize(text);
  const at = pattern ? folded.search(pattern) : -1;
  if (at < 0) return text.length > length ? `${text.slice(0, length).replace(/\s+\S*$/, "")}…` : text;
  let start = Math.max(0, at - 60);
  if (start > 0) start = text.indexOf(" ", start) + 1;
  let end = Math.min(text.length, start + length);
  if (end < text.length) end = text.lastIndexOf(" ", end);
  return `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`;
}
