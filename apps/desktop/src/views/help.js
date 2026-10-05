// Help: every feature, how to use it, and a search across all of it.
//
// The same view runs in the app and, from the unlock screen, in a dialog —
// there it has no screens to link to, so screen links become plain text.

import { h, mount } from "../lib/dom.js";
import { icon } from "../lib/icons.js";
import { modal } from "../ui/components.js";
import { SECTIONS, TOPICS } from "../help/topics.js";
import { buildIndex, search as searchTopics, termPattern } from "../help/search.js";
import { blocks, highlight, inline } from "../help/markup.js";

const INDEX = buildIndex(TOPICS);
const BY_ID = new Map(TOPICS.map((t) => [t.id, t]));
const SECTION_NAME = Object.fromEntries(SECTIONS);

/** Topics to start from, on the help's front page. */
const START_HERE = ["first-item", "backups", "claim", "values-explained"];

export function renderHelp(root, params = {}, ctx = {}) {
  let topicId = BY_ID.has(params.topic) ? params.topic : null;
  let query = params.q ?? "";
  let terms = [];

  const search = h("input", {
    type: "search",
    class: "search-input",
    placeholder: "Search help — backup, claim, spot price, forgot passphrase…",
    "aria-label": "Search help",
    "data-search": "",
    value: query,
  });
  const toc = h("nav", { class: "help-toc", "aria-label": "Help contents" });
  const content = h("div", { class: "help-content", "aria-live": "polite" });

  const links = {
    title: (id) => BY_ID.get(id)?.title ?? id,
    topic: (id, text) => h("button", { class: "link", onclick: () => open(id) }, text),
    screen: (view, text) =>
      ctx.navigate ? h("button", { class: "link", onclick: () => ctx.navigate(view) }, text) : h("strong", {}, text),
  };

  const scrollTop = () => {
    const scroller = root.closest(".main, .modal-body");
    if (scroller) scroller.scrollTop = 0;
  };

  function open(id, { keepQuery = false } = {}) {
    topicId = id;
    if (!keepQuery) { query = ""; search.value = ""; terms = []; }
    draw();
    scrollTop();
  }

  function drawToc() {
    mount(toc, SECTIONS.map(([section, name]) => h("div", { class: "help-toc-section" },
      h("h3", {}, name),
      h("ul", {}, TOPICS.filter((t) => t.section === section).map((t) => h("li", {},
        h("button", {
          class: t.id === topicId ? "help-toc-link active" : "help-toc-link",
          "aria-current": t.id === topicId ? "page" : null,
          onclick: () => open(t.id),
        }, t.title)
      )))
    )));
  }

  function home() {
    return [
      h("p", { class: "lede" }, "Everything Asset Manager can do, and how to do it. Search above, or start with a topic below."),
      h("div", { class: "help-start" }, START_HERE.map((id) => {
        const t = BY_ID.get(id);
        return h("button", { class: "help-start-card", onclick: () => open(id) },
          h("strong", {}, t.title), h("span", {}, t.summary));
      })),
      h("div", { class: "help-sections" }, SECTIONS.map(([section, name]) => h("section", { class: "help-section-card" },
        h("h2", {}, name),
        h("ul", {}, TOPICS.filter((t) => t.section === section).map((t) => h("li", {}, links.topic(t.id, t.title))))
      ))),
    ];
  }

  function results() {
    const found = searchTopics(INDEX, query);
    terms = found.terms;
    const pattern = termPattern(terms);
    if (!found.results.length) {
      return [
        h("h2", { class: "help-results-head" }, `Nothing found for “${query.trim()}”`),
        h("p", { class: "muted" }, "Try fewer or different words — or browse the contents."),
      ];
    }
    const list = h("ol", { class: "help-results" }, found.results.map((r, i) => {
      const title = h("strong", {}, r.topic.title);
      const snippet = h("span", { class: "help-snippet" }, r.snippet);
      highlight(title, pattern);
      highlight(snippet, pattern);
      return h("li", {}, h("button", {
        class: "help-result",
        "data-index": String(i),
        onclick: () => open(r.topic.id, { keepQuery: true }),
        onkeydown: (e) => {
          if (e.key === "ArrowDown") { e.preventDefault(); list.querySelector(`[data-index="${i + 1}"]`)?.focus(); }
          if (e.key === "ArrowUp") { e.preventDefault(); (list.querySelector(`[data-index="${i - 1}"]`) ?? search).focus(); }
        },
      }, h("span", { class: "help-result-section" }, SECTION_NAME[r.topic.section]), title, snippet));
    }));
    const n = found.results.length;
    return [
      h("h2", { class: "help-results-head" },
        found.partial ? `No topic has every word — the closest ${n === 1 ? "match" : `${n} matches`}` : `${n} topic${n === 1 ? "" : "s"} for “${query.trim()}”`),
      list,
    ];
  }

  function article() {
    const t = BY_ID.get(topicId);
    const at = TOPICS.indexOf(t);
    const prev = TOPICS[at - 1];
    const next = TOPICS[at + 1];
    const body = h("div", { class: "help-body" }, blocks(t.body, links));
    const el = h("article", { class: "help-article" },
      h("p", { class: "help-crumb" },
        h("button", { class: "link", onclick: () => open(null) }, "Help"), " › ", SECTION_NAME[t.section]),
      h("h1", {}, t.title),
      h("p", { class: "lede" }, t.summary),
      query.trim()
        ? h("p", { class: "help-back" }, h("button", { class: "link", onclick: () => { topicId = null; draw(); scrollTop(); } }, icon("back", { size: 14 }), ` Back to results for “${query.trim()}”`))
        : null,
      body,
      h("nav", { class: "help-pager", "aria-label": "Neighbouring topics" },
        prev ? h("button", { class: "help-pager-link", onclick: () => open(prev.id) }, h("span", {}, "Previous"), h("strong", {}, prev.title)) : h("span"),
        next ? h("button", { class: "help-pager-link next", onclick: () => open(next.id) }, h("span", {}, "Next"), h("strong", {}, next.title)) : h("span"))
    );
    if (query.trim()) {
      const marks = highlight(body, termPattern(terms));
      if (marks[0]) requestAnimationFrame(() => marks[0].scrollIntoView({ block: "center" }));
    }
    return el;
  }

  function draw() {
    drawToc();
    if (query.trim() && !topicId) mount(content, results());
    else if (topicId) mount(content, article());
    else mount(content, home());
  }

  search.addEventListener("input", () => {
    query = search.value;
    topicId = null;
    draw();
  });
  search.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && search.value) { e.stopPropagation(); search.value = ""; query = ""; topicId = null; draw(); }
    if (e.key === "ArrowDown") { e.preventDefault(); content.querySelector(".help-result")?.focus(); }
    if (e.key === "Enter") content.querySelector(".help-result")?.click();
  });

  mount(root,
    h("header", { class: "page-head" },
      h("div", {}, h("h1", {}, "Help"), h("p", { class: "page-sub" }, "How to do everything in Asset Manager"))),
    h("div", { class: "toolbar" }, h("div", { class: "search" }, icon("search", { size: 17 }), search)),
    h("div", { class: "help-layout" }, toc, content)
  );
  draw();
  if (params.focusSearch) search.focus();
}

/** Help in a dialog, for when the app itself is not open (the unlock screen). */
export function openHelpDialog(topic = null) {
  const body = h("div", { class: "help-dialog" });
  modal({ title: "Help", size: "full", body });
  renderHelp(body, { topic }, {});
}
