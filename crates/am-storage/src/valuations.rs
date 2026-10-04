//! Quotes and valuations — deliberately two separate things.
//!
//! A **quote** is about a market instrument: "gold was $2,014.30/troy oz at
//! 14:00". It is fetched once and reused by every holding that references it.
//!
//! A **valuation** is about *your* holding: "on that date I held 10 oz, worth
//! $20,143". It captures the quantity and inputs used, which is what lets a
//! historical chart stay correct after the quantity changes.
//!
//! Conflating them is the bug that makes charts lie, so they never share a
//! table or a function.
//!
//! # Confidence is four things, not one
//!
//! Collapsing these into a single dot loses the distinctions that matter:
//!
//! - **provenance** — manual, API, or appraisal
//! - **freshness** — age by the *source's* timestamp, not our fetch time
//! - **match quality** — exact identity vs. approximate
//! - **basis** — melt, replacement, insured, or estimated resale
//!
//! An exact identity match against a raw-card marketplace listing is not a
//! high-confidence graded-card price.

use am_core::{parse_decimal, Currency, Decimal, Money};
use serde::{Deserialize, Serialize};

use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum ValuationError {
    #[error("unknown asset: {0}")]
    UnknownAsset(String),
    #[error("{0:?} is not a valid decimal")]
    BadDecimal(String),
    #[error("{0}")]
    BadCurrency(String),
    #[error(transparent)]
    History(#[from] crate::events::EventError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

/// Where a number came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provenance {
    /// Entered by the owner. The most common case for collectibles, and not
    /// inferior — an owner who looked up comps is often more accurate than a
    /// thin automated match.
    Manual,
    Api,
    Appraisal,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::Manual => "manual",
            Provenance::Api => "api",
            Provenance::Appraisal => "appraisal",
        }
    }
}

/// What the number is measuring. Keeping these distinct matters: melt value
/// and insured replacement value are different numbers for the same coin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Basis {
    Melt,
    Replacement,
    Insured,
    EstimatedResale,
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Basis::Melt => "melt",
            Basis::Replacement => "replacement",
            Basis::Insured => "insured",
            Basis::EstimatedResale => "estimated_resale",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchQuality {
    Exact,
    Approximate,
    Manual,
}

impl MatchQuality {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchQuality::Exact => "exact",
            MatchQuality::Approximate => "approximate",
            MatchQuality::Manual => "manual",
        }
    }
}

/// A price for a market instrument. Not about any particular holding.
#[derive(Debug, Clone)]
pub struct NewQuote {
    /// Stable identity: `metal:XAU`, `coingecko:bitcoin`. Not a bare symbol —
    /// one symbol can name several tokens.
    pub instrument_id: String,
    /// Exact decimal, never minor units: a sub-cent quote rounded to cents
    /// becomes zero and takes the whole position with it.
    pub unit_quote: Decimal,
    pub currency: Currency,
    /// What one unit is: `troy_oz`, `coin`, `item`.
    pub quote_unit: String,
    pub source: String,
    pub match_quality: MatchQuality,
    /// When the *source* priced it.
    pub source_asof: String,
}

/// What a holding was worth, and the inputs used.
#[derive(Debug, Clone)]
pub struct NewValuation {
    pub asset_id: String,
    pub quote_id: Option<String>,
    pub value: Money,
    /// Quantity held *at the valuation date*. Recording it is what keeps a
    /// chart correct after the holding changes.
    pub quantity_at_time: Decimal,
    pub basis: Basis,
    pub provenance: Provenance,
    /// Weights, purities, premiums — whatever produced the number, so it can
    /// be explained later.
    pub inputs: serde_json::Value,
    pub asof: String,
}

#[derive(Debug, Clone)]
pub struct Valuation {
    pub valuation_id: String,
    pub asset_id: String,
    pub value: Money,
    pub quantity_at_time: Decimal,
    pub basis: String,
    pub provenance: String,
    pub asof: String,
    pub recorded_at: String,
}

