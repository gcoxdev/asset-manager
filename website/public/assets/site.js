// Progressive enhancements only: every article and download is static HTML.
const contents = document.querySelector(".guide-sidebar details");
if (contents) {
  const narrow = matchMedia("(max-width: 900px)");
  const adaptContents = () => { contents.open = !narrow.matches; };
  adaptContents();
  narrow.addEventListener("change", adaptContents);
}

const tabs = document.querySelector(".tour-tabs");
if (tabs) {
  const buttons = [...tabs.querySelectorAll("button")];
  tabs.hidden = false;
  tabs.setAttribute("role", "tablist");
  const select = index => buttons.forEach((button, i) => {
    const selected = index === i;
    button.setAttribute("aria-selected", String(selected));
    button.tabIndex = selected ? 0 : -1;
    document.getElementById(`shot-${button.dataset.shot}`).hidden = !selected;
  });
  buttons.forEach((button, index) => {
    const panel = document.getElementById(`shot-${button.dataset.shot}`);
    button.id = `tab-${button.dataset.shot}`;
    button.setAttribute("role", "tab");
    button.setAttribute("aria-controls", panel.id);
    panel.setAttribute("role", "tabpanel");
    panel.setAttribute("aria-labelledby", button.id);
    button.addEventListener("click", () => select(index));
    button.addEventListener("keydown", event => {
      const keys = { ArrowRight: (index + 1) % buttons.length, ArrowLeft: (index + buttons.length - 1) % buttons.length, Home: 0, End: buttons.length - 1 };
      if (!(event.key in keys)) return;
      event.preventDefault();
      select(keys[event.key]);
      buttons[keys[event.key]].focus();
    });
  });
  select(0);
}

const form = document.querySelector(".help-search");
if (form) {
  const status = form.querySelector(".search-status");
  try {
    const [{ buildIndex, search, termPattern }, response] = await Promise.all([
      import("./help-search.js"),
      fetch(new URL("./help-index.json", import.meta.url)),
    ]);
    if (!response.ok) throw new Error("Search index unavailable");
    const index = buildIndex(await response.json());
    const input = form.querySelector("input");
    const resultBox = document.querySelector(".search-results");
    const list = resultBox.querySelector("ol");
    const defaults = document.querySelector("[data-guide-default]");
    input.disabled = false;
    form.querySelector("button").disabled = false;
    // Only text nodes are inserted; queries are never treated as HTML.
    const markedText = (parent, text, pattern) => {
      if (!pattern) { parent.textContent = text; return; }
      let last = 0;
      for (const match of text.matchAll(pattern)) {
        parent.append(document.createTextNode(text.slice(last, match.index)));
        const mark = document.createElement("mark");
        mark.textContent = match[0]; parent.append(mark);
        last = match.index + match[0].length;
      }
      parent.append(document.createTextNode(text.slice(last)));
    };
    const draw = () => {
      const query = input.value.trim();
      const url = new URL(location.href);
      if (query) url.searchParams.set("q", query); else url.searchParams.delete("q");
      history.replaceState(null, "", url);
      defaults.hidden = Boolean(query);
      resultBox.hidden = !query;
      list.replaceChildren();
      if (!query) { status.textContent = `Search ${index.length} topics, or browse below.`; return; }
      const found = search(index, query);
      const n = found.results.length;
      const message = n ? `${n} ${found.partial ? "closest matches" : n === 1 ? "topic" : "topics"} for “${query}”` : `No topics found for “${query}”. Try fewer words, or clear the search to browse.`;
      document.getElementById("results-heading").textContent = n ? "Here’s what we found." : "Let’s try another search.";
      status.textContent = message;
      const pattern = termPattern(found.terms);
      for (const { topic, snippet } of found.results) {
        const li = document.createElement("li");
        const a = document.createElement("a");
        a.href = `/guide/${topic.id}/`;
        const title = document.createElement("strong"), p = document.createElement("p");
        markedText(title, topic.title, pattern);
        markedText(p, snippet, pattern);
        a.append(title, p); li.append(a); list.append(li);
      }
    };
    input.value = new URL(location.href).searchParams.get("q") ?? "";
    draw();
    input.addEventListener("input", draw);
    input.addEventListener("keydown", event => {
      if (event.key === "Escape") { input.value = ""; draw(); }
      if (event.key === "ArrowDown") { event.preventDefault(); list.querySelector("a")?.focus(); }
    });
    form.addEventListener("submit", event => {
      event.preventDefault(); draw();
      list.querySelector("a")?.focus();
    });
  } catch {
    status.textContent = "Search couldn’t load. You can still browse every guide below.";
  }
}
