import { escape as e, blocks } from "./markup.mjs";

const repo = "https://github.com/gcoxdev/asset-manager";
const arrow = '<span aria-hidden="true">↗</span>';
const paths = {
  box: '<path d="m12 3 9 5-9 5-9-5 9-5ZM3 8v9l9 5 9-5V8M12 13v9M7.5 5.5l9 5"/>',
  chart: '<path d="M3 3v18h18M7 15l4-4 4 2 6-8"/>',
  paper: '<path d="M14 2H5v20h14V7l-5-5ZM14 2v6h5M8 12h8M8 16h8"/>',
  lock: '<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3M12 14v3"/>',
  backup: '<path d="M3 11a9 9 0 1 1 2 7M3 4v7h7M12 7v5l3 2"/>',
  search: '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
  windows: '<path d="m3 5 8-1v7H3V5ZM14 3.5 21 2v9h-7V3.5ZM3 14h8v7l-8-1v-6ZM14 14h7v9l-7-1.5V14Z"/>',
  linux: '<path d="M9 10V6a3 3 0 0 1 6 0v4l4 9H5l4-9ZM8 19l-4 2M16 19l4 2M10 8h.01M14 8h.01M11 11h2"/>',
  laptop: '<path d="M5 4h14v13H5V4ZM2 17h20l-2 3H4l-2-3Z"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
};
const icon = name => `<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] ?? paths.box}</svg>`;
const brand = '<a class="brand" href="/" aria-label="Asset Manager home"><img src="/assets/icon.svg" width="36" height="36" alt=""><span>Asset Manager<span class="brand-caption">A little order. A lot of peace of mind.</span></span></a>';

export function layout({ title, description, body, active }) {
  return `<!doctype html>
<html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="description" content="${e(description)}"><meta name="theme-color" content="#f6f5ef">
<meta property="og:type" content="website"><meta property="og:title" content="${e(title)}"><meta property="og:description" content="${e(description)}">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'">
<title>${e(title)}</title><link rel="icon" href="/assets/icon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/assets/site.css"><script type="module" src="/assets/site.js"></script>
</head><body>
<a class="skip-link" href="#main">Skip to content</a>
<header class="site-header"><div class="container header-inner">${brand}
<nav aria-label="Main navigation"><a href="/#features"${active === "product" ? ' aria-current="page"' : ""}>The app</a><a href="/guide/"${active === "guide" ? ' aria-current="page"' : ""}>User guide</a><a href="/privacy/"${active === "privacy" ? ' aria-current="page"' : ""}>Privacy</a><a class="nav-download" href="/#download">Get the app <span aria-hidden="true">↓</span></a></nav>
</div></header>
<main id="main">${body}</main>
<footer class="site-footer"><div class="container footer-top">${brand}<p>Your things. Your records. Your control.</p><a href="${repo}" target="_blank" rel="noopener noreferrer">View on GitHub ${arrow}</a></div><div class="container footer-bottom"><span>Free & open source · AGPL-3.0</span><div><a href="/guide/">User guide</a><a href="/privacy/">Privacy & data</a><a href="${repo}/issues" target="_blank" rel="noopener noreferrer">Report an issue ${arrow}</a></div></div></footer>
</body></html>`;
}

const shot = (name, alt, eager = false) => `<img src="/assets/screenshots/${name}.png" width="2720" height="1700" alt="${e(alt)}" ${eager ? 'fetchpriority="high"' : 'loading="lazy"'} decoding="async">`;
const articleCard = (id, title, text, number) => `<a class="guide-card" href="/guide/${id}/"><span class="step-number">${number}</span><h3>${title}</h3><p>${text}</p><span class="text-link">Read the guide <span aria-hidden="true">→</span></span></a>`;

