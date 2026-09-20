//! Spot prices for precious metals.
//!
//! Three things the plan requires, none of which are optional:
//!
//! - **Persisted quota accounting.** metals.dev's free tier allows 100
//!   requests per month. A budget that resets when the app restarts is not a
//!   budget.
//! - **Manual spot override.** A dead API or an exhausted quota must never
//!   block valuation. A hand-entered price is a first-class input, not a
//!   failure mode.
//! - **Staleness display.** Under a constrained budget a price is routinely a
//!   day or two old. That is shown, not hidden and not treated as an error.

use am_core::{parse_decimal, Currency, Decimal, Freshness, Metal, QuotaBudget};
use serde::Serialize;

use crate::valuations::{self, MatchQuality, NewQuote, ValuationError};
use crate::vault::Vault;

/// A spot price with everything needed to judge it.
#[derive(Debug, Clone, Serialize)]
pub struct SpotPrice {
    pub metal: String,
    /// Exact decimal as text — a sub-cent rounding would destroy a silver
    /// position.
    pub price_per_troy_oz: String,
    pub currency: String,
    /// When the *source* priced it, not when it was fetched.
    pub source_asof: String,
    pub source: String,
    /// "current", "recent", "stale", "out of date".
    pub freshness: String,
    /// True when a value derived from this price should carry a caveat.
    pub needs_caveat: bool,
    pub age_hours: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuotaStatus {
    pub used_this_month: u32,
    pub monthly_limit: u32,
    pub remaining: u32,
    /// False once automatic polling has spent its allowance. Manual refresh
    /// keeps working past this point, which is the entire reason the two are
    /// budgeted separately.
    pub may_poll_automatically: bool,
    pub may_refresh_manually: bool,
    pub automatic_interval_hours: i64,
}

/// Hours between two RFC-3339-ish timestamps.
///
/// Compares the shared `YYYY-MM-DDTHH` prefix, which is enough to grade
/// staleness in whole hours and avoids a date-library dependency for one
/// subtraction.
fn age_hours(from: &str, to: &str) -> i64 {
    fn to_hours(ts: &str) -> Option<i64> {
        let date = ts.get(..10)?;
        let mut parts = date.split('-');
        let y: i64 = parts.next()?.parse().ok()?;
        let m: i64 = parts.next()?.parse().ok()?;
        let d: i64 = parts.next()?.parse().ok()?;

        // Days-from-civil, exact for the Gregorian calendar.
        let y_adj = if m <= 2 { y - 1 } else { y };
        let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
        let yoe = y_adj - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;

        let hour: i64 = ts.get(11..13).and_then(|h| h.parse().ok()).unwrap_or(0);
        Some(days * 24 + hour)
    }

    match (to_hours(from), to_hours(to)) {
        (Some(a), Some(b)) => b - a,
        // An unparseable timestamp is treated as very old rather than fresh:
        // failing safe means showing a caveat, not hiding one.
        _ => i64::MAX / 2,
    }
}

/// Where a spot price came from.
///
/// An enum rather than a boolean flag: `record_spot(.., true, ..)` at a call
/// site says nothing about what is true, and this distinction decides whether
/// the API quota is charged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotOrigin {
    /// Fetched from a metered provider. Charges one request against the
    /// monthly budget.
    Api,
    /// Also from the provider, but on a request already charged.
    ///
    /// One metals.dev call returns all four metals. Charging each stored
    /// price would spend the budget four times faster than requests are
    /// actually made, so only the first is charged — while all four keep
    /// API provenance, because that is where they came from.
    ApiSameRequest,
    /// Typed in by the owner. Free, and always available even with the quota
    /// exhausted or the provider down.
    Manual,
}

impl SpotOrigin {
    fn charges_quota(self) -> bool {
        matches!(self, SpotOrigin::Api)
    }

    fn is_from_provider(self) -> bool {
        matches!(self, SpotOrigin::Api | SpotOrigin::ApiSameRequest)
    }
}

/// A spot price to record.
pub struct SpotReading<'a> {
    pub metal: Metal,
    pub price_per_troy_oz: Decimal,
    pub currency: &'a Currency,
    pub source: &'a str,
    /// When the *source* priced it, not when it was fetched.
    pub source_asof: &'a str,
    pub origin: SpotOrigin,
}

