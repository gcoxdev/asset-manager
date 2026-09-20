//! Portfolio value over time.
//!
//! Implements `docs/chart-semantics.md`. The rules that matter, restated
//! because getting them wrong produces a confident lie:
//!
//! - Holdings come from the event log, replayed to each date — never from
//!   today's `assets.quantity`.
//! - Valuations carry **forward** from their date, never backward.
//! - An unpriced holding contributes nothing and is **counted**, so a partial
//!   total is never mistaken for a complete one.
//!
//! Points are computed at a fixed daily step: spacing them at valuation dates
//! would make a slow drift look like a cliff.

use am_core::{Currency, Money};
use serde::Serialize;

use crate::valuations::{portfolio_total_as_of, ValuationError};
use crate::vault::Vault;

#[derive(Debug, Clone, Serialize)]
pub struct SeriesPoint {
    /// ISO date, `YYYY-MM-DD`.
    pub date: String,
    /// Display-ready, e.g. `"1234.50 USD"`. Formatted here so no float is
    /// involved and per-currency minor digits are respected.
    pub total: String,
    /// Raw minor units, as text — JS numbers are 53-bit.
    pub total_minor: String,
    pub valued: usize,
    /// Holdings with no usable price on this date. Any point with a non-zero
    /// value here must be rendered as incomplete.
    pub unvalued: usize,
    /// True when a quantity event took effect on this date, so a step can be
    /// attributed to a purchase or sale rather than read as appreciation.
    pub quantity_event: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Series {
    pub points: Vec<SeriesPoint>,
    pub currency: String,
    /// Currencies held but excluded for want of an FX layer.
    pub skipped_currencies: Vec<String>,
    /// True when every point had full coverage. Only then may the chart be
    /// presented without a caveat.
    pub complete: bool,
}

/// Days between two ISO dates, treating both as plain calendar dates.
///
/// Deliberately simple: dates here are `YYYY-MM-DD` strings from the event
/// log, and pulling in a full calendar library for subtraction would be
/// overkill. Uses days-from-civil, which is exact for the Gregorian calendar.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    // Howard Hinnant's algorithm; exact, no floating point.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn parse_date(iso: &str) -> Option<i64> {
    let date = iso.get(..10)?;
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

fn format_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

#[derive(Debug, Clone, Copy)]
pub struct SeriesRequest<'a> {
    pub from: &'a str,
    pub to: &'a str,
    /// Cap on returned points. Longer ranges are downsampled by taking the
    /// **last** point in each bucket, so every rendered point is a value that
    /// genuinely held on some date.
    pub max_points: usize,
}

/// Build a portfolio series.
pub fn portfolio_series(
    vault: &Vault,
    request: SeriesRequest<'_>,
    currency: &Currency,
) -> Result<Series, ValuationError> {
    let from = parse_date(request.from)
        .ok_or_else(|| ValuationError::BadDecimal(request.from.to_string()))?;
    let to = parse_date(request.to)
        .ok_or_else(|| ValuationError::BadDecimal(request.to.to_string()))?;

    if to < from {
        return Ok(Series {
            points: Vec::new(),
            currency: currency.code().to_string(),
            skipped_currencies: Vec::new(),
            complete: true,
        });
    }

    // Dates on which a quantity event took effect, so steps can be attributed.
    let event_days: std::collections::HashSet<i64> = {
        let mut stmt =
            vault.conn().prepare("SELECT DISTINCT effective_date FROM asset_events")?;
        let dates: Vec<String> =
            stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
        dates.iter().filter_map(|d| parse_date(d)).collect()
    };

    let span = (to - from + 1) as usize;
    let step = span.div_ceil(request.max_points.max(1)).max(1);

    let mut points = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut complete = true;

    let mut day = from;
    while day <= to {
        // Take the last day of each bucket, so a downsampled point is a real
        // value rather than an average that never occurred.
        let bucket_end = (day + step as i64 - 1).min(to);
        let date = format_date(bucket_end);

        let total = portfolio_total_as_of(vault, &date, currency)?;
        if total.unvalued > 0 {
            complete = false;
        }
        for code in &total.skipped_currencies {
            if !skipped.contains(code) {
                skipped.push(code.clone());
            }
        }

        // Any event inside the bucket explains a step at this point.
        let quantity_event = (day..=bucket_end).any(|d| event_days.contains(&d));

        points.push(SeriesPoint {
            date,
            total: total.total.format(),
            total_minor: total.total.amount_minor.to_string(),
            valued: total.valued,
            unvalued: total.unvalued,
            quantity_event,
        });

        day = bucket_end + 1;
    }

    skipped.sort();
    Ok(Series {
        points,
        currency: currency.code().to_string(),
        skipped_currencies: skipped,
        complete,
    })
}

