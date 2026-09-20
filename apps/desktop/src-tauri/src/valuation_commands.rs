//! IPC for manual valuation.
//!
//! Per decision 4 there are no paid price feeds, so **manual entry is the
//! primary path** for most collectibles, not a fallback. It has to be
//! pleasant: single edits, bulk edits, and a portfolio view that is honest
//! about what it could not price.
//!
//! Decimals and money cross this boundary as **strings**. JavaScript numbers
//! are 53-bit, so a large minor-unit amount or an 18-decimal crypto quantity
//! would corrupt silently.

use am_core::{parse_decimal, Currency, Money};
use am_storage::events::{self, EventType, NewEvent};
use am_storage::valuations::{self, Basis, NewValuation, Provenance};
use am_storage::vault::VaultError;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::session::{IpcError, Session, SessionError};

type IpcResult<T> = Result<T, IpcError>;

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn bad_input(message: impl Into<String>) -> IpcError {
    IpcError { kind: "invalid_input".into(), message: message.into() }
}

fn storage(e: impl std::fmt::Display) -> SessionError {
    SessionError::Vault(VaultError::Other(e.to_string()))
}

/// A price as typed by a person: major units, e.g. "1299.50".
///
/// Parsed as exact decimal and converted to minor units **once**, at the end.
/// Parsing to f64 first would lose precision on ordinary amounts.
fn parse_money(amount: &str, currency: &str) -> Result<Money, IpcError> {
    let currency = Currency::new(currency)
        .map_err(|e| bad_input(format!("currency: {e}")))?;
    let decimal = parse_decimal(amount)
        .map_err(|_| bad_input(format!("{amount:?} is not a valid amount")))?;
    Money::from_total_decimal(decimal, currency)
        .map_err(|e| bad_input(format!("amount: {e}")))
}

