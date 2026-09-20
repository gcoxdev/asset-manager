# Chart semantics

What a point on a portfolio chart means, and what it must never imply.

Written before any chart shipped, because a chart is a claim about history and
an unconsidered one lies confidently. Where this document and the code
disagree, the code is the bug.

---

## 1. What a point means

A point at date *D* is:

> the sum, over every asset held on *D*, of **the most recent valuation at or
> before *D***, multiplied by nothing — the valuation already covers the whole
> holding.

Three things follow, and each rules out a tempting shortcut:

**Holdings come from the event log, not today's row.** Quantity on *D* is
replayed from `asset_events` up to *D*. Using `assets.quantity` would apply
today's holding to every past date — a stack sold down from 10 to 5 would
render as though it had always been 5.

**Valuations are carried forward, not interpolated.** A price recorded on
Jan 15 stays in effect until the next one. The flat line between points is
honest: it says *we last knew this on Jan 15*, not *it was worth this every
day*. Interpolating would invent measurements that were never taken.

**Valuations are never carried backward.** Before an asset's first valuation,
it contributes nothing — not its later price. Otherwise buying something would
retroactively enrich the past.

---

## 2. How quantity changes render

| Event | Effect on the chart |
|---|---|
| Acquire / Add | Value steps **up** at the effective date. This is not appreciation — see below. |
| Remove | Value steps **down** proportionally; earlier points keep the old quantity. |
| Dispose | Contribution **ends** at that date. Earlier points are untouched. |
| Correct | Adjusts from the effective date. A data fix, not a trade. |

**The distinction the chart must preserve:** a rise because you *bought more*
is not a rise because your holdings *gained value*. Both move the line up.

Since decision 2 excludes tax-lot accounting, the app does not compute
time-weighted return, which is the usual way to separate them. So the chart
must not present the total as a performance figure. Concretely:

- Points where a quantity event occurred are **marked**, so a step is visibly
  attributable to a purchase or sale.
- The chart is labelled *portfolio value*, never *return*, *performance* or
  *growth*.
- Percentage change between two arbitrary points is **not** offered, because
  over any range containing a purchase it would be meaningless.

A future time-weighted series would need per-flow data the app deliberately
does not keep. Saying so is better than approximating it.

---

## 3. How coverage gaps are shown

A portfolio total is a sum over the assets that could be priced. That is fine
as long as it never looks complete when it isn't.

- Every point carries `valued` and `unvalued` counts.
- Any point with `unvalued > 0` is rendered **visually distinct** (dashed
  segment or a hollow marker) and its tooltip states how many holdings are
  missing a price.
- The series is **not** annotated as "estimated" when coverage is full — an
  unqualified figure should mean full coverage.
- An asset with no valuation contributes **nothing**, and is counted in
  `unvalued`. It is never treated as zero-valued, which would be
  indistinguishable from a worthless item.

**Foreign currency.** Until an FX layer exists, holdings in another currency
are excluded and named, not converted at an assumed rate. A chart that
silently mixed EUR into a USD total would be wrong in a way nobody could see.

---

## 4. Range and resolution

- Points are computed at a **fixed daily step**, not at valuation dates. An
  uneven x-axis makes a slow drift look like a cliff.
- The series starts at the earliest event across the portfolio, not at the
  first valuation: the period when assets were held but unpriced is real and
  should be visible as low coverage rather than hidden.
- Long ranges are downsampled by taking the **last point in each bucket**
  (not the mean), so a rendered point is always a value that genuinely held
  on some date.

---

## 5. What is deliberately not offered

Each of these would require data the app does not keep, and approximating
them would produce plausible wrong numbers:

- **Time-weighted or money-weighted return** — needs per-flow cost data
  (decision 2).
- **Realized gain/loss** — needs tax lots (decision 2).
- **Benchmark comparison** — needs market index data (decision 4, no paid
  feeds).
- **Forecasts or trend lines** — extrapolating from a handful of manual
  valuations would dress a guess as analysis.

---

## 6. Test obligations

Any chart implementation must hold these, and they are asserted in
`am-storage`'s series tests:

1. A holding sold in June appears in a March point and not a July one.
2. A holding whose quantity halves shows the old quantity at earlier points.
3. An asset with no valuation raises `unvalued` and adds nothing to the total.
4. A valuation is carried forward to later dates but never backward.
5. A point before any event in the portfolio is zero with zero coverage, not
   an error.
