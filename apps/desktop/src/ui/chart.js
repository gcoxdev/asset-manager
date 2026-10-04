// Value-over-time chart.
//
// Semantics come from docs/chart-semantics.md; the rules visible here:
//
// - stretches where any holding was unpriced are DASHED, so a partial total
//   never looks complete, and the tooltip says how many were missing;
// - dates carrying a quantity change are MARKED, so a step can be attributed
//   to a purchase or sale rather than read as appreciation;
// - the y-axis starts at zero — an area chart that does not start at zero
//   exaggerates every wobble;
// - points sit at their DATES, not evenly by index, and the line STEPS: a
//   value holds until the next one is known (it is carried forward, never
//   interpolated), so a sloped line between two valuations months apart
//   would draw measurements nobody took.
//
// Marks follow the dataviz spec: 2px line, ~10% area wash, ≥8px markers with
// a surface ring, hairline recessive grid, one series so no legend box.

import { h, svg, clear } from "../lib/dom.js";
import { compactMoney, date as fmtDate, money } from "../lib/format.js";

/**
 * points: [{ date, value (number, major units — pixels only), display,
 *            unvalued, valued, event }]
 */
export function valueChart(points, { currency = "USD", height = 240, label = "Value" } = {}) {
  const root = h("div", { class: "chart", tabindex: "0", role: "img", "aria-label": `${label} chart` });
  const tooltip = h("div", { class: "chart-tip", hidden: true });
  root.append(tooltip);

  let width = 0;
  let geometry = null;
  let focusIndex = null;

  const draw = () => {
    width = Math.max(280, Math.floor(root.clientWidth));
    for (const node of [...root.children]) if (node !== tooltip) node.remove();
    geometry = render(points, width, height, currency);
    root.prepend(geometry.el);
    if (focusIndex !== null) show(focusIndex);
  };

  const show = (i) => {
    if (!geometry || i === null) return;
    focusIndex = i;
    const p = points[i];
    const x = geometry.x(i);
    const y = geometry.y(p.value);
    geometry.cross.setAttribute("x1", x);
    geometry.cross.setAttribute("x2", x);
    geometry.cross.removeAttribute("visibility");
    geometry.dot.setAttribute("cx", x);
    geometry.dot.setAttribute("cy", y);
    geometry.dot.removeAttribute("visibility");

    clear(tooltip);
    tooltip.append(
      h("div", { class: "chart-tip-date" }, fmtDate(p.date)),
      h("div", { class: "chart-tip-value" }, money(p.display)),
      p.unvalued > 0
        ? h("div", { class: "chart-tip-note" }, `${p.unvalued} holding${p.unvalued === 1 ? "" : "s"} unpriced`)
        : null,
      p.event ? h("div", { class: "chart-tip-note" }, "A holding changed — steps here are purchases or sales") : null
    );
    tooltip.hidden = false;
    const tipWidth = tooltip.offsetWidth;
    const left = Math.min(Math.max(x - tipWidth / 2, 0), width - tipWidth);
    tooltip.style.left = `${left}px`;
    tooltip.style.top = `${Math.max(0, y - tooltip.offsetHeight - 14)}px`;
  };

  const hide = () => {
    focusIndex = null;
    tooltip.hidden = true;
    geometry?.cross.setAttribute("visibility", "hidden");
    geometry?.dot.setAttribute("visibility", "hidden");
  };

  root.addEventListener("pointermove", (event) => {
    if (!geometry || points.length === 0) return;
    const rect = root.getBoundingClientRect();
    show(geometry.nearest(event.clientX - rect.left));
  });
  root.addEventListener("pointerleave", hide);
  root.addEventListener("blur", hide);
  root.addEventListener("keydown", (event) => {
    if (!points.length) return;
    if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
      event.preventDefault();
      const start = focusIndex ?? points.length - 1;
      show(Math.max(0, Math.min(points.length - 1, start + (event.key === "ArrowRight" ? 1 : -1))));
    }
    if (event.key === "Home") show(0);
    if (event.key === "End") show(points.length - 1);
  });

  // Removing the chart from the page also ends the observer, so navigating
  // back and forth does not accumulate them. (A chart can be built before it
  // is mounted, so only a chart that was on the page and left it counts.)
  let attached = false;
  const observer = new ResizeObserver(() => {
    if (!root.isConnected) {
      if (attached) observer.disconnect();
      return;
    }
    attached = true;
    if (Math.floor(root.clientWidth) !== width) draw();
  });
  observer.observe(root);
  requestAnimationFrame(draw);
  return root;
}

/** Round up to a clean axis maximum: 1, 2, 2.5, 5 × 10^n. */
function niceMax(value) {
  if (value <= 0) return 1;
  const exp = Math.floor(Math.log10(value));
  const base = 10 ** exp;
  for (const m of [1, 2, 2.5, 5, 10]) {
    if (value <= m * base) return m * base;
  }
  return 10 * base;
}

