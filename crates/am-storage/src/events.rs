//! Effective-dated ownership changes.
//!
//! `assets.quantity` is a **derived cache** of this event log, never an
//! independently edited value. That is what makes historical charts honest:
//!
//! > A `price_history` table plus *today's* quantity cannot reconstruct
//! > historical value. If a holding goes from 10 units to 5, applying 5 to
//! > every past quote silently rewrites the past.
//!
//! So the question "how much did I hold on 2026-03-01?" is answered by
//! replaying events up to that date, not by reading the current row.
//!
//! This is **not** tax-lot tracking. No FIFO, no LIFO, no realized gains —
//! decision 2 stands. It is the minimum needed for a chart that does not lie.

use am_core::{parse_decimal, sort_key, Decimal};
use serde::{Deserialize, Serialize};

use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum EventError {
    #[error("unknown asset: {0}")]
    UnknownAsset(String),
    #[error("{0:?} is not a valid decimal quantity")]
    BadQuantity(String),
    #[error(
        "removing {removing} would leave a negative holding (currently {current}) — \
         record a correction instead if the stored quantity is wrong"
    )]
    WouldGoNegative { removing: String, current: String },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

/// What happened to a holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    /// First acquisition. One per asset.
    Acquire,
    /// Bought more of something already held.
    Add,
    /// Partial disposal.
    Remove,
    /// Disposed of entirely. The asset stops contributing to today's total
    /// but keeps its past contribution — that distinction is the whole point.
    Dispose,
    /// The recorded quantity was wrong. Adjusts without implying a trade, so
    /// a data-entry fix is not rendered as a purchase.
    Correct,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            EventType::Acquire => "acquire",
            EventType::Add => "add",
            EventType::Remove => "remove",
            EventType::Dispose => "dispose",
            EventType::Correct => "correct",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "acquire" => EventType::Acquire,
            "add" => EventType::Add,
            "remove" => EventType::Remove,
            "dispose" => EventType::Dispose,
            "correct" => EventType::Correct,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct NewEvent {
    pub asset_id: String,
    pub event_type: EventType,
    /// When it happened, not when it was recorded. These differ whenever
    /// someone enters a purchase after the fact.
    pub effective_date: String,
    /// Signed change. Positive adds, negative removes.
    pub quantity_delta: Decimal,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub event_id: String,
    pub asset_id: String,
    pub event_type: EventType,
    pub effective_date: String,
    pub quantity_delta: Decimal,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub note: String,
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

/// Record an event and refresh the cached quantity.
///
/// Both happen in one transaction: if the cache could drift from the log,
/// every historical figure would become suspect.
pub fn record(vault: &Vault, event: &NewEvent, now: &str) -> Result<String, EventError> {
    let exists: i64 = vault.conn().query_row(
        "SELECT count(*) FROM assets WHERE asset_id = ?1",
        [&event.asset_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(EventError::UnknownAsset(event.asset_id.clone()));
    }

    // A removal that would push the holding below zero is almost always a
    // mistyped quantity. Refusing it with a pointer to `Correct` is more
    // useful than storing an impossible holding.
    if event.quantity_delta.is_sign_negative() {
        let current = quantity_as_of(vault, &event.asset_id, None)?;
        if current + event.quantity_delta < Decimal::ZERO
            && event.event_type != EventType::Correct
        {
            return Err(EventError::WouldGoNegative {
                removing: (-event.quantity_delta).to_string(),
                current: current.to_string(),
            });
        }
    }

    let event_id = uuid_v4();
    let tx = vault.conn().unchecked_transaction()?;

    tx.execute(
        "INSERT INTO asset_events
           (event_id, asset_id, event_type, effective_date, quantity_delta,
            amount_minor, currency, note, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            &event_id,
            &event.asset_id,
            event.event_type.as_str(),
            &event.effective_date,
            event.quantity_delta.to_string(),
            event.amount_minor,
            &event.currency,
            &event.note,
            now
        ],
    )?;

    refresh_quantity_cache_in(&tx, &event.asset_id, now)?;
    tx.commit()?;
    Ok(event_id)
}

/// Quantity held as of a date, by replaying the log.
///
/// `as_of = None` means "now". This is the function that makes a historical
/// chart correct — never read `assets.quantity` for a past date.
pub fn quantity_as_of(
    vault: &Vault,
    asset_id: &str,
    as_of: Option<&str>,
) -> Result<Decimal, EventError> {
    let deltas: Vec<String> = match as_of {
        Some(date) => {
            let mut stmt = vault.conn().prepare(
                "SELECT quantity_delta FROM asset_events
                 WHERE asset_id = ?1 AND effective_date <= ?2
                 ORDER BY effective_date, recorded_at",
            )?;
            let v = stmt
                .query_map(rusqlite::params![asset_id, date], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            v
        }
        None => {
            let mut stmt = vault.conn().prepare(
                "SELECT quantity_delta FROM asset_events
                 WHERE asset_id = ?1
                 ORDER BY effective_date, recorded_at",
            )?;
            let v = stmt.query_map([asset_id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            v
        }
    };

    let mut total = Decimal::ZERO;
    for raw in deltas {
        total += parse_decimal(&raw).map_err(|_| EventError::BadQuantity(raw))?;
    }
    Ok(total)
}

/// Rebuild the cached quantity from the log.
///
/// Exposed so a repair path exists: if the cache is ever suspected wrong, the
/// log is the source of truth and this restores agreement.
pub fn refresh_quantity_cache(
    vault: &Vault,
    asset_id: &str,
    now: &str,
) -> Result<Decimal, EventError> {
    let tx = vault.conn().unchecked_transaction()?;
    let total = refresh_quantity_cache_in(&tx, asset_id, now)?;
    tx.commit()?;
    Ok(total)
}

fn refresh_quantity_cache_in(
    tx: &rusqlite::Transaction<'_>,
    asset_id: &str,
    now: &str,
) -> Result<Decimal, EventError> {
    let mut stmt = tx.prepare(
        "SELECT quantity_delta FROM asset_events WHERE asset_id = ?1
         ORDER BY effective_date, recorded_at",
    )?;
    let deltas: Vec<String> =
        stmt.query_map([asset_id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut total = Decimal::ZERO;
    for raw in deltas {
        total += parse_decimal(&raw).map_err(|_| EventError::BadQuantity(raw))?;
    }

    tx.execute(
        "UPDATE assets SET quantity = ?1, quantity_sort = ?2, updated_at = ?3
         WHERE asset_id = ?4",
        rusqlite::params![total.to_string(), sort_key(total), now, asset_id],
    )?;

    // A fully disposed holding is retired rather than deleted, so its past
    // contribution survives while it stops counting toward today's total.
    if total == Decimal::ZERO {
        tx.execute(
            "UPDATE assets SET status = 'sold' WHERE asset_id = ?1 AND status = 'active'",
            [asset_id],
        )?;
    } else {
        tx.execute(
            "UPDATE assets SET status = 'active' WHERE asset_id = ?1 AND status = 'sold'",
            [asset_id],
        )?;
    }

    Ok(total)
}

pub fn history(vault: &Vault, asset_id: &str) -> Result<Vec<Event>, EventError> {
    let mut stmt = vault.conn().prepare(
        "SELECT event_id, asset_id, event_type, effective_date, quantity_delta,
                amount_minor, currency, note, recorded_at
         FROM asset_events WHERE asset_id = ?1
         ORDER BY effective_date, recorded_at",
    )?;

    let rows = stmt
        .query_map([asset_id], |r| {
            let delta: String = r.get(4)?;
            let kind: String = r.get(2)?;
            Ok(Event {
                event_id: r.get(0)?,
                asset_id: r.get(1)?,
                event_type: EventType::parse(&kind).unwrap_or(EventType::Correct),
                effective_date: r.get(3)?,
                quantity_delta: parse_decimal(&delta).unwrap_or(Decimal::ZERO),
                amount_minor: r.get(5)?,
                currency: r.get(6)?,
                note: r.get(7)?,
                recorded_at: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_crypto::KdfParams;
    use std::str::FromStr;

    const PASS: &str = "correct horse battery staple";
    const NOW: &str = "2026-09-19T00:00:00Z";

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let (vault, _r) = Vault::create(&dir.path().join("vault"), PASS, &fast(), NOW).unwrap();
        vault
            .conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, quantity, created_at, updated_at)
                 VALUES ('a1','silver_bullion','Silver Eagles','0',?1,?1)",
                [NOW],
            )
            .unwrap();
        (dir, vault)
    }

    fn event(kind: EventType, delta: &str, date: &str) -> NewEvent {
        NewEvent {
            asset_id: "a1".into(),
            event_type: kind,
            effective_date: date.into(),
            quantity_delta: Decimal::from_str(delta).unwrap(),
            amount_minor: None,
            currency: None,
            note: String::new(),
        }
    }

    #[test]
    fn quantity_is_derived_from_the_log() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();

        let cached: String = v
            .conn()
            .query_row("SELECT quantity FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cached, "10", "cache must follow the log");
        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::from(10));
    }

    #[test]
    fn historical_quantity_is_not_todays_quantity() {
        // The modelling bug this module exists to prevent.
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();
        record(&v, &event(EventType::Remove, "-5", "2026-06-01"), NOW).unwrap();

        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::from(5), "today");
        assert_eq!(
            quantity_as_of(&v, "a1", Some("2026-03-01")).unwrap(),
            Decimal::from(10),
            "in March the holding was 10 — applying today's 5 would rewrite the past"
        );
        assert_eq!(
            quantity_as_of(&v, "a1", Some("2025-12-01")).unwrap(),
            Decimal::ZERO,
            "before acquisition the holding was nothing, not 10"
        );
    }

    #[test]
    fn a_full_disposal_retires_without_erasing_history() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();
        record(&v, &event(EventType::Dispose, "-10", "2026-06-01"), NOW).unwrap();

        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::ZERO);

        let status: String = v
            .conn()
            .query_row("SELECT status FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "sold", "a disposed holding stops counting toward today");

        // ...but its past contribution survives.
        assert_eq!(quantity_as_of(&v, "a1", Some("2026-03-01")).unwrap(), Decimal::from(10));
    }

    #[test]
    fn reacquiring_makes_a_sold_asset_active_again() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "5", "2026-01-01"), NOW).unwrap();
        record(&v, &event(EventType::Dispose, "-5", "2026-02-01"), NOW).unwrap();
        record(&v, &event(EventType::Add, "3", "2026-03-01"), NOW).unwrap();

        let status: String = v
            .conn()
            .query_row("SELECT status FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "active");
        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::from(3));
    }

    #[test]
    fn fractional_quantities_stay_exact() {
        // Crypto: eight decimal places must survive replay.
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "0.12345678", "2026-01-01"), NOW).unwrap();
        record(&v, &event(EventType::Add, "0.87654322", "2026-02-01"), NOW).unwrap();

        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::ONE);

        let cached: String = v
            .conn()
            .query_row("SELECT quantity FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cached, "1.00000000", "exact decimal, not a float approximation");
    }

    #[test]
    fn removing_more_than_held_is_refused() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "5", "2026-01-01"), NOW).unwrap();

        let err = record(&v, &event(EventType::Remove, "-10", "2026-02-01"), NOW).unwrap_err();
        assert!(matches!(err, EventError::WouldGoNegative { .. }));
        // The message should point at the right fix.
        assert!(err.to_string().contains("correction"));

        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::from(5), "unchanged");
    }

    #[test]
    fn a_correction_may_go_negative_where_a_removal_may_not() {
        // Corrections fix bad data, including data that was too high.
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "5", "2026-01-01"), NOW).unwrap();
        record(&v, &event(EventType::Correct, "-8", "2026-02-01"), NOW).unwrap();

        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::from(-3));
    }

    #[test]
    fn events_on_an_unknown_asset_are_refused() {
        let (_d, v) = setup();
        let mut e = event(EventType::Acquire, "1", "2026-01-01");
        e.asset_id = "does-not-exist".into();
        assert!(matches!(record(&v, &e, NOW), Err(EventError::UnknownAsset(_))));
    }

    #[test]
    fn cache_can_be_rebuilt_from_the_log() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "7", "2026-01-01"), NOW).unwrap();

        // Corrupt the cache, as a bad migration or manual edit might.
        v.conn().execute("UPDATE assets SET quantity='999' WHERE asset_id='a1'", []).unwrap();

        let rebuilt = refresh_quantity_cache(&v, "a1", NOW).unwrap();
        assert_eq!(rebuilt, Decimal::from(7));

        let cached: String = v
            .conn()
            .query_row("SELECT quantity FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cached, "7", "the log is the source of truth");
    }

    #[test]
    fn history_is_ordered_by_when_things_happened() {
        // Events entered out of order must still replay chronologically.
        let (_d, v) = setup();
        record(&v, &event(EventType::Add, "3", "2026-06-01"), NOW).unwrap();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();

        let events = history(&v, "a1").unwrap();
        assert_eq!(events[0].effective_date, "2026-01-01", "earliest first");
        assert_eq!(events[1].effective_date, "2026-06-01");

        // And a mid-range query sees only what had happened by then.
        assert_eq!(quantity_as_of(&v, "a1", Some("2026-03-01")).unwrap(), Decimal::from(10));
    }

    #[test]
    fn sort_key_tracks_the_cached_quantity() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "100", "2026-01-01"), NOW).unwrap();

        let sort: f64 = v
            .conn()
            .query_row("SELECT quantity_sort FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sort, 100.0);
    }
}
