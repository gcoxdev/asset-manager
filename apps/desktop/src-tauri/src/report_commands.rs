//! Insurance report.
//!
//! The one output here that exists for someone else to read: an insurer
//! handling a claim. That shapes the design — it needs provenance for each
//! figure, photographs, and a clear statement of what the numbers are and are
//! not.
//!
//! # Plaintext by nature
//!
//! A report is meant to be sent, so it cannot be encrypted and still be
//! useful. That makes it the single easiest way to leak an entire catalogue,
//! and the caller warns before writing one.

use am_core::{Currency, Money};
use am_storage::vault::VaultError;
use serde::Serialize;
use tauri::State;

use crate::session::{IpcError, Session, SessionError};

type IpcResult<T> = Result<T, IpcError>;

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn storage(e: impl std::fmt::Display) -> SessionError {
    SessionError::Vault(VaultError::Other(e.to_string()))
}

#[derive(Serialize)]
pub struct ReportItem {
    pub name: String,
    pub type_id: String,
    pub quantity: String,
    pub storage_location: Option<String>,
    pub acquired_date: Option<String>,
    pub acquired: Option<String>,
    pub current: Option<String>,
    /// Where the current figure came from, so a valuation is not presented as
    /// authoritative when someone typed it in.
    pub value_source: Option<String>,
    pub value_asof: Option<String>,
    /// Object IDs, resolved to embedded images by the caller.
    pub photo_ids: Vec<String>,
}

#[derive(Serialize)]
pub struct InsuranceReport {
    pub generated_at: String,
    pub items: Vec<ReportItem>,
    pub total: String,
    pub valued: usize,
    /// Items with no valuation. Reported rather than omitted: an insurer
    /// should see that the total is partial.
    pub unvalued: usize,
    pub currency: String,
    pub warning: String,
}

fn format_money(minor: Option<i64>, code: Option<String>) -> Option<String> {
    let (amount, code) = (minor?, code?);
    Currency::new(&code).ok().map(|c| Money::new(amount, c).format())
}

/// Build an insurance report over active holdings.
#[tauri::command]
pub fn insurance_report(
    session: State<'_, Session>,
    currency: Option<String>,
) -> IpcResult<InsuranceReport> {
    session.touch();
    let code = currency.unwrap_or_else(|| "USD".into());
    let currency = Currency::new(&code)
        .map_err(|e| IpcError { kind: "invalid_input".into(), message: e.to_string() })?;
    let generated_at = now();

    session
        .with_vault(|vault| {
            // Sold and lost items are excluded: a claim covers what is held.
            let mut stmt = vault
                .conn()
                .prepare(
                    "SELECT asset_id, name, type_id, quantity, storage_location,
                            acquired_date, acquired_amount_minor, acquired_currency,
                            current_amount_minor, current_currency, value_source, value_asof
                     FROM assets
                     WHERE status = 'active'
                     ORDER BY name",
                )
                .map_err(storage)?;

            let rows: Vec<(String, ReportItem, Option<i64>, Option<String>)> = stmt
                .query_map([], |r| {
                    let asset_id: String = r.get(0)?;
                    let current_minor: Option<i64> = r.get(8)?;
                    let current_code: Option<String> = r.get(9)?;
                    Ok((
                        asset_id,
                        ReportItem {
                            name: r.get(1)?,
                            type_id: r.get(2)?,
                            quantity: r.get(3)?,
                            storage_location: r.get(4)?,
                            acquired_date: r.get(5)?,
                            acquired: format_money(r.get(6)?, r.get(7)?),
                            current: format_money(current_minor, current_code.clone()),
                            value_source: r.get(10)?,
                            value_asof: r.get(11)?,
                            photo_ids: Vec::new(),
                        },
                        current_minor,
                        current_code,
                    ))
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            drop(stmt);

            let mut items = Vec::new();
            let mut total = Money::zero(currency.clone());
            let mut valued = 0usize;
            let mut unvalued = 0usize;

            for (asset_id, mut item, minor, item_code) in rows {
                let mut photos = vault
                    .conn()
                    .prepare(
                        "SELECT m.object_id FROM asset_media m
                         JOIN objects o ON o.object_id = m.object_id
                         WHERE m.asset_id = ?1 AND o.gc_state = 'live'
                         ORDER BY m.is_primary DESC, m.sort_order",
                    )
                    .map_err(storage)?;
                item.photo_ids = photos
                    .query_map([&asset_id], |r| r.get(0))
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?;
                drop(photos);

                match (minor, item_code) {
                    // Only same-currency items contribute; there is no FX
                    // layer, and a silently converted total would be wrong in
                    // a way an insurer could not see.
                    (Some(amount), Some(item_code)) if item_code == code => {
                        total = total
                            .checked_add(&Money::new(amount, currency.clone()))
                            .map_err(|e| storage(e.to_string()))?;
                        valued += 1;
                    }
                    _ => unvalued += 1,
                }
                items.push(item);
            }

            Ok(InsuranceReport {
                generated_at: generated_at.clone(),
                items,
                total: total.format(),
                valued,
                unvalued,
                currency: code.clone(),
                warning: "This report is not encrypted. It lists what you own, \
                          what it is worth and where it is kept — treat the file \
                          as you would the items themselves."
                    .to_string(),
            })
        })
        .map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_formatting_respects_the_currency() {
        assert_eq!(
            format_money(Some(129_950), Some("USD".into())).as_deref(),
            Some("1299.50 USD")
        );
        // JPY has no minor units; 1000 is a thousand yen, not ten.
        assert_eq!(format_money(Some(1000), Some("JPY".into())).as_deref(), Some("1000 JPY"));
    }

    #[test]
    fn a_missing_amount_or_currency_yields_nothing() {
        // Never a zero, which would read as "worthless" rather than "unpriced".
        assert_eq!(format_money(None, Some("USD".into())), None);
        assert_eq!(format_money(Some(100), None), None);
        assert_eq!(format_money(Some(100), Some("NOTACURRENCY".into())), None);
    }
}
