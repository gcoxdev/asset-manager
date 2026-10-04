// Stroke icons, 24×24, drawn for this app.
//
// Inline SVG rather than an icon font or image files: nothing extra to load,
// they inherit the text colour, and the CSP needs no relaxation.

import { svg } from "./dom.js";

const PATHS = {
  overview: ["M3 13h8V3H3z", "M13 21h8V11h-8z", "M13 3h8v5h-8z", "M3 21h8v-5H3z"],
  holdings: ["M12 3 2 8l10 5 10-5-10-5z", "M2 13l10 5 10-5", "M2 18l10 5 10-5"],
  markets: ["M3 17l6-6 4 4 8-8", "M15 7h6v6"],
  reports: ["M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z", "M14 3v6h6", "M8 13h8", "M8 17h5"],
  settings: [
    "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
    "M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z",
  ],
  lock: ["M5 11h14v10H5z", "M8 11V7a4 4 0 0 1 8 0v4"],
  unlock: ["M5 11h14v10H5z", "M8 11V7a4 4 0 0 1 7.5-2"],
  plus: ["M12 5v14", "M5 12h14"],
  target: ["M12 19a7 7 0 1 0 0-14 7 7 0 0 0 0 14z", "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z", "M12 2v3", "M12 19v3", "M2 12h3", "M19 12h3"],
  search: ["M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z", "M21 21l-5-5"],
  x: ["M6 6l12 12", "M18 6 6 18"],
  back: ["M15 18l-6-6 6-6"],
  forward: ["M9 18l6-6-6-6"],
  down: ["M6 9l6 6 6-6"],
  more: ["M5 12h.01", "M12 12h.01", "M19 12h.01"],
  edit: ["M4 20h4L18.5 9.5a2.1 2.1 0 0 0-4-4L4 16v4z", "M13.5 6.5l4 4"],
  trash: ["M4 7h16", "M10 11v6", "M14 11v6", "M6 7l1 13h10l1-13", "M9 7V4h6v3"],
  image: ["M4 5h16v14H4z", "M4 16l5-5 4 4 3-3 4 4", "M15 9h.01"],
  camera: ["M4 8h3l2-3h6l2 3h3v11H4z", "M12 17a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z"],
  upload: ["M12 16V4", "M7 9l5-5 5 5", "M4 20h16"],
  download: ["M12 4v12", "M7 11l5 5 5-5", "M4 20h16"],
  refresh: ["M20 12a8 8 0 1 1-2.3-5.7", "M20 4v5h-5"],
  check: ["M5 12.5l4.5 4.5L19 7.5"],
  warn: ["M12 3 2 20h20L12 3z", "M12 10v4", "M12 17h.01"],
  info: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M12 11v5", "M12 8h.01"],
  clock: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M12 7v5l3 2"],
  pin: ["M12 21s-7-6.5-7-12a7 7 0 0 1 14 0c0 5.5-7 12-7 12z", "M12 11.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z"],
  star: ["M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z"],
  eye: ["M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z", "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z"],
  eyeOff: ["M3 3l18 18", "M10.6 5.1A10 10 0 0 1 12 5c6.5 0 10 7 10 7a17 17 0 0 1-3 3.9", "M6.6 6.6A17 17 0 0 0 2 12s3.5 7 10 7a9.6 9.6 0 0 0 5.4-1.6", "M9.9 9.9a3 3 0 0 0 4.2 4.2"],
  copy: ["M9 9h11v11H9z", "M5 15H4V4h11v1"],
  printer: ["M6 9V3h12v6", "M6 18H4v-7h16v7h-2", "M6 14h12v7H6z"],
  shield: ["M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"],
  key: ["M15 9a4 4 0 1 1-3 6.7L5 22H2v-3l7.3-7A4 4 0 0 1 15 9z", "M16.5 7.5h.01"],
  archive: ["M3 4h18v4H3z", "M5 8v12h14V8", "M10 12h4"],
  list: ["M8 6h13", "M8 12h13", "M8 18h13", "M3 6h.01", "M3 12h.01", "M3 18h.01"],
  grid: ["M4 4h7v7H4z", "M13 4h7v7h-7z", "M4 13h7v7H4z", "M13 13h7v7h-7z"],
  up: ["M7 17 17 7", "M8 7h9v9"],
  downRight: ["M7 7l10 10", "M17 8v9H8"],
  scan: ["M4 8V5a1 1 0 0 1 1-1h3", "M16 4h3a1 1 0 0 1 1 1v3", "M20 16v3a1 1 0 0 1-1 1h-3", "M8 20H5a1 1 0 0 1-1-1v-3", "M8 8v8", "M11 8v8", "M14 8v8", "M17 8v8"],
  swap: ["M7 4v16", "M3 16l4 4 4-4", "M17 20V4", "M21 8l-4-4-4 4"],
  tag: ["M3 12V3h9l9 9-9 9z", "M7.5 7.5h.01"],
  // Category and type glyphs.
  metal: ["M3 17l3-7h12l3 7z", "M7 10l2-4h6l2 4"],
  coin: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M12 7v10", "M15 9.5c0-1.4-1.3-2.2-3-2.2s-3 .8-3 2.2 1.3 2 3 2.3 3 .9 3 2.3-1.3 2.2-3 2.2-3-.8-3-2.2"],
  crypto: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M9 7.5h4.5a2.2 2.2 0 0 1 0 4.5H9z", "M9 12h5a2.2 2.2 0 0 1 0 4.5H9z", "M10.5 6v1.5", "M10.5 16.5V18", "M9 7.5v9"],
  comic: ["M4 4h11l5 5v11H4z", "M15 4v5h5", "M8 13l2-2 2 3 2-2 2 3"],
  card: ["M6 3h12v18H6z", "M9 7h6", "M9 17h6", "M12 10.5l1.5 1.5-1.5 1.5-1.5-1.5z"],
  gem: ["M6 3h12l4 6-10 12L2 9z", "M2 9h20", "M12 21 8 9l4-6 4 6z"],
  watch: ["M12 18a6 6 0 1 0 0-12 6 6 0 0 0 0 12z", "M12 9v3l2 1", "M9 6l1-4h4l1 4", "M9 18l1 4h4l1-4"],
  art: ["M12 3a9 9 0 0 0 0 18c1.5 0 2-1 2-2s-1-1.5-1-2.5 1-1.5 2-1.5h2a4 4 0 0 0 4-4c0-4.4-4-8-9-8z", "M7.5 12h.01", "M9.5 7.5h.01", "M14.5 7.5h.01"],
  wine: ["M8 3h8l-.5 6a3.5 3.5 0 0 1-7 0z", "M12 12.5V20", "M8 21h8"],
  music: ["M9 18V5l12-2v13", "M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6z", "M18 19a3 3 0 1 0 0-6 3 3 0 0 0 0 6z"],
  box: ["M21 8 12 3 3 8v8l9 5 9-5z", "M3 8l9 5 9-5", "M12 13v8"],
  cash: ["M3 6h18v12H3z", "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z", "M6 9v.01", "M18 15v.01"],
  game: ["M6 11h4", "M8 9v4", "M15 12h.01", "M18 10h.01", "M17.3 5H6.7a4 4 0 0 0-4 3.6l-.7 6.2A3 3 0 0 0 7 17.6L9 15h6l2 2.6a3 3 0 0 0 5-2.8l-.7-6.2a4 4 0 0 0-4-3.6z"],
  vinyl: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M12 14a2 2 0 1 0 0-4 2 2 0 0 0 0 4z", "M12 7a5 5 0 0 0-5 5"],
  signed: ["M3 17c3-1 4-8 6-8s0 6 2 6 2-3 3.5-3 1 2 2.5 2 2.5-1 4-2", "M3 21h18"],
  item: ["M20 7 12 3 4 7v10l8 4 8-4z", "M12 12l8-5", "M12 12v9", "M12 12 4 7"],
};