#[derive(Deserialize)]
pub struct PriceEntry {
    pub asset_id: String,
    /// Major units as text, e.g. "1299.50".
    pub amount: String,
    pub currency: String,
    /// ISO date this price applies to. Defaults to today when omitted.
    pub asof: Option<String>,
    pub basis: Option<String>,
    pub note: Option<String>,
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
#[tauri::command]
pub fn set_prices(
    session: State<'_, Session>,
    entries: Vec<PriceEntry>,
) -> IpcResult<Vec<PriceResult>> {
    session.touch();
    let timestamp = now();
    let today = timestamp[..10].to_string();

    session
        .with_vault(|vault| {
            let mut results = Vec::with_capacity(entries.len());

            for entry in &entries {
                let outcome = (|| -> Result<(), String> {
                    let money = parse_money(&entry.amount, &entry.currency)
                        .map_err(|e| e.message)?;

                    // The quantity held on the valuation date — not today's.
                    let asof = entry.asof.clone().unwrap_or_else(|| today.clone());
                    let quantity =
                        events::quantity_as_of(vault, &entry.asset_id, Some(&asof))
                            .map_err(|e| e.to_string())?;

                    valuations::record_valuation(
                        vault,
                        &NewValuation {
                            asset_id: entry.asset_id.clone(),
                            quote_id: None,
                            value: money,
                            quantity_at_time: quantity,
                            basis: basis_from(entry.basis.as_deref()),
                            provenance: Provenance::Manual,
                            inputs: serde_json::json!({
                                "note": entry.note.clone().unwrap_or_default(),
                            }),
                            asof,
                        },
                        &timestamp,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(())
                })();

                results.push(match outcome {
                    Ok(()) => PriceResult {
                        asset_id: entry.asset_id.clone(),
                        ok: true,
                        error: None,
                    },
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
    let code = currency.unwrap_or_else(|| "USD".into());
    let currency = Currency::new(&code).map_err(|e| bad_input(e.to_string()))?;
    let asof = asof.unwrap_or_else(|| now()[..10].to_string());

    session
        .with_vault(|vault| {
            let total = valuations::portfolio_total_as_of(vault, &asof, &currency)
                .map_err(storage)?;
            Ok(PortfolioView {
                total: total.total.format(),
                currency: code.clone(),
                valued: total.valued,
                unvalued: total.unvalued,
                skipped_currencies: total.skipped_currencies,
                asof,
            })
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct ValuationPoint {
    pub asof: String,
    pub amount: String,
    pub currency: String,
    pub basis: String,
    pub provenance: String,
    pub quantity_at_time: String,
}

#[tauri::command]
pub fn valuation_history(
    session: State<'_, Session>,
    asset_id: String,
) -> IpcResult<Vec<ValuationPoint>> {
    session.touch();
    session
        .with_vault(|vault| {
            let mut stmt = vault
                .conn()
                .prepare(
                    "SELECT asof, amount_minor, currency, basis, provenance, quantity_at_time
                     FROM valuations WHERE asset_id = ?1 ORDER BY asof",
                )
                .map_err(storage)?;

            let rows = stmt
                .query_map([&asset_id], |r| {
                    let minor: i64 = r.get(1)?;
                    let code: String = r.get(2)?;
                    // Format via Money so display never goes through a float.
                    let formatted = Currency::new(&code)
                        .map(|c| Money::new(minor, c).format())
                        .unwrap_or_else(|_| minor.to_string());
                    Ok(ValuationPoint {
                        asof: r.get(0)?,
                        amount: formatted,
                        currency: code,
                        basis: r.get(3)?,
                        provenance: r.get(4)?,
                        quantity_at_time: r.get(5)?,
                    })
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            Ok(rows)
        })
        .map_err(IpcError::from)
}

#[derive(Deserialize)]
pub struct QuantityChange {
    pub asset_id: String,
    /// "add", "remove", "dispose", or "correct".
    pub kind: String,
    /// Magnitude as text; the sign comes from `kind`.
    pub quantity: String,
    pub effective_date: Option<String>,
    pub note: Option<String>,
}

/// Record a quantity change against the event log.
#[tauri::command]
pub fn change_quantity(
    session: State<'_, Session>,
    change: QuantityChange,
) -> IpcResult<String> {
    session.touch();
    let timestamp = now();

    let magnitude = parse_decimal(&change.quantity)
        .map_err(|_| bad_input(format!("{:?} is not a valid quantity", change.quantity)))?;
    if magnitude.is_sign_negative() {
        return Err(bad_input("quantity must be positive; the direction comes from the action"));
    }

    let (event_type, delta) = match change.kind.as_str() {
        "add" => (EventType::Add, magnitude),
        "remove" => (EventType::Remove, -magnitude),
        "dispose" => (EventType::Dispose, -magnitude),
        "correct" => (EventType::Correct, magnitude),
        other => return Err(bad_input(format!("unknown change type: {other}"))),
    };

    let effective_date =
        change.effective_date.unwrap_or_else(|| timestamp[..10].to_string());

    session
        .with_vault(|vault| {
            events::record(
                vault,
                &NewEvent {
                    asset_id: change.asset_id.clone(),
                    event_type,
                    effective_date,
                    quantity_delta: delta,
                    amount_minor: None,
                    currency: None,
                    note: change.note.clone().unwrap_or_default(),
                },
                &timestamp,
            )
            .map_err(storage)
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct QuantityAt {
    pub quantity: String,
    pub asof: String,
}

/// Quantity held on a date, replayed from the event log.
#[tauri::command]
pub fn quantity_on(
    session: State<'_, Session>,
    asset_id: String,
    asof: String,
) -> IpcResult<QuantityAt> {
    session.touch();
    session
        .with_vault(|vault| {
            let quantity = events::quantity_as_of(vault, &asset_id, Some(&asof))
                .map_err(storage)?;
            Ok(QuantityAt { quantity: quantity.to_string(), asof: asof.clone() })
        })
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_is_parsed_exactly_not_through_a_float() {
        let money = parse_money("1299.50", "USD").unwrap();
        assert_eq!(money.amount_minor, 129_950);

        // A value f64 cannot represent exactly.
        let awkward = parse_money("0.07", "USD").unwrap();
        assert_eq!(awkward.amount_minor, 7);
    }

    #[test]
    fn zero_decimal_currencies_are_handled() {
        let yen = parse_money("1000", "JPY").unwrap();
        assert_eq!(yen.amount_minor, 1000, "JPY has no minor units");
    }

    #[test]
    fn bad_amounts_and_currencies_are_rejected() {
        assert!(parse_money("not a number", "USD").is_err());
        assert!(parse_money("10.00", "DOLLARS").is_err());
        assert!(parse_money("", "USD").is_err());
    }

    #[test]
    fn basis_defaults_to_estimated_resale() {
        assert_eq!(basis_from(None), Basis::EstimatedResale);
        assert_eq!(basis_from(Some("insured")), Basis::Insured);
        assert_eq!(basis_from(Some("nonsense")), Basis::EstimatedResale);
    }
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
    let code = currency.unwrap_or_else(|| "USD".into());
    let currency = Currency::new(&code).map_err(|e| bad_input(e.to_string()))?;
    let today = now()[..10].to_string();

    session
        .with_vault(|vault| {
            // Default range starts at the first event, so a period when
            // assets were held but unpriced shows as low coverage rather than
            // being hidden.
            let start = match from.clone() {
                Some(value) => value,
                None => am_storage::series::earliest_activity(vault)
                    .map_err(storage)?
                    .unwrap_or_else(|| today.clone()),
            };
            let end = to.clone().unwrap_or_else(|| today.clone());

            let series = am_storage::series::portfolio_series(
                vault,
                am_storage::series::SeriesRequest {
                    from: &start,
                    to: &end,
                    max_points: max_points.unwrap_or(180),
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
