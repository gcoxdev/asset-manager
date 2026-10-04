//! Market revaluation: turning stored quotes into holding valuations.
//!
//! Quotes are about instruments ("gold was $2,014/oz"); valuations are about
//! holdings ("my 10 Eagles were worth $22,000"). This module is the one place
//! that joins the two, so a price refresh actually moves the portfolio
//! rather than sitting in a table nobody reads.
//!
//! # Precedence
//!
//! Only assets with `pricing = 'market'` are touched. Entering a value by
//! hand switches an asset to `manual` (see migration 003), so a refresh can
//! never overwrite a hand-entered valuation — the rule is enforced by what
//! gets selected, not by a timestamp comparison that could be got wrong.
//!
//! # Idempotence
//!
//! A new valuation is written only when the quote or the quantity differs
//! from the last one recorded. Revaluing twice in a row writes nothing, so a
//! refresh button pressed repeatedly does not pad the history.

use std::collections::BTreeMap;
use std::str::FromStr;

use am_core::{
    parse_decimal, valuation::WeightBasis, valuation::WeightUnit, CoinIdentity, Currency,
    Decimal, Metal, MetalHolding, Money,
};
use serde::Serialize;

use crate::valuations::{record_valuation, Basis, NewValuation, Provenance};
use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum PricingError {
    #[error("{field}: {reason}")]
    BadAttr { field: &'static str, reason: String },
    #[error("{0}")]
    Storage(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

fn bad(field: &'static str, reason: impl Into<String>) -> PricingError {
    PricingError::BadAttr { field, reason: reason.into() }
}

/// What an asset tracks, read from its attributes.
#[derive(Debug, Clone, PartialEq)]
pub enum MarketSpec {
    Metal {
        metal: Metal,
        weight_per_item: Decimal,
        unit: WeightUnit,
        basis: WeightBasis,
        purity: Decimal,
        /// Percent over melt, e.g. `5` for 5%. May be negative for a discount.
        premium_pct: Decimal,
    },
    Coin {
        identity: CoinIdentity,
    },
}

pub fn unit_name(unit: WeightUnit) -> &'static str {
    match unit {
        WeightUnit::TroyOunce => "troy_oz",
        WeightUnit::Gram => "gram",
        WeightUnit::Pennyweight => "pennyweight",
        WeightUnit::Ounce => "ounce",
    }
}

pub fn parse_unit(name: &str) -> Option<WeightUnit> {
    Some(match name {
        "troy_oz" => WeightUnit::TroyOunce,
        "gram" => WeightUnit::Gram,
        "pennyweight" => WeightUnit::Pennyweight,
        "ounce" => WeightUnit::Ounce,
        _ => return None,
    })
}

impl MarketSpec {
    /// Read a spec from attributes. `Ok(None)` means the asset tracks no
    /// market — an ordinary collectible.
    pub fn from_attrs(attrs: &BTreeMap<String, String>) -> Result<Option<Self>, PricingError> {
        let get = |k: &str| attrs.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());

        if let Some(metal) = get("metal") {
            let metal = Metal::parse(metal).ok_or_else(|| bad("metal", "unknown metal"))?;
            let weight = parse_decimal(get("weight_per_item").unwrap_or(""))
                .map_err(|_| bad("weight", "enter the weight of one item"))?;
            if weight <= Decimal::ZERO {
                return Err(bad("weight", "must be greater than zero"));
            }
            let unit = parse_unit(get("weight_unit").unwrap_or("troy_oz"))
                .ok_or_else(|| bad("weight unit", "unknown unit"))?;
            let basis = match get("weight_basis").unwrap_or("gross") {
                "gross" => WeightBasis::Gross,
                "fine" => WeightBasis::Fine,
                _ => return Err(bad("weight basis", "must be gross or fine")),
            };
            let purity = parse_decimal(get("purity").unwrap_or("0.999"))
                .map_err(|_| bad("purity", "enter a fraction such as 0.999"))?;
            if purity <= Decimal::ZERO || purity > Decimal::ONE {
                return Err(bad("purity", "must be a fraction between 0 and 1, e.g. 0.9167"));
            }
            let premium_pct = match get("premium_pct") {
                None => Decimal::ZERO,
                Some(p) => parse_decimal(p.trim_end_matches('%'))
                    .map_err(|_| bad("premium", "enter a percentage such as 5"))?,
            };
            if premium_pct <= Decimal::from(-100) {
                return Err(bad("premium", "a discount of 100% or more leaves nothing"));
            }
            return Ok(Some(MarketSpec::Metal {
                metal,
                weight_per_item: weight,
                unit,
                basis,
                purity,
                premium_pct,
            }));
        }

        if let Some(coin_id) = get("coin_id") {
            let symbol = get("symbol").unwrap_or(coin_id);
            let identity = match (get("chain"), get("contract")) {
                (Some(chain), Some(contract)) => {
                    CoinIdentity::token(coin_id, symbol, chain, contract)
                }
                _ => CoinIdentity::native(coin_id, symbol),
            }
            .map_err(|e| bad("coin", e.to_string()))?;
            return Ok(Some(MarketSpec::Coin { identity }));
        }

        Ok(None)
    }

    pub fn instrument_id(&self) -> String {
        match self {
            MarketSpec::Metal { metal, .. } => metal.instrument_id(),
            MarketSpec::Coin { identity } => identity.instrument_id(),
        }
    }

    /// Value a holding of `quantity` at a unit quote.
    ///
    /// Returns the value, what it measures, and the inputs used — stored with
    /// the valuation so the figure can be explained later.
    pub fn value(
        &self,
        quantity: Decimal,
        unit_quote: Decimal,
        currency: Currency,
    ) -> Result<(Money, Basis, serde_json::Value), PricingError> {
        match self {
            MarketSpec::Metal { metal, weight_per_item, unit, basis, purity, premium_pct } => {
                let holding = MetalHolding {
                    quantity,
                    weight_per_item: *weight_per_item,
                    weight_unit: *unit,
                    weight_basis: *basis,
                    purity: *purity,
                };
                let fine = holding
                    .fine_weight(WeightUnit::TroyOunce)
                    .map_err(|e| PricingError::Storage(e.to_string()))?;
                let fraction = *premium_pct / Decimal::from(100);
                let value = holding
                    .market_value(unit_quote, fraction, currency)
                    .map_err(|e| PricingError::Storage(e.to_string()))?;
                // With no premium the number *is* melt; with one it is an
                // estimate of what a dealer would pay. Keeping them distinct
                // is the point of recording a basis.
                let valuation_basis =
                    if premium_pct.is_zero() { Basis::Melt } else { Basis::EstimatedResale };
                let inputs = serde_json::json!({
                    "metal": metal.code(),
                    "spot_per_troy_oz": unit_quote.to_string(),
                    "fine_troy_oz": fine.round_dp(6).normalize().to_string(),
                    "premium_pct": premium_pct.normalize().to_string(),
                });
                Ok((value, valuation_basis, inputs))
            }
            MarketSpec::Coin { identity } => {
                if quantity.is_sign_negative() {
                    return Err(PricingError::Storage("negative quantity".into()));
                }
                // Full precision until the final rounding: a sub-cent price
                // times a large balance is where early rounding erases value.
                let total = quantity
                    .checked_mul(unit_quote)
                    .ok_or_else(|| PricingError::Storage("value overflows".into()))?;
                let value = Money::from_total_decimal(total, currency)
                    .map_err(|e| PricingError::Storage(e.to_string()))?;
                let inputs = serde_json::json!({
                    "coin_id": identity.coin_id,
                    "unit_price": unit_quote.to_string(),
                });
                Ok((value, Basis::EstimatedResale, inputs))
            }
        }
    }
}

/// What happened when one asset was revalued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", content = "reason", rename_all = "snake_case")]
pub enum Outcome {
    Updated,
    /// Same quote and quantity as the last valuation; nothing written.
    Unchanged,
    /// No usable quote, or the asset does not track a market.
    Unpriced(String),
    /// Manually priced, sold, or otherwise not eligible.
    Skipped,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RevalueSummary {
    pub updated: usize,
    pub unchanged: usize,
    /// Names of market-priced holdings that could not be priced, with why.
    pub unpriced: Vec<(String, String)>,
}

struct Quote {
    quote_id: String,
    unit_quote: Decimal,
    currency: Currency,
    manual: bool,
}

fn latest_quote(vault: &Vault, instrument_id: &str) -> Result<Option<Quote>, PricingError> {
    let row: Option<(String, String, String, String)> = vault
        .conn()
        .query_row(
            "SELECT quote_id, unit_quote, currency, match_quality FROM quotes
             WHERE instrument_id = ?1
             ORDER BY source_asof DESC, fetched_at DESC, rowid DESC LIMIT 1",
            [instrument_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok();
    let Some((quote_id, raw, currency, quality)) = row else { return Ok(None) };
    Ok(Some(Quote {
        quote_id,
        unit_quote: Decimal::from_str(&raw)
            .map_err(|_| PricingError::Storage(format!("stored quote {raw:?} is malformed")))?,
        currency: Currency::new(&currency).map_err(|e| PricingError::Storage(e.to_string()))?,
        manual: quality == "manual",
    }))
}

/// Revalue one asset if it follows the market.
///
/// `today` is the owner's calendar date, which dates the valuation; `now` is
/// the UTC timestamp it is recorded at.
pub fn revalue_asset(
    vault: &Vault,
    asset_id: &str,
    now: &str,
    today: &str,
) -> Result<Outcome, PricingError> {
    let row: Option<(String, String, String)> = vault
        .conn()
        .query_row(
            "SELECT pricing, CASE WHEN deleted_at IS NULL THEN status ELSE 'trashed' END, attrs
             FROM assets WHERE asset_id = ?1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let Some((pricing, status, attrs_json)) = row else {
        return Err(PricingError::Storage(format!("unknown asset: {asset_id}")));
    };
    if pricing != "market" || status != "active" {
        return Ok(Outcome::Skipped);
    }

    let attrs: BTreeMap<String, String> = serde_json::from_str(&attrs_json).unwrap_or_default();
    let spec = match MarketSpec::from_attrs(&attrs) {
        Ok(Some(spec)) => spec,
        Ok(None) => return Ok(Outcome::Unpriced("does not track a market price".into())),
        Err(e) => return Ok(Outcome::Unpriced(e.to_string())),
    };

    let quantity = crate::events::quantity_as_of(vault, asset_id, None)
        .map_err(|e| PricingError::Storage(e.to_string()))?;
    if quantity <= Decimal::ZERO {
        return Ok(Outcome::Skipped);
    }

    let Some(quote) = latest_quote(vault, &spec.instrument_id())? else {
        let what = match &spec {
            MarketSpec::Metal { metal, .. } => format!("no {} spot price yet", metal),
            MarketSpec::Coin { identity } => format!("no price yet for {}", identity.coin_id),
        };
        return Ok(Outcome::Unpriced(what));
    };

    // Skip when nothing has changed since the last valuation.
    let last: Option<(Option<String>, String)> = vault
        .conn()
        .query_row(
            "SELECT quote_id, quantity_at_time FROM valuations
             WHERE asset_id = ?1 AND voided_at IS NULL
             ORDER BY asof DESC, recorded_at DESC, rowid DESC LIMIT 1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    if let Some((Some(last_quote), last_qty)) = &last {
        let same_qty = parse_decimal(last_qty).map(|q| q == quantity).unwrap_or(false);
        if *last_quote == quote.quote_id && same_qty {
            return Ok(Outcome::Unchanged);
        }
    }

    let (value, basis, inputs) = spec.value(quantity, quote.unit_quote, quote.currency)?;
    record_valuation(
        vault,
        &NewValuation {
            asset_id: asset_id.to_string(),
            quote_id: Some(quote.quote_id),
            value,
            quantity_at_time: quantity,
            basis,
            // A holding valued from a hand-typed spot price is a manual
            // figure, whatever arithmetic sits between.
            provenance: if quote.manual { Provenance::Manual } else { Provenance::Api },
            inputs,
            asof: today.to_string(),
        },
        now,
    )
    .map_err(|e| PricingError::Storage(e.to_string()))?;
    Ok(Outcome::Updated)
}

/// Revalue every market-priced holding.
pub fn revalue_all(
    vault: &Vault,
    now: &str,
    today: &str,
) -> Result<RevalueSummary, PricingError> {
    let assets: Vec<(String, String)> = {
        let mut stmt = vault.conn().prepare(
            "SELECT asset_id, name FROM assets
             WHERE pricing = 'market' AND status = 'active' AND deleted_at IS NULL",
        )?;
        let rows =
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
        rows
    };

    let mut summary = RevalueSummary::default();
    for (asset_id, name) in assets {
        match revalue_asset(vault, &asset_id, now, today)? {
            Outcome::Updated => summary.updated += 1,
            Outcome::Unchanged => summary.unchanged += 1,
            Outcome::Unpriced(reason) => summary.unpriced.push((name, reason)),
            Outcome::Skipped => {}
        }
    }
    Ok(summary)
}

/// Coin IDs of every held, market-priced crypto asset, for a batched fetch.
///
/// Only what is held: every ID costs provider credits, and asking about coins
/// nobody owns would spend them for nothing — and tell the provider more
/// than it needs to know.
pub fn held_coin_ids(vault: &Vault) -> Result<Vec<String>, PricingError> {
    let mut stmt = vault.conn().prepare(
        "SELECT DISTINCT json_extract(attrs, '$.coin_id') FROM assets
         WHERE status = 'active' AND deleted_at IS NULL
           AND json_extract(attrs, '$.coin_id') IS NOT NULL",
    )?;
    let ids: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{self, NewAsset, Pricing};
    use crate::spot::{record_spot, SpotOrigin, SpotReading};
    use crate::valuations::{record_quote, MatchQuality, NewQuote};
    use am_crypto::KdfParams;

    const PASS: &str = "correct horse battery staple";
    const NOW: &str = "2026-09-19T10:00:00Z";

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let (vault, _r) = Vault::create(&dir.path().join("vault"), PASS, &fast(), NOW).unwrap();
        (dir, vault)
    }

    fn usd() -> Currency {
        Currency::new("USD").unwrap()
    }

    fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn eagles(v: &Vault, quantity: i64, premium: &str) -> String {
        assets::create(
            v,
            &NewAsset {
                type_id: "gold_bullion".into(),
                name: "Gold Eagles".into(),
                quantity: Decimal::from(quantity),
                quantity_unit: "coin".into(),
                acquired_date: Some("2026-01-01".into()),
                effective_date: None,
                acquired_cost: None,
                acquired_from: None,
                storage_location: None,
                notes: String::new(),
                insured: None,
                attrs: attrs(&[
                    ("metal", "XAU"),
                    ("weight_per_item", "1.0909"),
                    ("weight_unit", "troy_oz"),
                    ("weight_basis", "gross"),
                    ("purity", "0.9167"),
                    ("premium_pct", premium),
                ]),
                pricing: Pricing::Market,
                review_every_days: None,
            },
            NOW,
        )
        .unwrap()
    }

    fn gold_spot(v: &Vault, price: &str, origin: SpotOrigin, asof: &str) {
        record_spot(
            v,
            SpotReading {
                metal: Metal::Gold,
                price_per_troy_oz: Decimal::from_str(price).unwrap(),
                currency: &usd(),
                source: "test",
                source_asof: asof,
                origin,
            },
            NOW,
        )
        .unwrap();
    }

    fn current(v: &Vault, id: &str) -> (Option<i64>, Option<String>) {
        v.conn()
            .query_row(
                "SELECT current_amount_minor, value_source FROM assets WHERE asset_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn a_spot_price_values_a_market_holding() {
        let (_d, v) = setup();
        let id = eagles(&v, 10, "");
        gold_spot(&v, "2000", SpotOrigin::Api, "2026-09-19T09:00:00Z");

        assert_eq!(revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap(), Outcome::Updated);
        // 10 × 1.0909 × 0.9167 = 10.0002803 oz × $2000 = $20,000.56
        let (minor, source) = current(&v, &id);
        assert_eq!(minor, Some(2_000_056));
        assert_eq!(source.as_deref(), Some("api"));

        let basis: String = v
            .conn()
            .query_row("SELECT basis FROM valuations WHERE asset_id = ?1", [&id], |r| r.get(0))
            .unwrap();
        assert_eq!(basis, "melt", "no premium means the figure is melt value");
    }

    #[test]
    fn a_premium_raises_the_value_and_changes_the_basis() {
        let (_d, v) = setup();
        let id = eagles(&v, 1, "5");
        gold_spot(&v, "2000", SpotOrigin::Api, "2026-09-19T09:00:00Z");
        revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap();

        let (minor, _) = current(&v, &id);
        // 1.00002803 oz × 2000 × 1.05 = 2100.0589
        assert_eq!(minor, Some(210_006));
        let basis: String = v
            .conn()
            .query_row("SELECT basis FROM valuations WHERE asset_id = ?1", [&id], |r| r.get(0))
            .unwrap();
        assert_eq!(basis, "estimated_resale");
    }

    #[test]
    fn revaluing_twice_writes_once() {
        let (_d, v) = setup();
        let id = eagles(&v, 2, "");
        gold_spot(&v, "2000", SpotOrigin::Api, "2026-09-19T09:00:00Z");

        assert_eq!(revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap(), Outcome::Updated);
        assert_eq!(revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap(), Outcome::Unchanged);
        let n: i64 = v
            .conn()
            .query_row("SELECT count(*) FROM valuations WHERE asset_id = ?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1, "an unchanged quote must not pad the history");
    }

    #[test]
    fn a_hand_typed_spot_price_gives_a_manual_valuation() {
        let (_d, v) = setup();
        let id = eagles(&v, 1, "");
        gold_spot(&v, "1999.99", SpotOrigin::Manual, NOW);
        revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap();
        assert_eq!(current(&v, &id).1.as_deref(), Some("manual"));
    }

    #[test]
    fn manually_priced_assets_are_never_touched() {
        // The precedence rule: a refresh cannot overwrite a hand valuation.
        let (_d, v) = setup();
        let id = eagles(&v, 1, "");
        assets::set_pricing(&v, &id, Pricing::Manual, NOW).unwrap();
        gold_spot(&v, "2000", SpotOrigin::Api, "2026-09-19T09:00:00Z");

        assert_eq!(revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap(), Outcome::Skipped);
        assert_eq!(current(&v, &id).0, None);
    }

    #[test]
    fn a_missing_quote_is_reported_not_zeroed() {
        let (_d, v) = setup();
        let id = eagles(&v, 1, "");
        let summary = revalue_all(&v, NOW, &NOW[..10]).unwrap();
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.unpriced.len(), 1);
        assert!(summary.unpriced[0].1.contains("Gold"), "{:?}", summary.unpriced);
        assert_eq!(current(&v, &id).0, None, "unknown, never zero");
    }

    #[test]
    fn a_quantity_change_is_picked_up_on_the_next_revalue() {
        use crate::events::{record, EventType, NewEvent};
        let (_d, v) = setup();
        let id = eagles(&v, 4, "");
        gold_spot(&v, "2000", SpotOrigin::Api, "2026-09-19T09:00:00Z");
        revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap();

        record(
            &v,
            &NewEvent {
                asset_id: id.clone(),
                event_type: EventType::Add,
                effective_date: "2026-09-19".into(),
                quantity_delta: Decimal::from(4),
                amount_minor: None,
                currency: None,
                note: String::new(),
            },
            NOW,
        )
        .unwrap();
        assert_eq!(revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap(), Outcome::Updated);
        assert_eq!(current(&v, &id).0, Some(1_600_045), "8 × 1.00002803 oz × $2000");
    }

    #[test]
    fn crypto_holdings_value_at_full_precision() {
        let (_d, v) = setup();
        let id = assets::create(
            &v,
            &NewAsset {
                type_id: "crypto".into(),
                name: "Shiba".into(),
                quantity: Decimal::from(100_000_000),
                quantity_unit: "coin".into(),
                acquired_date: None,
                effective_date: None,
                acquired_cost: None,
                acquired_from: None,
                storage_location: None,
                notes: String::new(),
                insured: None,
                attrs: attrs(&[("coin_id", "shiba-inu"), ("symbol", "SHIB")]),
                pricing: Pricing::Market,
                review_every_days: None,
            },
            NOW,
        )
        .unwrap();
        record_quote(
            &v,
            &NewQuote {
                instrument_id: "coingecko:shiba-inu".into(),
                unit_quote: Decimal::from_str("0.00001234").unwrap(),
                currency: usd(),
                quote_unit: "coin".into(),
                source: "coingecko".into(),
                match_quality: MatchQuality::Exact,
                source_asof: NOW.into(),
            },
            NOW,
        )
        .unwrap();

        revalue_asset(&v, &id, NOW, &NOW[..10]).unwrap();
        assert_eq!(current(&v, &id).0, Some(123_400), "$1,234.00 — not $0 from early rounding");
        assert_eq!(held_coin_ids(&v).unwrap(), vec!["shiba-inu".to_string()]);
    }

    #[test]
    fn bad_metal_attributes_are_explained() {
        let cases = [
            (vec![("metal", "XAU"), ("weight_per_item", "")], "weight"),
            (vec![("metal", "XAU"), ("weight_per_item", "1"), ("purity", "999")], "purity"),
            (vec![("metal", "unobtainium")], "metal"),
            (
                vec![("metal", "XAG"), ("weight_per_item", "1"), ("weight_unit", "stone")],
                "unit",
            ),
        ];
        for (pairs, field) in cases {
            let err = MarketSpec::from_attrs(&attrs(&pairs)).unwrap_err();
            assert!(err.to_string().contains(field), "{err} should mention {field}");
        }
        assert_eq!(MarketSpec::from_attrs(&attrs(&[("title", "X-Men")])).unwrap(), None);
    }
}