export function home(release) {
  const base = `${release.repository}/releases/download/v${release.version}/`;
  const download = key => e(base + release.assets[key]);
  return `
<section class="hero container">
  <div class="hero-copy"><p class="eyebrow"><span class="status-dot"></span> For the things worth keeping</p>
  <h1>A place for<br>everything<br>you <em>value.</em></h1>
  <p class="hero-lede">The collection you built. The watch you inherited. The paperwork you hope you’ll never need.</p>
  <p class="hero-description">Keep what you own, what it’s worth, and its story together in one private desktop app.</p>
  <div class="actions"><a class="button" href="#download">Download Asset Manager <span aria-hidden="true">↓</span></a><a class="quiet-link" href="#tour">Take a look <span aria-hidden="true">→</span></a></div>
  <p class="hero-footnote">Free & open source. No account required.</p></div>
  <div class="hero-visual"><div class="visual-caption"><span class="tiny-square"></span> YOUR COLLECTION, AT A GLANCE <span>01 / 03</span></div><a class="app-window" href="/assets/screenshots/overview.png" aria-label="Open the full overview screenshot"><div class="window-bar"><span></span><span></span><span></span><p>Asset Manager / Overview</p></div>${shot("overview", "Asset Manager overview showing a fictional collection, its value, and a value history chart.", true)}</a><div class="visual-note">${icon("lock")} <span>On your computer.<br><strong>Under your control.</strong></span></div><p class="demo-label">Real app. Fictional collection. All screenshots use dummy data.</p></div>
</section>
<div class="principles"><div class="container"><span>${icon("laptop")} Local by default</span><span>${icon("lock")} Encrypted at rest</span><span>${icon("backup")} Backups you control</span><a href="${repo}" target="_blank" rel="noopener noreferrer">Open source, inside and out ${arrow}</a></div></div>

<section class="section container" id="features"><div class="section-heading"><div><p class="eyebrow">01 / A clearer picture</p><h2>More than a list<br>of things.</h2></div><p>From a few meaningful possessions to a growing collection, keep the details that a spreadsheet tends to leave behind.</p></div>
<div class="feature-grid">
<article>${icon("box")}<h3>Give every item a story.</h3><p>Photos, receipts, serial numbers, condition, purchase details and storage locations. Everything that belongs with an item, in one place.</p><a href="/guide/kinds/">Explore what you can catalog <span aria-hidden="true">→</span></a></article>
<article>${icon("chart")}<h3>Know what it’s worth.</h3><p>Record your own valuations or enable metal and crypto prices. See dates and sources, track changes, and keep unknown values clearly marked.</p><a href="/guide/values-explained/">Understand the numbers <span aria-hidden="true">→</span></a></article>
<article>${icon("paper")}<h3>Keep the evidence close.</h3><p>Save appraisals and receipts with your items. Prepare an insurance inventory or a claim with photos and the evidence behind each value.</p><a href="/guide/insurance-report/">See how reports work <span aria-hidden="true">→</span></a></article>
</div><div class="collection-strip"><span>MAKE ROOM FOR</span><p>Coins & bullion <span>·</span> Watches & jewelry <span>·</span> Cards & comics <span>·</span> Crypto <span>·</span> Art <span>·</span> Everyday things</p></div></section>

<section class="tour-section section" id="tour"><div class="container"><div class="section-heading"><div><p class="eyebrow">02 / A look inside</p><h2>A little less searching.<br>A lot more knowing.</h2></div><p>See the big picture, find a particular item, or close the vault when you’re done. Familiar tools, with the details that matter.</p></div>
<div class="tour-tabs" aria-label="Screenshot views" hidden><button type="button" data-shot="overview">${icon("chart")} The big picture</button><button type="button" data-shot="holdings">${icon("box")} Your holdings</button><button type="button" data-shot="unlock">${icon("lock")} Your private vault</button></div>
<div class="tour-panels">
<figure id="shot-overview"><a href="/assets/screenshots/overview.png">${shot("overview", "Overview of a fictional collection with total value, known purchase costs and a historical value chart.")}</a><figcaption><strong>Your collection, in context.</strong> View allocation, changes over time, and the items that need attention. <span>Dummy data · Select the image to view full size.</span></figcaption></figure>
<figure id="shot-holdings"><a href="/assets/screenshots/holdings.png">${shot("holdings", "Holdings table from the dummy catalog, including metals, watches, crypto and collectibles, with costs and values.")}</a><figcaption><strong>Find the thing you’re looking for.</strong> Search, filter, sort and update your catalog, one item or many at a time. <span>Dummy data · Select the image to view full size.</span></figcaption></figure>
<figure id="shot-unlock"><a href="/assets/screenshots/unlock.png">${shot("unlock", "Asset Manager unlock screen for a fictional vault, offering passphrase or recovery key access.")}</a><figcaption><strong>Open only when you need it.</strong> Unlock with your passphrase or recovery key, and lock the vault when you step away. <span>Demo vault · Select the image to view full size.</span></figcaption></figure>
</div></div></section>

<section class="privacy-band"><div class="container privacy-grid"><div><p class="eyebrow">03 / Private by design</p><h2>A catalog of your life<br>should stay <em>yours.</em></h2><p>No account to create. No catalog to upload. Your vault and its attachments are encrypted on your computer.</p><a class="button button-light" href="/privacy/">How your data is protected <span aria-hidden="true">→</span></a></div><div class="privacy-points"><article><span>01</span><div><h3>Work offline.</h3><p>Catalog, value, search and back up your things without a connection. Price feeds and balance lookups are optional.</p></div></article><article><span>02</span><div><h3>Take your records with you.</h3><p>Back up the encrypted vault, restore it on another computer, or use a portable Linux or Windows build.</p></div></article><article><span>03</span><div><h3>Keep your recovery key safe.</h3><p>You hold the keys. There’s no password reset if you lose both your passphrase and recovery key.</p></div></article></div></div></section>

<section class="section container"><div class="section-heading"><div><p class="eyebrow">04 / Start with one thing</p><h2>A small start.<br>A useful habit.</h2></div><a class="text-link" href="/guide/">Browse the complete user guide <span aria-hidden="true">→</span></a></div><div class="guide-cards">${articleCard("create-vault", "Make a private space.", "Create your vault, choose a passphrase, and put your recovery key somewhere safe.", "01")}${articleCard("first-item", "Add something you value.", "Start with one item. Add its photo, what you paid, and whatever else you know.", "02")}${articleCard("backups", "Make a second copy.", "Back up your vault and verify it. Give your records a home beyond this computer.", "03")}</div></section>

<section class="download-section section" id="download"><div class="container"><div class="download-heading"><p class="eyebrow">Your collection starts here</p><h2>Make yourself at home.</h2><p>Download Asset Manager for your computer. Your catalog stays with you.</p><a class="release-badge" href="${repo}/releases/tag/v${e(release.version)}" target="_blank" rel="noopener noreferrer">v${e(release.version)} · ${release.prerelease ? "Preview release" : "Release"} ${arrow}</a></div>
<div class="download-grid"><article>${icon("windows")}<h3>Windows</h3><p>For Windows PCs · x64</p><a class="button" href="${download("windows")}" target="_blank" rel="noopener noreferrer">Download .msi <span aria-hidden="true">↓</span></a><a class="secondary-download" href="${download("windowsPortable")}" target="_blank" rel="noopener noreferrer">Portable .zip <span aria-hidden="true">↓</span></a><small>Unsigned build. Windows may show a security warning.</small></article>
<article>${icon("linux")}<h3>Linux</h3><p>For Linux desktops · x86-64</p><a class="button" href="${download("linux")}" target="_blank" rel="noopener noreferrer">Download AppImage <span aria-hidden="true">↓</span></a><div class="download-alternatives"><a href="${download("linuxDeb")}" target="_blank" rel="noopener noreferrer">Debian / Ubuntu .deb</a><a href="${download("linuxPortable")}" target="_blank" rel="noopener noreferrer">Portable .tar.gz</a></div><small>For AppImage, allow the file to run as a program.</small></article>
<article>${icon("laptop")}<h3>macOS</h3><p>Apple silicon & Intel · Universal</p><a class="button button-outline" href="${download("macos")}" target="_blank" rel="noopener noreferrer">Download .dmg <span aria-hidden="true">↓</span></a><a class="secondary-download" href="${repo}/releases/tag/v${e(release.version)}" target="_blank" rel="noopener noreferrer">Read release notes ${arrow}</a><small>Experimental. Built automatically; not yet manually tested.</small></article></div>
<div class="download-note"><span>Free & open source · No subscription</span><div><a href="${repo}/releases" target="_blank" rel="noopener noreferrer">All releases ${arrow}</a><a href="${download("checksums")}" target="_blank" rel="noopener noreferrer">Checksums</a><a href="/guide/portable/">About portable mode</a></div></div>
</div></section>

<section class="section container faq-section"><div><p class="eyebrow">A few good questions</p><h2>Before you<br>get started.</h2></div><div class="faq-list">
<details><summary>Is this a cloud service?</summary><p>No. Asset Manager is a desktop app. Your catalog lives in an encrypted vault on your computer. There is no account, cloud catalog or automatic device sync. You can transfer an encrypted backup between computers.</p></details>
<details><summary>Can I use it without an internet connection?</summary><p>Yes. Core cataloging, manual valuations, reports and backups work offline. Optional market prices and watch-only Bitcoin balance lookups need a connection. <a href="/guide/network/">See exactly what those features send.</a></p></details>
<details><summary>What if I already use a spreadsheet?</summary><p>Import a CSV and map your columns to the app’s fields. You can also export existing items, edit supported fields in a spreadsheet and import them back. <a href="/guide/spreadsheet-import/">Read the import guide.</a></p></details>
<details><summary>Does it automatically value everything?</summary><p>No. Metals and crypto can follow supported market feeds. For collectibles and other items, record your own value with its date and supporting evidence. Items without a value stay unknown rather than being treated as zero.</p></details>
<details><summary>What happens if I forget my passphrase?</summary><p>Your saved recovery key can unlock the vault. If you lose both, there is no reset service or master key. <a href="/guide/recovery-key/">Save your recovery key when you create the vault.</a></p></details>
</div></section>`;
}