fn uuid_v4() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw[6] = (raw[6] & 0x0f) | 0x40;
    raw[8] = (raw[8] & 0x3f) | 0x80;
    let h: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

pub fn record_quote(
    vault: &Vault,
    quote: &NewQuote,
    now: &str,
) -> Result<String, ValuationError> {
    let quote_id = uuid_v4();
    vault.conn().execute(
        "INSERT INTO quotes
           (quote_id, instrument_id, unit_quote, currency, quote_unit, source,
            match_quality, source_asof, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            &quote_id,
            &quote.instrument_id,
            // Stored as text so full precision survives the round trip.
            quote.unit_quote.to_string(),
            quote.currency.code(),
            &quote.quote_unit,
            &quote.source,
            quote.match_quality.as_str(),
            &quote.source_asof,
            now
        ],
    )?;
    Ok(quote_id)
}

/// Most recent quote for an instrument, by the source's timestamp.
///
/// Ordered by `source_asof`, not `fetched_at`: re-fetching an unchanged stale
/// price must not make it look newly valued.
pub fn latest_quote(
    vault: &Vault,
    instrument_id: &str,
) -> Result<Option<(String, Decimal, Currency, String)>, ValuationError> {
    let row: Option<(String, String, String, String)> = vault
        .conn()
        .query_row(
            "SELECT quote_id, unit_quote, currency, source_asof FROM quotes
             WHERE instrument_id = ?1
             ORDER BY source_asof DESC, fetched_at DESC, rowid DESC LIMIT 1",
            [instrument_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();

    let Some((quote_id, raw_quote, currency, asof)) = row else { return Ok(None) };
    let unit_quote =
        parse_decimal(&raw_quote).map_err(|_| ValuationError::BadDecimal(raw_quote))?;
    let currency =
        Currency::new(&currency).map_err(|e| ValuationError::BadCurrency(e.to_string()))?;
    Ok(Some((quote_id, unit_quote, currency, asof)))
}

/// Record a valuation and refresh the asset's cached current value.
pub fn record_valuation(
    vault: &Vault,
    valuation: &NewValuation,
    now: &str,
) -> Result<String, ValuationError> {
    let exists: i64 = vault.conn().query_row(
        "SELECT count(*) FROM assets WHERE asset_id = ?1",
        [&valuation.asset_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(ValuationError::UnknownAsset(valuation.asset_id.clone()));
    }

    let valuation_id = uuid_v4();
    let tx = crate::atomic::begin(vault.conn())?;

    tx.execute(
        "INSERT INTO valuations
           (valuation_id, asset_id, quote_id, amount_minor, currency, quantity_at_time,
            basis, provenance, inputs, asof, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            &valuation_id,
            &valuation.asset_id,
            &valuation.quote_id,
            valuation.value.amount_minor,
            valuation.value.currency.code(),
            valuation.quantity_at_time.to_string(),
            valuation.basis.as_str(),
            valuation.provenance.as_str(),
            valuation.inputs.to_string(),
            &valuation.asof,
            now
        ],
    )?;

    refresh_current_value_in(&tx, &valuation.asset_id, now)?;
    tx.commit()?;
    Ok(valuation_id)
}

/// Scale a whole-holding valuation to the quantity actually held.
///
/// A valuation is the value of the *whole* holding at `quantity_at_time`.
/// When the quantity has since changed — sold half, bought more — the
/// figure in effect is that value in proportion, which is what
/// `docs/chart-semantics.md` means by "steps down proportionally".
///
/// Left unscaled when either side is zero: a disposed holding contributes
/// nothing anyway, and a valuation recorded at quantity zero (written by an
/// older build's same-day date bug) carries no ratio to apply.
pub fn scale_to_quantity(
    value: &Money,
    quantity_at_time: Decimal,
    quantity_now: Decimal,
) -> Result<Money, ValuationError> {
    if quantity_at_time == quantity_now
        || quantity_at_time <= Decimal::ZERO
        || quantity_now <= Decimal::ZERO
    {
        return Ok(value.clone());
    }
    let scaled = value
        .to_decimal()
        .checked_mul(quantity_now)
        .and_then(|v| v.checked_div(quantity_at_time))
        .ok_or_else(|| ValuationError::BadDecimal("scaled value overflows".into()))?;
    Money::from_total_decimal(scaled, value.currency.clone())
        .map_err(|e| ValuationError::BadDecimal(e.to_string()))
}

/// Refresh `assets.current_*` from the latest valuation.
///
/// The asset's current value is a **derived cache**, never independently
/// edited, so the two can never disagree. It is scaled to the quantity held
/// now, so recording a sale moves the figure without a new valuation.
pub(crate) fn refresh_current_value_in(
    tx: &rusqlite::Connection,
    asset_id: &str,
    now: &str,
) -> Result<(), ValuationError> {
    let latest: Option<(i64, String, String, String, String)> = tx
        .query_row(
            "SELECT amount_minor, currency, provenance, asof, quantity_at_time FROM valuations
             WHERE asset_id = ?1 ORDER BY asof DESC, recorded_at DESC, rowid DESC LIMIT 1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .ok();

    if let Some((amount, currency, provenance, asof, at_time)) = latest {
        let quantity_now: String =
            tx.query_row("SELECT quantity FROM assets WHERE asset_id = ?1", [asset_id], |r| {
                r.get(0)
            })?;
        let code =
            Currency::new(&currency).map_err(|e| ValuationError::BadCurrency(e.to_string()))?;
        let at_time =
            parse_decimal(&at_time).map_err(|_| ValuationError::BadDecimal(at_time))?;
        let quantity_now = parse_decimal(&quantity_now)
            .map_err(|_| ValuationError::BadDecimal(quantity_now))?;
        let scaled = scale_to_quantity(&Money::new(amount, code), at_time, quantity_now)?;

        tx.execute(
            "UPDATE assets
             SET current_amount_minor = ?1, current_currency = ?2,
                 value_source = ?3, value_asof = ?4, updated_at = ?5
             WHERE asset_id = ?6",
            rusqlite::params![scaled.amount_minor, currency, provenance, asof, now, asset_id],
        )?;
    }
    Ok(())
}

/// Valuation in effect on a date — the figure a historical chart should use.
pub fn valuation_as_of(
    vault: &Vault,
    asset_id: &str,
    as_of: &str,
) -> Result<Option<Valuation>, ValuationError> {
    let row = vault
        .conn()
        .query_row(
            "SELECT valuation_id, asset_id, amount_minor, currency, quantity_at_time,
                    basis, provenance, asof, recorded_at
             FROM valuations
             WHERE asset_id = ?1 AND asof <= ?2
             ORDER BY asof DESC, recorded_at DESC, rowid DESC LIMIT 1",
            rusqlite::params![asset_id, as_of],
            |r| {
                let amount: i64 = r.get(2)?;
                let currency: String = r.get(3)?;
                let quantity: String = r.get(4)?;
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    amount,
                    currency,
                    quantity,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                ))
            },
        )
        .ok();

    let Some((id, asset, amount, currency, quantity, basis, provenance, asof, recorded)) = row
    else {
        return Ok(None);
    };

    let currency =
        Currency::new(&currency).map_err(|e| ValuationError::BadCurrency(e.to_string()))?;
    Ok(Some(Valuation {
        valuation_id: id,
        asset_id: asset,
        value: Money::new(amount, currency),
        quantity_at_time: parse_decimal(&quantity)
            .map_err(|_| ValuationError::BadDecimal(quantity))?,
        basis,
        provenance,
        asof,
        recorded_at: recorded,
    }))
}

/// Portfolio total on a date, plus coverage.
///
/// Returns how many assets had a valuation and how many did not. **A missing
/// price stays unknown rather than counting as zero**, which would understate
/// a collection and look identical to a worthless item.
pub fn portfolio_total_as_of(
    vault: &Vault,
    as_of: &str,
    currency: &Currency,
) -> Result<PortfolioTotal, ValuationError> {
    // Every asset, whatever its status today: one lost in October still
    // counts in March. Whether it counts on *this* date is asked below.
    let mut stmt = vault.conn().prepare("SELECT asset_id FROM assets")?;
    let asset_ids: Vec<String> =
        stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut total = Money::zero(currency.clone());
    let mut valued = 0usize;
    let mut unvalued = 0usize;
    let mut skipped_currencies = Vec::new();

    for asset_id in asset_ids {
        // A holding disposed of before this date contributes nothing.
        let quantity = crate::events::quantity_as_of(vault, &asset_id, Some(as_of))
            .map_err(|e| ValuationError::BadDecimal(e.to_string()))?;
        if quantity == Decimal::ZERO {
            continue;
        }
        // Lost or retired on this date: not held in any sense a total means.
        if !crate::lifecycle::counts_on(vault, &asset_id, as_of)? {
            continue;
        }

        match valuation_as_of(vault, &asset_id, as_of)? {
            Some(valuation) => {
                if &valuation.value.currency != currency {
                    // No FX conversion yet, so a foreign-currency holding is
                    // reported separately rather than added incorrectly.
                    skipped_currencies.push(valuation.value.currency.code().to_string());
                    unvalued += 1;
                    continue;
                }
                // The valuation covers the holding as it was then; scale it
                // to what was held on this date.
                let held =
                    scale_to_quantity(&valuation.value, valuation.quantity_at_time, quantity)?;
                total = total
                    .checked_add(&held)
                    .map_err(|e| ValuationError::BadDecimal(e.to_string()))?;
                valued += 1;
            }
            None => unvalued += 1,
        }
    }

    skipped_currencies.sort();
    skipped_currencies.dedup();

    Ok(PortfolioTotal { total, valued, unvalued, skipped_currencies })
}

#[derive(Debug, Clone)]
pub struct PortfolioTotal {
    pub total: Money,
    /// Assets that contributed to the total.
    pub valued: usize,
    /// Assets held but with no usable valuation. Shown alongside the total so
    /// a partial figure is never mistaken for a complete one.
    pub unvalued: usize,
    /// Currencies present but not converted.
    pub skipped_currencies: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{self, EventType, NewEvent};
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

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let (vault, _r) = Vault::create(&dir.path().join("vault"), PASS, &fast(), NOW).unwrap();
        (dir, vault)
    }

    fn add_asset(v: &Vault, id: &str, name: &str) {
        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, quantity, created_at, updated_at)
                 VALUES (?1,'generic',?2,'0',?3,?3)",
                rusqlite::params![id, name, NOW],
            )
            .unwrap();
    }

    fn acquire(v: &Vault, id: &str, qty: &str, date: &str) {
        events::record(
            v,
            &NewEvent {
                asset_id: id.into(),
                event_type: EventType::Acquire,
                effective_date: date.into(),
                quantity_delta: d(qty),
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
                quantity_at_time: d(qty),
                basis: Basis::EstimatedResale,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: asof.into(),
            },
            NOW,
        )
        .unwrap();
    }

    #[test]
    fn quotes_keep_full_precision() {
        let (_d, v) = setup();
        record_quote(
            &v,
            &NewQuote {
                instrument_id: "coingecko:some-token".into(),
                unit_quote: d("0.00000123"),
                currency: usd(),
                quote_unit: "coin".into(),
                source: "test".into(),
                match_quality: MatchQuality::Exact,
                source_asof: "2026-09-19T12:00:00Z".into(),
            },
            NOW,
        )
        .unwrap();

        let (_, quote, _, _) = latest_quote(&v, "coingecko:some-token").unwrap().unwrap();
        assert_eq!(quote, d("0.00000123"), "a sub-cent quote must not be rounded away");
    }

    #[test]
    fn latest_quote_uses_source_time_not_fetch_time() {
        // Re-fetching a stale price must not make it look fresh.
        let (_d, v) = setup();

        record_quote(
            &v,
            &NewQuote {
                instrument_id: "metal:XAU".into(),
                unit_quote: d("2000"),
                currency: usd(),
                quote_unit: "troy_oz".into(),
                source: "test".into(),
                match_quality: MatchQuality::Exact,
                source_asof: "2026-09-19T12:00:00Z".into(),
            },
            "2026-09-19T12:00:05Z",
        )
        .unwrap();

        // Fetched later, but the source priced it earlier.
        record_quote(
            &v,
            &NewQuote {
                instrument_id: "metal:XAU".into(),
                unit_quote: d("1900"),
                currency: usd(),
                quote_unit: "troy_oz".into(),
                source: "test".into(),
                match_quality: MatchQuality::Exact,
                source_asof: "2026-09-19T09:00:00Z".into(),
            },
            "2026-09-19T15:00:00Z",
        )
        .unwrap();

        let (_, quote, _, asof) = latest_quote(&v, "metal:XAU").unwrap().unwrap();
        assert_eq!(quote, d("2000"), "the newer source price wins, not the newer fetch");
        assert_eq!(asof, "2026-09-19T12:00:00Z");
    }

    #[test]
    fn current_value_is_a_cache_of_the_latest_valuation() {
        let (_d, v) = setup();
        add_asset(&v, "a1", "Coin");
        acquire(&v, "a1", "1", "2026-01-01");

        value(&v, "a1", 100_000, "1", "2026-01-15");
        value(&v, "a1", 120_000, "1", "2026-06-15");

        let (amount, source): (i64, String) = v
            .conn()
            .query_row(
                "SELECT current_amount_minor, value_source FROM assets WHERE asset_id='a1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(amount, 120_000, "cache follows the most recent valuation");
        assert_eq!(source, "manual");
    }

    #[test]
    fn historical_valuation_does_not_use_todays_number() {
        let (_d, v) = setup();
        add_asset(&v, "a1", "Coin");
        acquire(&v, "a1", "1", "2026-01-01");

        value(&v, "a1", 100_000, "1", "2026-01-15");
        value(&v, "a1", 150_000, "1", "2026-06-15");

        let march = valuation_as_of(&v, "a1", "2026-03-01").unwrap().unwrap();
        assert_eq!(march.value.format(), "1000.00 USD", "March should not see June's price");

        let july = valuation_as_of(&v, "a1", "2026-07-01").unwrap().unwrap();
        assert_eq!(july.value.format(), "1500.00 USD");

        // Before any valuation existed, there is no number — not zero.
        assert!(valuation_as_of(&v, "a1", "2025-12-01").unwrap().is_none());
    }

    #[test]
    fn a_disposed_holding_leaves_todays_total_but_keeps_its_past() {
        // The chart-honesty case, end to end.
        let (_d, v) = setup();
        add_asset(&v, "a1", "Sold Coin");
        acquire(&v, "a1", "1", "2026-01-01");
        value(&v, "a1", 100_000, "1", "2026-01-15");

        events::record(
            &v,
            &NewEvent {
                asset_id: "a1".into(),
                event_type: EventType::Dispose,
                effective_date: "2026-06-01".into(),
                quantity_delta: d("-1"),
                amount_minor: Some(110_000),
                currency: Some("USD".into()),
                note: "sold".into(),
            },
            NOW,
        )
        .unwrap();

        let march = portfolio_total_as_of(&v, "2026-03-01", &usd()).unwrap();
        assert_eq!(march.total.format(), "1000.00 USD", "still held in March");
        assert_eq!(march.valued, 1);

        let july = portfolio_total_as_of(&v, "2026-07-01", &usd()).unwrap();
        assert_eq!(july.total.format(), "0.00 USD", "no longer held in July");
        assert_eq!(july.valued, 0);
    }

    #[test]
    fn missing_valuations_are_counted_not_treated_as_zero() {
        let (_d, v) = setup();
        add_asset(&v, "a1", "Valued");
        add_asset(&v, "a2", "Not valued");
        acquire(&v, "a1", "1", "2026-01-01");
        acquire(&v, "a2", "1", "2026-01-01");
        value(&v, "a1", 50_000, "1", "2026-01-15");

        let total = portfolio_total_as_of(&v, "2026-06-01", &usd()).unwrap();
        assert_eq!(total.total.format(), "500.00 USD");
        assert_eq!(total.valued, 1);
        assert_eq!(total.unvalued, 1, "an unpriced holding must be visible, not silently zero");
    }

    #[test]
    fn foreign_currency_holdings_are_reported_not_mixed_in() {
        // Adding EUR to USD would be wrong; there is no FX layer yet.
        let (_d, v) = setup();
        add_asset(&v, "a1", "Dollars");
        add_asset(&v, "a2", "Euros");
        acquire(&v, "a1", "1", "2026-01-01");
        acquire(&v, "a2", "1", "2026-01-01");

        value(&v, "a1", 10_000, "1", "2026-01-15");
        record_valuation(
            &v,
            &NewValuation {
                asset_id: "a2".into(),
                quote_id: None,
                value: Money::new(20_000, Currency::new("EUR").unwrap()),
                quantity_at_time: Decimal::ONE,
                basis: Basis::EstimatedResale,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: "2026-01-15".into(),
            },
            NOW,
        )
        .unwrap();

        let total = portfolio_total_as_of(&v, "2026-06-01", &usd()).unwrap();
        assert_eq!(total.total.format(), "100.00 USD", "only the USD holding");
        assert_eq!(total.skipped_currencies, vec!["EUR"]);
        assert_eq!(total.unvalued, 1);
    }

    #[test]
    fn valuations_record_the_quantity_at_the_time() {
        let (_d, v) = setup();
        add_asset(&v, "a1", "Stack");
        acquire(&v, "a1", "10", "2026-01-01");
        value(&v, "a1", 300_000, "10", "2026-01-15");

        let recorded = valuation_as_of(&v, "a1", "2026-02-01").unwrap().unwrap();
        assert_eq!(
            recorded.quantity_at_time,
            Decimal::from(10),
            "the quantity used must be stored, not re-derived later"
        );
    }

    #[test]
    fn valuations_on_unknown_assets_are_refused() {
        let (_d, v) = setup();
        let result = record_valuation(
            &v,
            &NewValuation {
                asset_id: "nope".into(),
                quote_id: None,
                value: Money::new(100, usd()),
                quantity_at_time: Decimal::ONE,
                basis: Basis::Melt,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: "2026-01-01".into(),
            },
            NOW,
        );
        assert!(matches!(result, Err(ValuationError::UnknownAsset(_))));
    }

    #[test]
    fn basis_and_provenance_are_preserved() {
        let (_d, v) = setup();
        add_asset(&v, "a1", "Coin");
        acquire(&v, "a1", "1", "2026-01-01");

        record_valuation(
            &v,
            &NewValuation {
                asset_id: "a1".into(),
                quote_id: None,
                value: Money::new(200_000, usd()),
                quantity_at_time: Decimal::ONE,
                basis: Basis::Insured,
                provenance: Provenance::Appraisal,
                inputs: serde_json::json!({"appraiser": "test"}),
                asof: "2026-01-15".into(),
            },
            NOW,
        )
        .unwrap();

        let recorded = valuation_as_of(&v, "a1", "2026-02-01").unwrap().unwrap();
        assert_eq!(recorded.basis, "insured", "insured value is not resale value");
        assert_eq!(recorded.provenance, "appraisal");
    }

    #[test]
    fn scaling_follows_the_quantity_held() {
        let value = Money::new(300_000, usd());
        let d = |s: &str| Decimal::from_str(s).unwrap();

        assert_eq!(scale_to_quantity(&value, d("10"), d("5")).unwrap().amount_minor, 150_000);
        assert_eq!(scale_to_quantity(&value, d("10"), d("15")).unwrap().amount_minor, 450_000);
        assert_eq!(scale_to_quantity(&value, d("10"), d("10")).unwrap().amount_minor, 300_000);
        // No ratio to apply.
        assert_eq!(scale_to_quantity(&value, d("0"), d("3")).unwrap().amount_minor, 300_000);
        // Rounds once, half-even, at the end.
        assert_eq!(
            scale_to_quantity(&Money::new(100, usd()), d("3"), d("1")).unwrap().amount_minor,
            33
        );
    }

    #[test]
    fn an_asset_bought_today_counts_in_todays_total() {
        // Regression: the acquire event used to carry a timestamp, which
        // compared after today's date and dropped the asset from the total.
        let dir = tempfile::tempdir().unwrap();
        let (v, _r) = Vault::create(&dir.path().join("vault"), PASS, &fast(), NOW).unwrap();
        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, quantity, created_at, updated_at)
                 VALUES ('a1','generic','Watch','0',?1,?1)",
                [NOW],
            )
            .unwrap();
        events::record(
            &v,
            &NewEvent {
                asset_id: "a1".into(),
                event_type: EventType::Acquire,
                effective_date: "2026-09-19T15:30:00Z".into(),
                quantity_delta: Decimal::ONE,
                amount_minor: None,
                currency: None,
                note: String::new(),
            },
            NOW,
        )
        .unwrap();
        record_valuation(
            &v,
            &NewValuation {
                asset_id: "a1".into(),
                quote_id: None,
                value: Money::new(500_000, usd()),
                quantity_at_time: Decimal::ONE,
                basis: Basis::EstimatedResale,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: "2026-09-19".into(),
            },
            NOW,
        )
        .unwrap();

        let total = portfolio_total_as_of(&v, "2026-09-19", &usd()).unwrap();
        assert_eq!(total.valued, 1);
        assert_eq!(total.total.amount_minor, 500_000);
    }

    #[test]
    fn a_portfolio_total_scales_a_valuation_to_the_quantity_then_held() {
        let dir = tempfile::tempdir().unwrap();
        let (v, _r) = Vault::create(&dir.path().join("vault"), PASS, &fast(), NOW).unwrap();
        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, quantity, created_at, updated_at)
                 VALUES ('a1','silver_bullion','Eagles','0',?1,?1)",
                [NOW],
            )
            .unwrap();
        let ev = |kind, delta: &str, date: &str| NewEvent {
            asset_id: "a1".into(),
            event_type: kind,
            effective_date: date.into(),
            quantity_delta: Decimal::from_str(delta).unwrap(),
            amount_minor: None,
            currency: None,
            note: String::new(),
        };
        events::record(&v, &ev(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();
        record_valuation(
            &v,
            &NewValuation {
                asset_id: "a1".into(),
                quote_id: None,
                value: Money::new(300_000, usd()),
                quantity_at_time: Decimal::from(10),
                basis: Basis::EstimatedResale,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: "2026-01-15".into(),
            },
            NOW,
        )
        .unwrap();
        events::record(&v, &ev(EventType::Remove, "-5", "2026-06-01"), NOW).unwrap();

        let march = portfolio_total_as_of(&v, "2026-03-01", &usd()).unwrap();
        let july = portfolio_total_as_of(&v, "2026-07-01", &usd()).unwrap();
        assert_eq!(march.total.amount_minor, 300_000, "before the sale, all ten");
        assert_eq!(july.total.amount_minor, 150_000, "after it, five of ten");
    }
}
