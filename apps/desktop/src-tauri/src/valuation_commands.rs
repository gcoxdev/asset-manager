//! IPC for valuation: prices, quantity changes, totals and charts.
//!
//! Per decision 4 there are no paid price feeds, so **manual entry is the
//! primary path** for most collectibles, not a fallback. It has to be
//! pleasant: single edits, bulk edits, and a portfolio view that is honest
//! about what it could not price.

use am_core::{parse_decimal, Currency, Decimal, Money};
use am_storage::assets::{self, Pricing};
use am_storage::events::{self, EventType, NewEvent};
use am_storage::valuations::{self, Basis, NewValuation, Provenance};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::ipc::{
    atomically, bad_input, base_currency, currency_or, now, parse_money, parse_money_opt,
    storage, today, IpcResult,
};
use crate::session::{IpcError, Session};

#[derive(Deserialize)]
pub struct PriceEntry {
    pub asset_id: String,
    /// Major units as text, e.g. "1299.50".
    pub amount: String,
    /// Defaults to the vault's base currency.
    #[serde(default)]
    pub currency: Option<String>,
    /// ISO date this price applies to. Defaults to today.
    #[serde(default)]
    pub asof: Option<String>,
    #[serde(default)]
    pub basis: Option<String>,
    #[serde(default)]
    pub provenance: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Why it is worth this: comparables, a range, a confidence, the
    /// appraisal. Kept with the valuation so the history explains itself.
    #[serde(default)]
    pub evidence: Option<Evidence>,
}

#[derive(Deserialize, Default)]
pub struct Evidence {
    #[serde(default)]
    pub comparables: Vec<Comparable>,
    #[serde(default)]
    pub low: Option<String>,
    #[serde(default)]
    pub high: Option<String>,
    /// "low", "medium" or "high".
    #[serde(default)]
    pub confidence: Option<String>,
    /// An attachment of this asset — the appraisal, a printed listing.
    #[serde(default)]
    pub document: Option<String>,
}