function searchForm() {
  return `<form class="help-search" action="/guide/" role="search"><label for="help-query">Search the user guide</label><div>${icon("search")}<input id="help-query" name="q" type="search" placeholder="Try “backup”, “photos” or “forgot password”…" autocomplete="off" disabled><button type="submit" disabled>Search <span aria-hidden="true">→</span></button></div><p class="search-status" role="status">Preparing search. You can browse all topics below.</p></form><noscript><p class="callout">Search needs JavaScript. Every guide is available through the topic links below.</p></noscript>`;
}
function sidebar(help, active) {
  return `<aside class="guide-sidebar"><details open><summary>Browse all topics</summary><nav aria-label="Guide topics">${help.sections.map(([id, name]) => `<div><h2>${e(name)}</h2><ul>${help.topics.filter(t => t.section === id).map(t => `<li><a href="/guide/${t.id}/"${active === t.id ? ' aria-current="page"' : ""}>${e(t.title)}</a></li>`).join("")}</ul></div>`).join("")}</nav></details></aside>`;
}
const resultBox = '<section class="search-results" aria-label="Search results" hidden><h2 id="results-heading">Search results</h2><ol></ol></section>';
export function guideHome(help) {
  return `<section class="guide-hero container"><p class="eyebrow">The Asset Manager handbook</p><h1>A little guidance.<br>A lot you can do.</h1><p>The same complete help you’ll find in the app. From your first item to your next backup, find the steps here.</p>${searchForm()}</section><div class="container guide-layout">${sidebar(help)}<div class="guide-main">${resultBox}<div data-guide-default><div class="section-heading compact"><h2>Start here.</h2><span class="muted">${help.topics.length} topics · ${help.sections.length} sections</span></div><div class="guide-cards compact-cards">${articleCard("first-item", "Your first item", "A quick walk through adding something to your catalog.", "01")}${articleCard("backups", "Your first backup", "Keep a second copy of the whole encrypted vault.", "02")}</div><div class="topic-directory">${help.sections.map(([id, name], i) => `<section><span class="eyebrow">${String(i + 1).padStart(2, "0")}</span><h2>${e(name)}</h2><ul>${help.topics.filter(t => t.section === id).map(t => `<li><a href="/guide/${t.id}/">${e(t.title)} <span aria-hidden="true">→</span></a><p>${e(t.summary)}</p></li>`).join("")}</ul></section>`).join("")}</div></div></div></div>`;
}