/// Earliest date anything happened, for a default chart range.
///
/// Starts at the first *event*, not the first valuation: a period when assets
/// were held but unpriced is real, and should show as low coverage rather
/// than be hidden.
pub fn earliest_activity(vault: &Vault) -> Result<Option<String>, ValuationError> {
    let earliest: Option<String> = vault
        .conn()
        .query_row("SELECT min(effective_date) FROM asset_events", [], |r| {
            r.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten();
    Ok(earliest)
}

/// Zero-value point, for ranges with no data.
pub fn empty_point(date: &str, currency: &Currency) -> SeriesPoint {
    let zero = Money::zero(currency.clone());
    SeriesPoint {
        date: date.to_string(),
        total: zero.format(),
        total_minor: "0".into(),
        valued: 0,
        unvalued: 0,
        quantity_event: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{self, EventType, NewEvent};
    use crate::valuations::{record_valuation, Basis, NewValuation, Provenance};
    use am_core::Decimal;
    use am_crypto::KdfParams;
    use std::str::FromStr;

    const PASS: &str = "correct horse battery staple";
    const NOW: &str = "2026-09-19T00:00:00Z";

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    fn usd() -> Currency {
        Currency::new("USD").unwrap()
    }

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let (vault, _r) = Vault::create(&dir.path().join("vault"), PASS, &fast(), NOW).unwrap();
        (dir, vault)
    }

    fn add_asset(v: &Vault, id: &str) {
        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, quantity, created_at, updated_at)
                 VALUES (?1,'generic',?1,'0',?2,?2)",
                rusqlite::params![id, NOW],
            )
            .unwrap();
    }

    fn event(v: &Vault, id: &str, kind: EventType, delta: &str, date: &str) {
        events::record(
            v,
            &NewEvent {
                asset_id: id.into(),
                event_type: kind,
                effective_date: date.into(),
                quantity_delta: Decimal::from_str(delta).unwrap(),
                amount_minor: None,
                currency: None,
                note: String::new(),
            },
            NOW,
        )
        .unwrap();
    }

    fn value(v: &Vault, id: &str, minor: i64, qty: &str, asof: &str) {
        record_valuation(
            v,
            &NewValuation {
                asset_id: id.into(),
                quote_id: None,
                value: Money::new(minor, usd()),
                quantity_at_time: Decimal::from_str(qty).unwrap(),
                basis: Basis::EstimatedResale,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: asof.into(),
            },
            NOW,
        )
        .unwrap();
    }

    fn series(v: &Vault, from: &str, to: &str, max: usize) -> Series {
        portfolio_series(v, SeriesRequest { from, to, max_points: max }, &usd()).unwrap()
    }

    #[test]
    fn date_arithmetic_is_exact() {
        // Including a leap day, which off-by-one bugs love.
        assert_eq!(parse_date("2026-01-01").unwrap() + 1, parse_date("2026-01-02").unwrap());
        assert_eq!(parse_date("2024-02-28").unwrap() + 1, parse_date("2024-02-29").unwrap());
        assert_eq!(parse_date("2024-02-29").unwrap() + 1, parse_date("2024-03-01").unwrap());
        assert_eq!(parse_date("2026-12-31").unwrap() + 1, parse_date("2027-01-01").unwrap());

        for iso in ["2026-01-01", "2024-02-29", "1999-12-31"] {
            assert_eq!(format_date(parse_date(iso).unwrap()), iso);
        }
        assert!(parse_date("not-a-date").is_none());
        assert!(parse_date("2026-13-01").is_none());
    }

    /// Obligation 1: a holding sold in June appears in a March point, not July.
    #[test]
    fn a_disposed_holding_leaves_later_points_only() {
        let (_d, v) = setup();
        add_asset(&v, "a1");
        event(&v, "a1", EventType::Acquire, "1", "2026-01-01");
        value(&v, "a1", 100_000, "1", "2026-01-15");
        event(&v, "a1", EventType::Dispose, "-1", "2026-06-01");

        let s = series(&v, "2026-01-01", "2026-12-31", 365);
        let march = s.points.iter().find(|p| p.date == "2026-03-01").unwrap();
        let july = s.points.iter().find(|p| p.date == "2026-07-01").unwrap();

        assert_eq!(march.total, "1000.00 USD");
        assert_eq!(july.total, "0.00 USD", "a sold holding must not persist");
    }

    /// Obligation 2: earlier points keep the earlier quantity.
    #[test]
    fn earlier_points_keep_the_earlier_quantity() {
        let (_d, v) = setup();
        add_asset(&v, "a1");
        event(&v, "a1", EventType::Acquire, "10", "2026-01-01");
        value(&v, "a1", 300_000, "10", "2026-01-15");
        event(&v, "a1", EventType::Remove, "-5", "2026-06-01");
        value(&v, "a1", 150_000, "5", "2026-06-01");

        let s = series(&v, "2026-01-01", "2026-12-31", 365);
        let march = s.points.iter().find(|p| p.date == "2026-03-01").unwrap();
        let july = s.points.iter().find(|p| p.date == "2026-07-01").unwrap();

        assert_eq!(march.total, "3000.00 USD", "March held 10, not today's 5");
        assert_eq!(july.total, "1500.00 USD");
    }

    /// Obligation 3: an unpriced holding adds nothing and is counted.
    #[test]
    fn unpriced_holdings_are_counted_not_zeroed() {
        let (_d, v) = setup();
        add_asset(&v, "priced");
        add_asset(&v, "unpriced");
        event(&v, "priced", EventType::Acquire, "1", "2026-01-01");
        event(&v, "unpriced", EventType::Acquire, "1", "2026-01-01");
        value(&v, "priced", 50_000, "1", "2026-01-15");

        let s = series(&v, "2026-02-01", "2026-02-01", 1);
        let point = &s.points[0];

        assert_eq!(point.total, "500.00 USD");
        assert_eq!(point.valued, 1);
        assert_eq!(point.unvalued, 1, "the unpriced holding must be visible");
        assert!(!s.complete, "the series must not claim completeness");
    }

    /// Obligation 4: valuations carry forward, never backward.
    #[test]
    fn valuations_carry_forward_but_never_backward() {
        let (_d, v) = setup();
        add_asset(&v, "a1");
        event(&v, "a1", EventType::Acquire, "1", "2026-01-01");
        value(&v, "a1", 100_000, "1", "2026-03-01");

        let s = series(&v, "2026-01-01", "2026-06-01", 365);

        let before = s.points.iter().find(|p| p.date == "2026-02-01").unwrap();
        assert_eq!(before.total, "0.00 USD", "a later price must not enrich the past");
        assert_eq!(before.unvalued, 1, "...and the holding is counted as unpriced");

        let after = s.points.iter().find(|p| p.date == "2026-05-01").unwrap();
        assert_eq!(after.total, "1000.00 USD", "the last known price carries forward");
    }

    /// Obligation 5: a point before any activity is zero, not an error.
    #[test]
    fn dates_before_any_activity_are_zero_with_no_coverage() {
        let (_d, v) = setup();
        add_asset(&v, "a1");
        event(&v, "a1", EventType::Acquire, "1", "2026-06-01");
        value(&v, "a1", 100_000, "1", "2026-06-01");

        let s = series(&v, "2026-01-01", "2026-01-31", 31);
        let point = &s.points[0];

        assert_eq!(point.total, "0.00 USD");
        assert_eq!(point.valued, 0);
        assert_eq!(point.unvalued, 0, "nothing was held, so nothing is missing a price");
        assert!(s.complete, "zero holdings is complete coverage, not a gap");
    }

    #[test]
    fn quantity_events_are_marked_so_steps_can_be_explained() {
        // A step from buying more is not appreciation, and must be
        // attributable.
        let (_d, v) = setup();
        add_asset(&v, "a1");
        event(&v, "a1", EventType::Acquire, "1", "2026-01-01");
        value(&v, "a1", 100_000, "1", "2026-01-01");
        event(&v, "a1", EventType::Add, "1", "2026-06-15");
        value(&v, "a1", 200_000, "2", "2026-06-15");

        let s = series(&v, "2026-06-14", "2026-06-16", 3);
        let marked: Vec<&str> =
            s.points.iter().filter(|p| p.quantity_event).map(|p| p.date.as_str()).collect();
        assert_eq!(marked, ["2026-06-15"], "only the purchase date is marked");
    }

    #[test]
    fn downsampling_returns_real_values_not_averages() {
        let (_d, v) = setup();
        add_asset(&v, "a1");
        event(&v, "a1", EventType::Acquire, "1", "2026-01-01");
        value(&v, "a1", 100_000, "1", "2026-01-01");
        value(&v, "a1", 200_000, "1", "2026-01-20");

        // A month of data into 3 points.
        let s = series(&v, "2026-01-01", "2026-01-31", 3);
        assert!(s.points.len() <= 3);

        // Every value must be one the portfolio genuinely held.
        for point in &s.points {
            assert!(
                ["1000.00 USD", "2000.00 USD"].contains(&point.total.as_str()),
                "downsampling invented the value {}",
                point.total
            );
        }
        assert_eq!(s.points.last().unwrap().date, "2026-01-31", "range end is included");
    }

    #[test]
    fn foreign_currency_is_excluded_and_named() {
        let (_d, v) = setup();
        add_asset(&v, "usd");
        add_asset(&v, "eur");
        event(&v, "usd", EventType::Acquire, "1", "2026-01-01");
        event(&v, "eur", EventType::Acquire, "1", "2026-01-01");
        value(&v, "usd", 10_000, "1", "2026-01-01");

        record_valuation(
            &v,
            &NewValuation {
                asset_id: "eur".into(),
                quote_id: None,
                value: Money::new(50_000, Currency::new("EUR").unwrap()),
                quantity_at_time: Decimal::ONE,
                basis: Basis::EstimatedResale,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: "2026-01-01".into(),
            },
            NOW,
        )
        .unwrap();

        let s = series(&v, "2026-02-01", "2026-02-01", 1);
        assert_eq!(s.points[0].total, "100.00 USD", "only the USD holding");
        assert_eq!(s.skipped_currencies, vec!["EUR"]);
        assert!(!s.complete);
    }

    #[test]
    fn an_inverted_range_yields_nothing_rather_than_panicking() {
        let (_d, v) = setup();
        let s = series(&v, "2026-06-01", "2026-01-01", 10);
        assert!(s.points.is_empty());
    }

    #[test]
    fn earliest_activity_finds_the_first_event() {
        let (_d, v) = setup();
        assert_eq!(earliest_activity(&v).unwrap(), None);

        add_asset(&v, "a1");
        event(&v, "a1", EventType::Acquire, "1", "2026-03-15");
        event(&v, "a1", EventType::Add, "1", "2026-01-10");

        assert_eq!(
            earliest_activity(&v).unwrap().as_deref(),
            Some("2026-01-10"),
            "the earliest event, not the earliest recorded"
        );
    }

    #[test]
    fn a_single_day_range_produces_one_point() {
        let (_d, v) = setup();
        add_asset(&v, "a1");
        event(&v, "a1", EventType::Acquire, "1", "2026-01-01");
        value(&v, "a1", 100_000, "1", "2026-01-01");

        let s = series(&v, "2026-06-01", "2026-06-01", 100);
        assert_eq!(s.points.len(), 1);
        assert_eq!(s.points[0].date, "2026-06-01");
    }
}