/** An icon element. `label` makes it announced; otherwise it is decorative. */
export function icon(name, { size = 18, label } = {}) {
  const paths = PATHS[name] ?? PATHS.item;
  return svg(
    "svg",
    {
      class: "icon",
      width: size,
      height: size,
      viewBox: "0 0 24 24",
      fill: "none",
      stroke: "currentColor",
      "stroke-width": 1.8,
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
      role: label ? "img" : null,
      "aria-label": label ?? null,
      "aria-hidden": label ? null : "true",
    },
    paths.map((d) => svg("path", { d }))
  );
}

/** The brand mark: padlock over ledger bars, matching the app icon. */
export function brandMark(size = 32) {
  return svg(
    "svg",
    { class: "brand-mark", width: size, height: size, viewBox: "0 0 512 512", "aria-hidden": "true" },
    svg("rect", { x: 16, y: 16, width: 480, height: 480, rx: 92, fill: "#223643" }),
    svg("path", {
      d: "M 176 232 V 176 a 80 80 0 0 1 160 0 v 56",
      fill: "none",
      stroke: "#e8c14f",
      "stroke-width": 40,
      "stroke-linecap": "round",
    }),
    svg("rect", { x: 136, y: 232, width: 240, height: 180, rx: 28, fill: "#e8c14f" }),
    svg("rect", { x: 176, y: 286, width: 160, height: 26, rx: 13, fill: "#18262f" }),
    svg("rect", { x: 176, y: 336, width: 104, height: 26, rx: 13, fill: "#18262f" })
  );
}

/** Which glyph stands for a type. */
export function typeIcon(typeId, category) {
  const byType = {
    gold_bullion: "metal", silver_bullion: "metal", platinum_bullion: "metal",
    palladium_bullion: "metal", junk_silver: "coin", sovereign_coin: "coin",
    crypto: "crypto", comic: "comic", trading_card: "card", tcg_card: "card",
    numismatic_coin: "coin", memorabilia: "signed", sealed_product: "box",
    video_game: "game", vinyl: "vinyl", art: "art", watch: "watch", jewelry: "gem",
    instrument: "music", wine: "wine", cash: "cash", generic: "item",
    firearm: "target", ammunition: "target", firearm_accessory: "target",
  };
  if (byType[typeId]) return byType[typeId];
  return { metals: "metal", crypto: "crypto", collectibles: "card", valuables: "gem", firearms: "target", cash: "cash" }[category] ?? "item";
}