/** "2026-03-01" → whole days since the epoch, for spacing points by date. */
function dayOf(iso) {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso ?? "");
  return m ? Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])) / 86_400_000 : NaN;
}

function render(points, width, height, currency) {
  const pad = { top: 16, right: 16, bottom: 26, left: 56 };
  const plotW = width - pad.left - pad.right;
  const plotH = height - pad.top - pad.bottom;
  const max = niceMax(Math.max(...points.map((p) => p.value), 0) * 1.08);
  const n = points.length;

  // By date where every point has one; evenly otherwise.
  const days = points.map((p) => dayOf(p.date));
  const span = days[n - 1] - days[0];
  const byDate = n > 1 && days.every(Number.isFinite) && span > 0;
  const x = (i) =>
    pad.left + (n <= 1 ? plotW / 2 : byDate ? ((days[i] - days[0]) / span) * plotW : (i / (n - 1)) * plotW);
  const y = (v) => pad.top + plotH - (v / max) * plotH;

  const el = svg("svg", { width, height, viewBox: `0 0 ${width} ${height}`, class: "chart-svg" });

  // Grid: four hairlines with clean labels.
  for (let t = 0; t <= 4; t += 1) {
    const value = (max / 4) * t;
    const gy = y(value);
    el.append(
      svg("line", { class: t === 0 ? "chart-base" : "chart-grid", x1: pad.left, x2: width - pad.right, y1: gy, y2: gy }),
      svg("text", { class: "chart-axis", x: pad.left - 8, y: gy + 4, "text-anchor": "end" }, compactMoney(value, currency))
    );
  }

  // X labels: first, middle, last — a dense date axis is noise at this size.
  const labelAt = [0, Math.floor((n - 1) / 2), n - 1].filter((v, i, a) => a.indexOf(v) === i);
  for (const i of labelAt) {
    if (!points[i]) continue;
    const anchor = i === 0 ? "start" : i === n - 1 ? "end" : "middle";
    el.append(
      svg("text", { class: "chart-axis", x: x(i), y: height - 6, "text-anchor": anchor }, fmtDate(points[i].date))
    );
  }

  if (n >= 2) {
    // Each value holds until the next: across to the next date, then up or
    // down to the new value.
    const step = (i) => `H ${x(i)} V ${y(points[i].value)}`;

    // Area wash under the whole line.
    const area =
      `M ${x(0)} ${y(0)} V ${y(points[0].value)} ` +
      points.slice(1).map((_, k) => step(k + 1)).join(" ") +
      ` V ${y(0)} Z`;
    el.append(svg("path", { class: "chart-area", d: area }));

    // Line, split into runs of equal coverage so partial stretches dash.
    let run = [0];
    const flush = (partial) => {
      if (run.length < 2) return;
      const d = `M ${x(run[0])} ${y(points[run[0]].value)} ` + run.slice(1).map(step).join(" ");
      el.append(svg("path", { class: partial ? "chart-line partial" : "chart-line", d }));
    };
    for (let i = 1; i < n; i += 1) {
      const partial = points[i - 1].unvalued > 0;
      if ((points[i].unvalued > 0) !== partial) {
        run.push(i);
        flush(partial);
        run = [i];
      } else {
        run.push(i);
      }
    }
    flush(points[n - 1].unvalued > 0);

    // Quantity-change markers.
    points.forEach((p, i) => {
      if (p.event) el.append(svg("circle", { class: "chart-event", cx: x(i), cy: y(p.value), r: 4 }));
    });

    // End dot: the current value, labelled by the hero figure above.
    el.append(svg("circle", { class: "chart-end", cx: x(n - 1), cy: y(points[n - 1].value), r: 4.5 }));
  }

  const cross = svg("line", { class: "chart-cross", x1: 0, x2: 0, y1: pad.top, y2: pad.top + plotH, visibility: "hidden" });
  const dot = svg("circle", { class: "chart-hover", r: 5, cx: 0, cy: 0, visibility: "hidden" });
  el.append(cross, dot);

  const nearest = (px) => {
    let best = 0;
    for (let i = 1; i < n; i += 1) if (Math.abs(x(i) - px) < Math.abs(x(best) - px)) best = i;
    return best;
  };

  return { el, x, y, cross, dot, nearest };
}

/** A tiny trend line for a card — decorative, so hidden from assistive tech. */
export function sparkline(values, { width = 120, height = 32 } = {}) {
  const el = svg("svg", { class: "sparkline", width, height, viewBox: `0 0 ${width} ${height}`, "aria-hidden": "true" });
  if (values.length < 2) return el;
  const max = Math.max(...values);
  const min = Math.min(...values);
  const span = max - min || 1;
  const x = (i) => 2 + (i / (values.length - 1)) * (width - 4);
  const y = (v) => height - 3 - ((v - min) / span) * (height - 6);
  el.append(svg("path", { class: "sparkline-line", d: values.map((v, i) => `${i ? "L" : "M"} ${x(i)} ${y(v)}`).join(" ") }));
  return el;
}
