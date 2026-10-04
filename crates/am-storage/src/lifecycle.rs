//! Lost, retired and recovered: dated status changes.
//!
//! An asset's status on a date is the latest status event on or before it —
//! active if there is none. Historical totals ask this, not today's status,
//! so marking something lost today leaves every earlier total as it was.
//!
//! Sold is not here. It is what the event log says when nothing is left (see
//! `events`), and setting it by hand would make status and quantity disagree.

use am_core::Decimal;

use crate::events::{normalize_date, EventError};
use crate::vault::Vault;

/// A status the owner can set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// Held — including recovered after being lost.
    Active,
    Lost,
    /// Kept in the catalog for its history but no longer counted: given
    /// away, destroyed, used up.
    Retired,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Lifecycle::Active => "active",
            Lifecycle::Lost => "lost",
            Lifecycle::Retired => "retired",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "active" => Lifecycle::Active,
            "lost" => Lifecycle::Lost,
            "retired" => Lifecycle::Retired,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusEvent {
    pub event_id: String,
    pub status: String,
    pub effective_date: String,
    pub note: String,
    pub recorded_at: String,
}

/// Record a status change, effective on `date`, and refresh the cached
/// status. A change to the status already in effect on that date is a no-op.
pub fn set_status_in(
    conn: &rusqlite::Connection,
    asset_id: &str,
    status: Lifecycle,
    date: &str,
    note: &str,
    now: &str,
) -> Result<bool, EventError> {
    let date = normalize_date(date)?;
    if status_as_of_in(conn, asset_id, &date)? == status {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO status_events (event_id, asset_id, status, effective_date, note, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![new_id(), asset_id, status.as_str(), &date, note, now],
    )?;
    refresh_status_in(conn, asset_id)?;
    Ok(true)
}

pub fn set_status(
    vault: &Vault,
    asset_id: &str,
    status: Lifecycle,
    date: &str,
    note: &str,
    now: &str,
) -> Result<bool, EventError> {
    let unit = crate::atomic::begin(vault.conn())?;
    let changed = set_status_in(&unit, asset_id, status, date, note, now)?;
    unit.commit()?;
    Ok(changed)
}

/// The owner-set status in effect on a date.
pub fn status_as_of_in(
    conn: &rusqlite::Connection,
    asset_id: &str,
    date: &str,
) -> Result<Lifecycle, EventError> {
    let latest: Option<String> = conn
        .query_row(
            "SELECT status FROM status_events WHERE asset_id = ?1 AND effective_date <= ?2
             ORDER BY effective_date DESC, recorded_at DESC LIMIT 1",
            rusqlite::params![asset_id, date],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(latest.as_deref().and_then(Lifecycle::parse).unwrap_or(Lifecycle::Active))
}

/// Whether an asset counts toward a total on a date: held, and not then lost
/// or retired.
pub fn counts_on(vault: &Vault, asset_id: &str, date: &str) -> Result<bool, EventError> {
    Ok(status_as_of_in(vault.conn(), asset_id, date)? == Lifecycle::Active)
}

/// Set the cached `assets.status` from the history: the latest owner-set
/// status, except that a holding with nothing left is sold. A lost or
/// retired item stays lost or retired even if its count is zero.
pub(crate) fn refresh_status_in(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<(), EventError> {
    let latest = status_as_of_in(conn, asset_id, "9999-12-31")?;
    let quantity: String =
        conn.query_row("SELECT quantity FROM assets WHERE asset_id = ?1", [asset_id], |r| {
            r.get(0)
        })?;
    let held = am_core::parse_decimal(&quantity).unwrap_or(Decimal::ZERO);
    let status = match latest {
        Lifecycle::Lost | Lifecycle::Retired => latest.as_str(),
        Lifecycle::Active if held == Decimal::ZERO => "sold",
        Lifecycle::Active => "active",
    };
    conn.execute(
        "UPDATE assets SET status = ?1 WHERE asset_id = ?2",
        rusqlite::params![status, asset_id],
    )?;
    Ok(())
}

/// Every status change, oldest first.
pub fn history(vault: &Vault, asset_id: &str) -> Result<Vec<StatusEvent>, EventError> {
    let mut stmt = vault.conn().prepare(
        "SELECT event_id, status, effective_date, note, recorded_at FROM status_events
         WHERE asset_id = ?1 ORDER BY effective_date, recorded_at",
    )?;
    let rows = stmt
        .query_map([asset_id], |r| {
            Ok(StatusEvent {
                event_id: r.get(0)?,
                status: r.get(1)?,
                effective_date: r.get(2)?,
                note: r.get(3)?,
                recorded_at: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn new_id() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{self, NewAsset, Pricing};
    use crate::valuations::{
        portfolio_total_as_of, record_valuation, Basis, NewValuation, Provenance,
    };
    use am_core::{Currency, Money};
    use am_crypto::KdfParams;
    use std::collections::BTreeMap;

    const NOW: &str = "2026-10-10T00:00:00Z";

    fn usd() -> Currency {
        Currency::new("USD").unwrap()
    }

    /// A vault with one watch, bought in January and valued at $5,000.
    fn setup() -> (tempfile::TempDir, Vault, String) {
        let dir = tempfile::tempdir().unwrap();
        let fast = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
        let (v, _r) =
            Vault::create(&dir.path().join("v"), "correct horse battery staple", &fast, NOW)
                .unwrap();
        let id = assets::create(
            &v,
            &NewAsset {
                type_id: "watch".into(),
                name: "Watch".into(),
                quantity: Decimal::ONE,
                quantity_unit: "item".into(),
                acquired_date: Some("2026-01-15".into()),
                effective_date: None,
                acquired_cost: None,
                acquired_from: None,
                storage_location: None,
                notes: String::new(),
                insured: None,
                attrs: BTreeMap::new(),
                pricing: Pricing::Manual,
                review_every_days: None,
            },
            NOW,
        )
        .unwrap();
        record_valuation(
            &v,
            &NewValuation {
                asset_id: id.clone(),
                quote_id: None,
                value: Money::new(500_000, usd()),
                quantity_at_time: Decimal::ONE,
                basis: Basis::EstimatedResale,
                provenance: Provenance::Appraisal,
                inputs: serde_json::json!({}),
                asof: "2026-02-01".into(),
            },
            NOW,
        )
        .unwrap();
        (dir, v, id)
    }

    fn total_on(v: &Vault, date: &str) -> i64 {
        portfolio_total_as_of(v, date, &usd()).unwrap().total.amount_minor
    }

    fn status(v: &Vault, id: &str) -> String {
        assets::get(v, id).unwrap().status
    }

    #[test]
    fn a_loss_keeps_the_value_held_before_it() {
        let (_d, v, id) = setup();
        set_status(&v, &id, Lifecycle::Lost, "2026-10-01", "burglary", NOW).unwrap();

        assert_eq!(total_on(&v, "2026-03-01"), 500_000, "March is untouched");
        assert_eq!(total_on(&v, "2026-09-30"), 500_000);
        assert_eq!(total_on(&v, "2026-10-01"), 0, "gone from the day it was lost");
        assert_eq!(status(&v, &id), "lost");
    }

    #[test]
    fn a_recovery_counts_again_from_the_day_it_came_back() {
        let (_d, v, id) = setup();
        set_status(&v, &id, Lifecycle::Lost, "2026-05-01", "", NOW).unwrap();
        set_status(&v, &id, Lifecycle::Active, "2026-08-01", "found", NOW).unwrap();

        assert_eq!(total_on(&v, "2026-04-30"), 500_000);
        assert_eq!(total_on(&v, "2026-06-01"), 0);
        assert_eq!(total_on(&v, "2026-08-01"), 500_000);
        assert_eq!(status(&v, &id), "active");
        assert_eq!(history(&v, &id).unwrap().len(), 2);
    }

    #[test]
    fn setting_the_status_already_in_effect_records_nothing() {
        let (_d, v, id) = setup();
        assert!(!set_status(&v, &id, Lifecycle::Active, "2026-03-01", "", NOW).unwrap());
        assert!(set_status(&v, &id, Lifecycle::Retired, "2026-03-01", "", NOW).unwrap());
        assert!(!set_status(&v, &id, Lifecycle::Retired, "2026-04-01", "", NOW).unwrap());
        assert_eq!(history(&v, &id).unwrap().len(), 1);
    }

    #[test]
    fn the_edit_form_dates_a_status_change() {
        let (_d, v, id) = setup();
        let r = assets::get(&v, &id).unwrap();
        let edit = assets::AssetEdit {
            type_id: r.type_id.clone(),
            name: r.name.clone(),
            status: "lost".into(),
            quantity_unit: r.quantity_unit.clone(),
            acquired_date: r.acquired_date.clone(),
            acquired_cost: None,
            acquired_from: None,
            storage_location: None,
            notes: String::new(),
            insured: None,
            attrs: BTreeMap::new(),
            review_every_days: None,
            cost_covers_holding: false,
            status_date: Some("2026-09-15".into()),
        };
        assets::update(&v, &id, &edit, NOW).unwrap();
        assert_eq!(status(&v, &id), "lost");
        assert_eq!(history(&v, &id).unwrap()[0].effective_date, "2026-09-15");
        assert_eq!(total_on(&v, "2026-09-14"), 500_000);
    }
}