#[derive(Deserialize)]
pub struct Comparable {
    pub description: String,
    /// Major units as written.
    pub price: String,
    /// "sold", "asking" or "auction".
    pub kind: String,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

/// Check evidence and turn it into what is stored in the valuation's inputs:
/// amounts in minor units with the valuation's currency, dates normalized.
fn evidence_json(
    vault: &am_storage::vault::Vault,
    asset_id: &str,
    evidence: &Evidence,
    value: &Money,
    today: &str,
) -> Result<serde_json::Value, String> {
    let currency = value.currency.clone();
    let money = |text: &str| {
        parse_money(text, &currency).map(|m| m.amount_minor).map_err(|e| e.message)
    };
    if evidence.comparables.len() > 20 {
        return Err("up to 20 comparables".into());
    }
    let mut comparables = Vec::new();
    for c in &evidence.comparables {
        let description = c.description.trim();
        if description.is_empty() || description.chars().count() > 300 {
            return Err("each comparable needs a short description".into());
        }
        if !["sold", "asking", "auction"].contains(&c.kind.as_str()) {
            return Err(format!("unknown kind of comparable: {}", c.kind));
        }
        let date = match c.date.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(d) => {
                let d = events::normalize_date(d).map_err(|e| e.to_string())?;
                if d.as_str() > today {
                    return Err("a comparable cannot be dated in the future".into());
                }
                Some(d)
            }
        };
        comparables.push(serde_json::json!({
            "description": description,
            "price_minor": money(&c.price)?,
            "kind": c.kind,
            "date": date,
            "source": c.source.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        }));
    }
    let low =
        evidence.low.as_deref().filter(|s| !s.trim().is_empty()).map(money).transpose()?;
    let high =
        evidence.high.as_deref().filter(|s| !s.trim().is_empty()).map(money).transpose()?;
    if let (Some(l), Some(h)) = (low, high) {
        if l > h {
            return Err("the low end of the range is above the high end".into());
        }
    }
    if low.is_some_and(|l| l > value.amount_minor)
        || high.is_some_and(|h| h < value.amount_minor)
    {
        return Err("the value is outside the range given for it".into());
    }
    let confidence = evidence.confidence.as_deref().filter(|c| !c.is_empty());
    if confidence.is_some_and(|c| !["low", "medium", "high"].contains(&c)) {
        return Err("confidence is low, medium or high".into());
    }
    if let Some(object_id) = &evidence.document {
        let attached: i64 = vault
            .conn()
            .query_row(
                "SELECT count(*) FROM asset_media WHERE asset_id = ?1 AND object_id = ?2",
                [asset_id, object_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if attached == 0 {
            return Err("that document is not attached to this asset".into());
        }
    }
    Ok(serde_json::json!({
        "comparables": comparables,
        "low_minor": low,
        "high_minor": high,
        "confidence": confidence,
        "document": evidence.document,
    }))
}

#[derive(Serialize)]
pub struct PriceResult {
    pub asset_id: String,
    pub ok: bool,
    pub error: Option<String>,
}

fn basis_from(name: Option<&str>) -> Basis {
    match name {
        Some("melt") => Basis::Melt,
        Some("replacement") => Basis::Replacement,
        Some("insured") => Basis::Insured,
        _ => Basis::EstimatedResale,
    }
}

/// Set the price for several assets at once.
///
/// Bulk edit is the point: setting prices one dialog at a time does not scale
/// to a few hundred collectibles. Each row succeeds or fails independently and
/// reports back, so one bad cell does not discard the rest of the work.
///
/// A hand-entered price switches the asset to manual pricing. That is the
/// precedence rule: a later market refresh must never overwrite a value the
/// owner typed in.
#[tauri::command]
pub fn set_prices(
    session: State<'_, Session>,
    entries: Vec<PriceEntry>,
) -> IpcResult<Vec<PriceResult>> {
    session.touch();
    let timestamp = now();
    let today = today();

    session
        .with_vault(|vault| {
            let base = base_currency(vault);
            let mut results = Vec::with_capacity(entries.len());

            for entry in &entries {
                // Each entry is its own unit: the valuation and the switch to
                // manual pricing land together, and a bad cell undoes only
                // its own row.
                let unit = am_storage::atomic::begin(vault.conn()).map_err(storage)?;
                let outcome = (|| -> Result<(), String> {
                    let currency =
                        currency_or(&entry.currency, &base).map_err(|e| e.message)?;
                    let money = parse_money(&entry.amount, &currency).map_err(|e| e.message)?;

                    let asof = match entry.asof.as_deref().map(str::trim) {
                        None | Some("") => today.clone(),
                        Some(d) => events::normalize_date(d).map_err(|e| e.to_string())?,
                    };
                    if asof > today {
                        return Err("a valuation cannot be dated in the future".into());
                    }
                    // The quantity held on the valuation date — not today's.
                    let quantity = events::quantity_as_of(vault, &entry.asset_id, Some(&asof))
                        .map_err(|e| e.to_string())?;
                    if quantity <= Decimal::ZERO {
                        return Err(format!("this asset was not held on {asof}"));
                    }

                    valuations::record_valuation(
                        vault,
                        &NewValuation {
                            asset_id: entry.asset_id.clone(),
                            quote_id: None,
                            value: money.clone(),
                            quantity_at_time: quantity,
                            basis: basis_from(entry.basis.as_deref()),
                            provenance: match entry.provenance.as_deref() {
                                Some("appraisal") => Provenance::Appraisal,
                                _ => Provenance::Manual,
                            },
                            inputs: {
                                let mut inputs = serde_json::json!({
                                    "note": entry.note.clone().unwrap_or_default(),
                                });
                                if let Some(evidence) = &entry.evidence {
                                    inputs["evidence"] = evidence_json(
                                        vault,
                                        &entry.asset_id,
                                        evidence,
                                        &money,
                                        &today,
                                    )?;
                                }
                                inputs
                            },
                            asof,
                        },
                        &timestamp,
                    )
                    .map_err(|e| e.to_string())?;
                    assets::set_pricing(vault, &entry.asset_id, Pricing::Manual, &timestamp)
                        .map_err(|e| e.to_string())?;
                    Ok(())
                })();

                let outcome = outcome.and_then(|()| unit.commit().map_err(|e| e.to_string()));
                results.push(match outcome {
                    Ok(()) => {
                        PriceResult { asset_id: entry.asset_id.clone(), ok: true, error: None }
                    }
                    Err(message) => PriceResult {
                        asset_id: entry.asset_id.clone(),
                        ok: false,
                        error: Some(message),
                    },
                });
            }

            Ok(results)
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct PortfolioView {
    /// Formatted for display, e.g. "35800.06 USD". Never a JS number.
    pub total: String,
    pub total_minor: String,
    pub currency: String,
    pub valued: usize,
    /// Held assets with no usable price. Shown beside the total so a partial
    /// figure is never mistaken for a complete one.
    pub unvalued: usize,
    /// Currencies present but not converted — there is no FX layer yet.
    pub skipped_currencies: Vec<String>,
    pub asof: String,
}

#[tauri::command]
pub fn portfolio_total(
    session: State<'_, Session>,
    currency: Option<String>,
    asof: Option<String>,
) -> IpcResult<PortfolioView> {
    session.touch();
    let asof = asof.unwrap_or_else(today);

    session
        .with_vault(|vault| {
            let currency = currency_or(&currency, &base_currency(vault))
                .map_err(|e| storage(e.message))?;
            let total =
                valuations::portfolio_total_as_of(vault, &asof, &currency).map_err(storage)?;
            Ok(PortfolioView {
                total: total.total.format(),
                total_minor: total.total.amount_minor.to_string(),
                currency: currency.code().to_string(),
                valued: total.valued,
                unvalued: total.unvalued,
                skipped_currencies: total.skipped_currencies,
                asof: asof.clone(),
            })
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn dashboard(session: State<'_, Session>) -> IpcResult<am_storage::summary::Dashboard> {
    session.touch();
    let today = today();
    session
        .with_vault(|vault| {
            am_storage::summary::dashboard(vault, &base_currency(vault), &today)
                .map_err(storage)
        })
        .map_err(IpcError::from)
}

#[derive(Deserialize)]
pub struct QuantityChange {
    pub asset_id: String,
    /// "add", "remove", "dispose", or "correct".
    pub kind: String,
    /// For add/remove, how many. For correct, the **right total** — what the
    /// holding should have said all along. Ignored for dispose, which always
    /// disposes of everything held.
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub effective_date: Option<String>,
    /// Price paid (add) or received (remove, dispose), in major units.
    #[serde(default)]
    pub amount: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// Turn a requested change into a signed event delta.
///
/// "Correct" takes the corrected total rather than a delta: a person fixing a
/// miscount knows the right number, not the difference, and asking for the
/// difference is how a correction gets applied in the wrong direction.
fn delta_for(
    kind: &str,
    typed: Option<&str>,
    held: Decimal,
) -> Result<(EventType, Decimal), IpcError> {
    let magnitude = || -> Result<Decimal, IpcError> {
        let text = typed
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| bad_input("enter a quantity"))?;
        let value =
            parse_decimal(text).map_err(|_| bad_input(format!("{text:?} is not a number")))?;
        if value.is_sign_negative() {
            return Err(bad_input("enter a positive number; the action gives the direction"));
        }
        Ok(value)
    };

    Ok(match kind {
        "add" => {
            let m = magnitude()?;
            if m.is_zero() {
                return Err(bad_input("adding nothing changes nothing"));
            }
            (EventType::Add, m)
        }
        "remove" => {
            let m = magnitude()?;
            if m.is_zero() {
                return Err(bad_input("removing nothing changes nothing"));
            }
            if m >= held {
                return Err(bad_input(
                    "that is the whole holding — record it as sold instead, so its \
                     history ends cleanly",
                ));
            }
            (EventType::Remove, -m)
        }
        "dispose" => {
            if held <= Decimal::ZERO {
                return Err(bad_input("nothing is held to dispose of"));
            }
            (EventType::Dispose, -held)
        }
        "correct" => {
            let target = magnitude()?;
            if target == held {
                return Err(bad_input("that is already the recorded quantity"));
            }
            (EventType::Correct, target - held)
        }
        other => return Err(bad_input(format!("unknown change type: {other}"))),
    })
}

/// Record a quantity change against the event log.
#[tauri::command]
pub fn change_quantity(
    session: State<'_, Session>,
    change: QuantityChange,
) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    let today = today();

    let effective_date = match change.effective_date.as_deref().map(str::trim) {
        None | Some("") => today.clone(),
        Some(d) => events::normalize_date(d).map_err(|e| bad_input(e.to_string()))?,
    };
    if effective_date > today {
        return Err(bad_input("a change cannot be dated in the future"));
    }

    session
        .with_vault(|vault| {
            atomically(vault, || {
                // What was held on the chosen date, not today: "sold all" in
                // March sells what was there in March, and "correct to 7" on a
                // past date means 7 then.
                let held =
                    events::quantity_as_of(vault, &change.asset_id, Some(&effective_date))
                        .map_err(storage)?;
                let (event_type, delta) =
                    delta_for(&change.kind, change.quantity.as_deref(), held)
                        .map_err(|e| storage(e.message))?;

                let currency = currency_or(&change.currency, &base_currency(vault))
                    .map_err(|e| storage(e.message))?;
                let amount = parse_money_opt(&change.amount, &currency)
                    .map_err(|e| storage(e.message))?;

                let event_id = events::record(
                    vault,
                    &NewEvent {
                        asset_id: change.asset_id.clone(),
                        event_type,
                        effective_date: effective_date.clone(),
                        quantity_delta: delta,
                        amount_minor: amount.as_ref().map(|m| m.amount_minor),
                        currency: amount.as_ref().map(|m| m.currency.code().to_string()),
                        note: change.note.clone().unwrap_or_default(),
                    },
                    &timestamp,
                )
                .map_err(storage)?;

                am_storage::pricing::revalue_asset(vault, &change.asset_id, &timestamp, &today)
                    .map_err(storage)?;
                Ok(event_id)
            })
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct ChartSeries {
    pub points: Vec<am_storage::series::SeriesPoint>,
    pub currency: String,
    pub skipped_currencies: Vec<String>,
    /// False when any point had an unpriced holding. The chart must show a
    /// caveat rather than presenting a partial total as complete.
    pub complete: bool,
}

/// Portfolio value over time.
///
/// Semantics are specified in `docs/chart-semantics.md` and asserted in
/// `am-storage::series`. In particular this is a **value** series, not a
/// return series: decision 2 excludes the per-flow data needed to separate
/// "bought more" from "gained value", so the caller must not label it as
/// performance.
#[tauri::command]
pub fn portfolio_series(
    session: State<'_, Session>,
    from: Option<String>,
    to: Option<String>,
    currency: Option<String>,
    max_points: Option<usize>,
) -> IpcResult<ChartSeries> {
    session.touch();
    let today = today();

    session
        .with_vault(|vault| {
            let currency: Currency = currency_or(&currency, &base_currency(vault))
                .map_err(|e| storage(e.message))?;
            // Default range starts at the first event, so a period when
            // assets were held but unpriced shows as low coverage rather than
            // being hidden.
            let earliest = am_storage::series::earliest_activity(vault)
                .map_err(storage)?
                .unwrap_or_else(|| today.clone());
            let start = match from.clone() {
                // Never before anything happened: leading zeros would flatten
                // the line into the axis.
                Some(value) if value > earliest => value,
                _ => earliest,
            };
            let end = to.clone().unwrap_or_else(|| today.clone());

            let series = am_storage::series::portfolio_series(
                vault,
                am_storage::series::SeriesRequest {
                    from: &start,
                    to: &end,
                    max_points: max_points.unwrap_or(180).clamp(2, 1000),
                },
                &currency,
            )
            .map_err(storage)?;

            Ok(ChartSeries {
                points: series.points,
                currency: series.currency,
                skipped_currencies: series.skipped_currencies,
                complete: series.complete,
            })
        })
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        parse_decimal(s).unwrap()
    }

    #[test]
    fn basis_defaults_to_estimated_resale() {
        assert_eq!(basis_from(None), Basis::EstimatedResale);
        assert_eq!(basis_from(Some("insured")), Basis::Insured);
        assert_eq!(basis_from(Some("nonsense")), Basis::EstimatedResale);
    }

    #[test]
    fn a_correction_takes_the_right_total_in_either_direction() {
        // The old behaviour could only ever add.
        let (kind, delta) = delta_for("correct", Some("7"), d("10")).unwrap();
        assert_eq!((kind, delta), (EventType::Correct, d("-3")));
        let (_, delta) = delta_for("correct", Some("12"), d("10")).unwrap();
        assert_eq!(delta, d("2"));
        assert!(delta_for("correct", Some("10"), d("10")).is_err(), "no-op");
    }

    #[test]
    fn a_disposal_takes_the_whole_holding_whatever_is_typed() {
        let (kind, delta) = delta_for("dispose", Some("1"), d("25")).unwrap();
        assert_eq!((kind, delta), (EventType::Dispose, d("-25")));
        assert!(delta_for("dispose", None, Decimal::ZERO).is_err());
    }

    #[test]
    fn removing_everything_is_steered_to_a_sale() {
        assert!(delta_for("remove", Some("5"), d("5")).is_err());
        assert_eq!(delta_for("remove", Some("2"), d("5")).unwrap().1, d("-2"));
    }

    #[test]
    fn nonsense_quantities_are_refused() {
        assert!(delta_for("add", Some("-3"), d("5")).is_err());
        assert!(delta_for("add", Some("lots"), d("5")).is_err());
        assert!(delta_for("add", Some("0"), d("5")).is_err());
        assert!(delta_for("add", None, d("5")).is_err());
        assert!(delta_for("teleport", Some("1"), d("5")).is_err());
    }
}