/// Record a spot price.
pub fn record_spot(
    vault: &Vault,
    reading: SpotReading<'_>,
    now: &str,
) -> Result<String, ValuationError> {
    let SpotReading { metal, price_per_troy_oz, currency, source, source_asof, origin } =
        reading;
    let quote_id = valuations::record_quote(
        vault,
        &NewQuote {
            instrument_id: metal.instrument_id(),
            unit_quote: price_per_troy_oz,
            currency: currency.clone(),
            quote_unit: "troy_oz".into(),
            source: source.to_string(),
            match_quality: if origin.is_from_provider() {
                MatchQuality::Exact
            } else {
                MatchQuality::Manual
            },
            source_asof: source_asof.to_string(),
        },
        now,
    )?;

    if origin.charges_quota() {
        record_api_use(vault, source, now)?;
    }
    Ok(quote_id)
}

/// Latest spot price, with staleness judged against `now`.
pub fn latest_spot(
    vault: &Vault,
    metal: Metal,
    now: &str,
) -> Result<Option<SpotPrice>, ValuationError> {
    let row: Option<(String, String, String, String)> = vault
        .conn()
        .query_row(
            "SELECT unit_quote, currency, source_asof, source FROM quotes
             WHERE instrument_id = ?1
             ORDER BY source_asof DESC, fetched_at DESC LIMIT 1",
            [metal.instrument_id()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();

    let Some((price, currency, source_asof, source)) = row else { return Ok(None) };

    let age = age_hours(&source_asof, now);
    let freshness = Freshness::from_age_hours(age);

    Ok(Some(SpotPrice {
        metal: metal.code().to_string(),
        price_per_troy_oz: price,
        currency,
        source_asof,
        source,
        freshness: freshness.label().to_string(),
        needs_caveat: freshness.needs_caveat(),
        age_hours: age,
    }))
}

/// Count one API request against this month's budget.
fn record_api_use(vault: &Vault, provider: &str, now: &str) -> Result<(), ValuationError> {
    let period = &now[..7]; // YYYY-MM
    let budget = QuotaBudget::default();

    vault.conn().execute(
        "INSERT INTO provider_quota (provider, period, used, limit_total)
         VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(provider, period) DO UPDATE SET used = used + 1",
        rusqlite::params![provider, period, budget.monthly_limit],
    )?;
    Ok(())
}

pub fn quota_status(
    vault: &Vault,
    provider: &str,
    now: &str,
) -> Result<QuotaStatus, ValuationError> {
    let period = &now[..7];
    let budget = QuotaBudget::default();

    let used: u32 = vault
        .conn()
        .query_row(
            "SELECT used FROM provider_quota WHERE provider = ?1 AND period = ?2",
            rusqlite::params![provider, period],
            |r| r.get(0),
        )
        .unwrap_or(0);

    Ok(QuotaStatus {
        used_this_month: used,
        monthly_limit: budget.monthly_limit,
        remaining: budget.remaining(used),
        may_poll_automatically: budget.may_poll_automatically(used),
        may_refresh_manually: budget.may_refresh_manually(used),
        automatic_interval_hours: budget.automatic_interval_hours(),
    })
}

/// Whether enough time has passed for another automatic poll.
///
/// Checks both the clock and the budget: an interval alone would still
/// overrun in a long month, and a budget alone would spend the whole
/// allowance on the first day.
pub fn should_poll_automatically(
    vault: &Vault,
    metal: Metal,
    provider: &str,
    now: &str,
) -> Result<bool, ValuationError> {
    let status = quota_status(vault, provider, now)?;
    if !status.may_poll_automatically {
        return Ok(false);
    }

    match latest_spot(vault, metal, now)? {
        None => Ok(true), // never fetched
        Some(spot) => Ok(spot.age_hours >= status.automatic_interval_hours),
    }
}

/// Parse a hand-entered spot price.
pub fn parse_spot_input(text: &str) -> Result<Decimal, ValuationError> {
    let value =
        parse_decimal(text).map_err(|_| ValuationError::BadDecimal(text.to_string()))?;
    if value <= Decimal::ZERO {
        return Err(ValuationError::BadDecimal(format!(
            "{text}: a spot price must be greater than zero"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_crypto::KdfParams;
    use std::str::FromStr;

    const PASS: &str = "correct horse battery staple";
    const NOW: &str = "2026-09-19T12:00:00Z";

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

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn age_in_hours_is_exact_across_days_and_months() {
        assert_eq!(age_hours("2026-09-19T12:00:00Z", "2026-09-19T12:00:00Z"), 0);
        assert_eq!(age_hours("2026-09-19T12:00:00Z", "2026-09-19T15:00:00Z"), 3);
        assert_eq!(age_hours("2026-09-19T12:00:00Z", "2026-09-20T12:00:00Z"), 24);
        assert_eq!(age_hours("2026-08-31T00:00:00Z", "2026-09-01T00:00:00Z"), 24);
        // Leap day.
        assert_eq!(age_hours("2024-02-28T00:00:00Z", "2024-03-01T00:00:00Z"), 48);
    }

    #[test]
    fn an_unparseable_timestamp_fails_safe_as_very_old() {
        // Better to show a caveat wrongly than to present junk as fresh.
        assert!(age_hours("garbage", NOW) > 24 * 365);
    }

    #[test]
    fn spot_roundtrips_with_full_precision() {
        let (_d, v) = setup();
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Silver,
                price_per_troy_oz: d("30.125"),
                currency: &usd(),
                source: "test",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Api,
            },
            NOW,
        )
        .unwrap();

        let spot = latest_spot(&v, Metal::Silver, NOW).unwrap().unwrap();
        assert_eq!(spot.price_per_troy_oz, "30.125");
        assert_eq!(spot.metal, "XAG");
        assert_eq!(spot.freshness, "current");
        assert!(!spot.needs_caveat);
    }

    #[test]
    fn staleness_is_reported_not_hidden() {
        // Two bands, since the boundary is where a mislabel would hide.
        // 2026-08-01 to 2026-09-19 is ~49 days, past the 30-day cutoff.
        let (_d, v) = setup();
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "test",
                source_asof: "2026-08-01T12:00:00Z",
                origin: SpotOrigin::Api,
            },
            NOW,
        )
        .unwrap();

        let spot = latest_spot(&v, Metal::Gold, NOW).unwrap().unwrap();
        assert_eq!(spot.freshness, "out of date", "49 days is past the 30-day cutoff");
        assert!(spot.needs_caveat);
        assert!(spot.age_hours > 24 * 30);

        // Two weeks old: stale, but still usable.
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Silver,
                price_per_troy_oz: d("30"),
                currency: &usd(),
                source: "test",
                source_asof: "2026-09-05T12:00:00Z",
                origin: SpotOrigin::Api,
            },
            NOW,
        )
        .unwrap();
        let silver = latest_spot(&v, Metal::Silver, NOW).unwrap().unwrap();
        assert_eq!(silver.freshness, "stale");
        assert!(silver.needs_caveat, "past a week, a caveat is warranted");
    }

    #[test]
    fn a_day_old_price_does_not_nag() {
        // Normal under a 100-request monthly budget; nagging would train the
        // user to ignore the warning that matters.
        let (_d, v) = setup();
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "test",
                source_asof: "2026-09-18T12:00:00Z",
                origin: SpotOrigin::Api,
            },
            NOW,
        )
        .unwrap();

        let spot = latest_spot(&v, Metal::Gold, NOW).unwrap().unwrap();
        assert_eq!(spot.freshness, "recent");
        assert!(!spot.needs_caveat);
    }

    #[test]
    fn manual_entry_does_not_consume_the_api_quota() {
        let (_d, v) = setup();
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "manual",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Manual,
            },
            NOW,
        )
        .unwrap();

        let status = quota_status(&v, "metals.dev", NOW).unwrap();
        assert_eq!(status.used_this_month, 0, "a hand-entered price is not an API call");
    }

    #[test]
    fn quota_accounting_persists_across_reopen() {
        // A budget that resets on restart is not a budget.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");

        {
            let (v, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
            for _ in 0..5 {
                record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "metals.dev",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Api,
            },
            NOW,
        )
        .unwrap();
            }
        }

        let v = Vault::unlock(&root, crate::header::Credential::Passphrase, PASS).unwrap();
        let status = quota_status(&v, "metals.dev", NOW).unwrap();
        assert_eq!(status.used_this_month, 5);
        assert_eq!(status.remaining, 95);
    }

    #[test]
    fn quota_is_tracked_per_month() {
        let (_d, v) = setup();
        for _ in 0..3 {
            record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "metals.dev",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Api,
            },
            "2026-09-19T12:00:00Z",
        )
        .unwrap();
        }

        assert_eq!(quota_status(&v, "metals.dev", "2026-09-30T00:00:00Z").unwrap().used_this_month, 3);
        assert_eq!(
            quota_status(&v, "metals.dev", "2026-10-01T00:00:00Z").unwrap().used_this_month,
            0,
            "a new month starts a fresh allowance"
        );
    }

    #[test]
    fn automatic_polling_stops_before_manual_refresh_does() {
        let (_d, v) = setup();
        let budget = QuotaBudget::default();

        for _ in 0..budget.automatic_allowance() {
            record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "metals.dev",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Api,
            },
            NOW,
        )
        .unwrap();
        }

        let status = quota_status(&v, "metals.dev", NOW).unwrap();
        assert!(!status.may_poll_automatically, "automatic polling has spent its share");
        assert!(status.may_refresh_manually, "the user can still ask for a fresh price");
        assert_eq!(status.remaining, 75);
    }

    #[test]
    fn automatic_polling_respects_the_interval() {
        let (_d, v) = setup();

        // Just fetched.
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "metals.dev",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Api,
            },
            NOW,
        )
        .unwrap();
        assert!(
            !should_poll_automatically(&v, Metal::Gold, "metals.dev", NOW).unwrap(),
            "must not poll again an hour later"
        );

        // Two days on, past the ~30-hour interval.
        assert!(should_poll_automatically(
            &v, Metal::Gold, "metals.dev", "2026-09-21T12:00:00Z"
        )
        .unwrap());
    }

    #[test]
    fn a_metal_never_fetched_is_polled_immediately() {
        let (_d, v) = setup();
        assert!(should_poll_automatically(&v, Metal::Platinum, "metals.dev", NOW).unwrap());
    }

    #[test]
    fn metals_have_independent_prices() {
        let (_d, v) = setup();
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "t",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Manual,
            },
            NOW,
        )
        .unwrap();
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Silver,
                price_per_troy_oz: d("30"),
                currency: &usd(),
                source: "t",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Manual,
            },
            NOW,
        )
        .unwrap();

        assert_eq!(
            latest_spot(&v, Metal::Gold, NOW).unwrap().unwrap().price_per_troy_oz,
            "2000"
        );
        assert_eq!(
            latest_spot(&v, Metal::Silver, NOW).unwrap().unwrap().price_per_troy_oz,
            "30"
        );
        assert!(latest_spot(&v, Metal::Palladium, NOW).unwrap().is_none());
    }

    #[test]
    fn one_request_returning_four_metals_charges_once() {
        // The budget counts requests, not prices. Charging per metal would
        // exhaust a 100-request month in 25 actual calls.
        let (_d, v) = setup();
        for (i, metal) in
            [Metal::Gold, Metal::Silver, Metal::Platinum, Metal::Palladium].iter().enumerate()
        {
            record_spot(
                &v,
                SpotReading {
                    metal: *metal,
                    price_per_troy_oz: d("100"),
                    currency: &usd(),
                    source: "metals.dev",
                    source_asof: "2026-09-19T11:00:00Z",
                    origin: if i == 0 { SpotOrigin::Api } else { SpotOrigin::ApiSameRequest },
                },
                NOW,
            )
            .unwrap();
        }

        let status = quota_status(&v, "metals.dev", NOW).unwrap();
        assert_eq!(status.used_this_month, 1, "four prices, one request");

        // All four keep provider provenance despite only one being charged.
        let stored: i64 = v
            .conn()
            .query_row(
                "SELECT count(*) FROM quotes WHERE match_quality = 'exact'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, 4, "every fetched price is API-sourced");
    }

    #[test]
    fn manual_input_rejects_nonsense() {
        assert!(parse_spot_input("2000.50").is_ok());
        assert!(parse_spot_input("0").is_err(), "zero is not a price");
        assert!(parse_spot_input("-30").is_err(), "negative is not a price");
        assert!(parse_spot_input("free").is_err());
        assert!(parse_spot_input("").is_err());
    }

    #[test]
    fn a_newer_source_price_wins_over_a_newer_fetch() {
        let (_d, v) = setup();
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("2000"),
                currency: &usd(),
                source: "t",
                source_asof: "2026-09-19T11:00:00Z",
                origin: SpotOrigin::Manual,
            },
            "2026-09-19T11:05:00Z",
        )
        .unwrap();
        // Fetched later, but priced earlier — a cached response.
        record_spot(
            &v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: d("1900"),
                currency: &usd(),
                source: "t",
                source_asof: "2026-09-19T08:00:00Z",
                origin: SpotOrigin::Manual,
            },
            "2026-09-19T14:00:00Z",
        )
        .unwrap();

        let spot = latest_spot(&v, Metal::Gold, NOW).unwrap().unwrap();
        assert_eq!(spot.price_per_troy_oz, "2000", "source time decides, not fetch time");
    }
}