export function guideArticle(topic, help) {
  const topics = new Map(help.topics.map(t => [t.id, t]));
  const index = help.topics.indexOf(topic);
  const section = help.sections.find(([id]) => id === topic.section)?.[1];
  const previous = help.topics[index - 1], next = help.topics[index + 1];
  return `<div class="container guide-top"><a href="/guide/">← All guides</a><a href="/guide/#help-query">Search the guide ${icon("search")}</a></div><div class="container guide-layout article-layout">${sidebar(help, topic.id)}<article class="guide-article"><p class="eyebrow">${e(section)}</p><h1>${e(topic.title)}</h1><p class="article-summary">${e(topic.summary)}</p><div class="article-body">${blocks(topic.body, topics)}</div><aside class="article-note">This guide mirrors the help inside Asset Manager. Screen names refer to controls in the desktop app.</aside><nav class="article-pager" aria-label="Adjacent guides">${previous ? `<a href="/guide/${previous.id}/"><span>← Previous</span><strong>${e(previous.title)}</strong></a>` : "<span></span>"}${next ? `<a href="/guide/${next.id}/"><span>Next →</span><strong>${e(next.title)}</strong></a>` : ""}</nav></article></div>`;
}

export function privacy() {
  return `<section class="container privacy-intro"><p class="eyebrow">Privacy & your data</p><h1>Ownership includes<br>your <em>information.</em></h1><p class="hero-lede">A list of your valuables is valuable in its own right. Here’s where your data lives, what protects it, and what remains your responsibility.</p></section>
<div class="container privacy-content"><nav class="privacy-index" aria-label="On this page"><a href="#your-vault">Your vault</a><a href="#encryption">Encryption</a><a href="#connections">Optional connections</a><a href="#recovery">Backups & recovery</a><a href="#limits">The limits</a><a href="#this-website">This website</a></nav><div class="article-body">
<section id="your-vault"><h2>One vault. On your computer.</h2><p>The app keeps your catalog in a local encrypted vault, including its photos and documents. There is no Asset Manager account, hosted catalog, advertising or in-app tracking.</p><p>Installed builds use your per-user application directory. Portable Linux and Windows builds keep their data in <code>AssetManagerData</code> beside the app. There is no automatic cloud sync.</p><a href="/guide/portable/">Learn about portable mode →</a></section>
<section id="encryption"><h2>Encryption is part of the app.</h2><p>SQLCipher encrypts the catalog database. Photos, documents and thumbnails use authenticated encryption. Your passphrase unlocks a randomly generated data key; your recovery key provides a separate way to unlock that same vault.</p><dl><dt>Argon2id</dt><dd>Derives a key from your passphrase and makes repeated guesses expensive.</dd><dt>XChaCha20-Poly1305</dt><dd>Encrypts attachments and checks for tampering.</dd><dt>HKDF-SHA256</dt><dd>Derives separate keys for the database, objects and thumbnails.</dd></dl><p>The implementation is open source. That makes it inspectable; it is not a claim of an independent security audit.</p><a href="${repo}/blob/main/docs/threat-model.md" target="_blank" rel="noopener noreferrer">Read the project’s threat model ${arrow}</a></section>
<section id="connections"><h2>Network features are your choice.</h2><p>Manual cataloging and valuations work offline. Optional price feeds contact external providers for metal prices, crypto prices or exchange rates. Watch-only Bitcoin balance lookups contact a balance provider when enabled.</p><p>Those providers can see your IP address and the identifiers you request. This can disclose an interest in an asset or associate an address with you. The app does not send the whole catalog to those providers.</p><a href="/guide/network/">See what each network feature sends →</a></section>
<section id="recovery"><h2>Keep a backup. Keep the key.</h2><p>An encrypted backup protects against losing a computer or damaging a vault. Keep a copy on another device, use the backup verification feature, and test recovery before you need it.</p><p>The recovery key is a credential, not a backup of your catalog. Store it safely, separately from the computer. Anyone who has it can unlock a compatible copy of the vault.</p><aside class="callout warning"><strong>There is no password reset.</strong><p>If you lose both your passphrase and recovery key, nobody can recover your catalog, including the author.</p></aside><a href="/guide/restore/">How to restore a backup →</a></section>
<section id="limits"><h2>Know what encryption cannot do.</h2><p>A compromised computer can expose data while the vault is open. A weak passphrase, unencrypted swap, printed reports, screenshots and deliberately exported files need their own care.</p><p>CSV exports, saved attachment copies and reports can contain private information in plaintext. Choose where you save them and who you share them with. Full-disk encryption provides an additional layer of protection.</p><a href="/guide/security/">Read the app’s security guide →</a></section>
<section id="this-website"><h2>A simple website, too.</h2><p>This site does not include analytics, advertising, external fonts or tracking cookies. Help search runs in your browser and does not send your query to a search service. Screenshots show a fictional demo catalog, not anyone’s private vault.</p><p>Your browser requests these pages from the website’s host, which may keep ordinary server access logs. Download and source-code links take you to GitHub, where GitHub’s policies apply.</p></section>
</div></div>`;
}
export function notFound() {
  return '<section class="container not-found"><p class="eyebrow">404 / A small detour</p><h1>This page isn’t<br>in the catalog.</h1><p>The address may have changed. The guide is a good place to start.</p><div class="actions"><a class="button" href="/guide/">Open the user guide →</a><a href="/">Back to home</a></div></section>';
}
