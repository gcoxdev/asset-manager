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
    #[error("{0:?} is not a date — use YYYY-MM-DD")]
    BadDate(String),
    #[error(
        "removing {removing} would leave a negative holding (currently {current}) — \
         record a correction instead if the stored quantity is wrong"
    )]
    WouldGoNegative { removing: String, current: String },
    #[error(
        "that would leave a negative holding on {date}: a later sale or removal depends \
         on these units — record the change on or after that date, or correct the later one"
    )]
    WouldGoNegativeLater { date: String },
    #[error(
        "moving the acquisition to that date would leave a negative holding on {date}, \
         before anything was acquired"
    )]
    AcquisitionAfterRemoval { date: String },
    #[error("that amount is too large to add to the recorded cost")]
    AmountOverflow,
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

/// Reduce a date or timestamp to the calendar date it names.
///
/// Effective dates are compared as text against plain `YYYY-MM-DD` dates
/// throughout. A full timestamp sorts *after* its own date, so storing one
/// made a same-day acquisition look not-yet-held — the asset dropped out of
/// that day's total. Every write path normalizes here so that cannot recur.
pub fn normalize_date(raw: &str) -> Result<String, EventError> {
    let trimmed = raw.trim();
    let date = trimmed.get(..10).ok_or_else(|| EventError::BadDate(raw.to_string()))?;
    let bytes = date.as_bytes();
    let shape_ok = bytes.iter().enumerate().all(|(i, b)| match i {
        4 | 7 => *b == b'-',
        _ => b.is_ascii_digit(),
    });
    let year: i64 = date.get(0..4).and_then(|y| y.parse().ok()).unwrap_or(0);
    let month: u32 = date.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(0);
    let day: u32 = date.get(8..10).and_then(|d| d.parse().ok()).unwrap_or(0);
    // A real calendar date, not just day 1–31: "2026-02-30" would otherwise
    // be stored and sort between real dates as though it existed.
    if !shape_ok || !crate::series::is_calendar_date(year, month, day) {
        return Err(EventError::BadDate(raw.to_string()));
    }
    Ok(date.to_string())
}

/// Record an event and refresh the cached quantity.
///
/// Both happen in one transaction: if the cache could drift from the log,
/// every historical figure would become suspect.
///
/// # Cost basis
///
/// Decision 2 keeps a single acquisition cost for the whole position, not
/// lots. Quantity changes keep that one number honest:
///
/// - **Add** with a price paid adds it to the position's cost. Without one
///   the cost is left as it was — the added units have unknown cost, and
///   inventing zero would overstate the gain — and marked incomplete, so no
///   gain is computed against a cost that covers only part of the holding.
/// - **Remove** reduces cost in proportion to what remains (average cost),
///   so a half-sold position does not keep its full original cost.
/// - **Dispose** records the sale date and, if given, what it sold for.
/// - **Correct** fixes the count and leaves cost alone: it is a data fix, not
///   a trade.
///
/// These are applied by replaying the whole log in effective-date order (see
/// [`rebuild_cost_in`]), so the result does not depend on the order changes
/// were typed in.
pub fn record(vault: &Vault, event: &NewEvent, now: &str) -> Result<String, EventError> {
    let effective_date = normalize_date(&event.effective_date)?;
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
    // useful than storing an impossible holding. Checked across the whole
    // timeline, not just today's total: a sale backdated to February must
    // fit what was held in February, and must not take units that a later
    // sale already accounted for.
    if event.quantity_delta.is_sign_negative() && event.event_type != EventType::Correct {
        check_removal_fits(vault, &event.asset_id, &effective_date, event.quantity_delta)?;
    }

    let event_id = uuid_v4();
    let tx = crate::atomic::begin(vault.conn())?;

    tx.execute(
        "INSERT INTO asset_events
           (event_id, asset_id, event_type, effective_date, quantity_delta,
            amount_minor, currency, note, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            &event_id,
            &event.asset_id,
            event.event_type.as_str(),
            &effective_date,
            event.quantity_delta.to_string(),
            event.amount_minor,
            &event.currency,
            &event.note,
            now
        ],
    )?;

    refresh_quantity_cache_in(&tx, &event.asset_id, now)?;
    rebuild_cost_in(&tx, &event.asset_id)?;
    if event.event_type == EventType::Dispose {
        tx.execute(
            "UPDATE assets SET sold_date = ?1, sold_amount_minor = ?2, sold_currency = ?3
             WHERE asset_id = ?4",
            rusqlite::params![
                &effective_date,
                event.amount_minor,
                event.amount_minor.and(event.currency.clone()),
                &event.asset_id
            ],
        )?;
    }
    crate::valuations::refresh_current_value_in(&tx, &event.asset_id, now)
        .map_err(|e| EventError::BadQuantity(e.to_string()))?;
    tx.commit()?;
    Ok(event_id)
}

/// Every event's date and quantity change, in the order they took effect.
/// Same-day events apply in the order they were recorded.
fn timeline(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<Vec<(String, Decimal)>, EventError> {
    let mut stmt = conn.prepare(
        "SELECT effective_date, quantity_delta FROM asset_events WHERE asset_id = ?1
         ORDER BY effective_date, recorded_at",
    )?;
    let rows = stmt
        .query_map([asset_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(date, raw)| {
            let delta = parse_decimal(&raw).map_err(|_| EventError::BadQuantity(raw))?;
            Ok((date, delta))
        })
        .collect()
}

/// The smallest holding at any point in the timeline, with the date it
/// occurs. Zero for an empty timeline.
fn lowest_holding(events: &[(String, Decimal)]) -> (Decimal, Option<String>) {
    let mut held = Decimal::ZERO;
    let mut lowest = (Decimal::ZERO, None);
    for (date, delta) in events {
        held += *delta;
        if held < lowest.0 {
            lowest = (held, Some(date.clone()));
        }
    }
    lowest
}

/// Refuse a removal that does not fit the holding on its date, or that would
/// leave a later removal taking units no longer there.
fn check_removal_fits(
    vault: &Vault,
    asset_id: &str,
    effective_date: &str,
    delta: Decimal,
) -> Result<(), EventError> {
    let mut events = timeline(vault.conn(), asset_id)?;
    // Recorded now, so it applies after everything already on its date.
    let at = events.partition_point(|(date, _)| date.as_str() <= effective_date);
    let held_then: Decimal = events[..at].iter().map(|(_, d)| *d).sum();
    if held_then + delta < Decimal::ZERO {
        return Err(EventError::WouldGoNegative {
            removing: (-delta).normalize().to_string(),
            current: held_then.normalize().to_string(),
        });
    }
    events.insert(at, (effective_date.to_string(), delta));
    let mut held = Decimal::ZERO;
    for (i, (date, d)) in events.iter().enumerate() {
        held += *d;
        if i > at && held < Decimal::ZERO {
            return Err(EventError::WouldGoNegativeLater { date: date.clone() });
        }
    }
    Ok(())
}

/// Check that moving the acquisition to a new date leaves no point in the
/// history holding less than before the move — a sale dated before the
/// purchase would otherwise appear as a negative holding.
pub(crate) fn check_acquisition_move(
    conn: &rusqlite::Connection,
    asset_id: &str,
    new_date: &str,
) -> Result<(), EventError> {
    let mut stmt = conn.prepare(
        "SELECT effective_date, quantity_delta, event_type = 'acquire' FROM asset_events
         WHERE asset_id = ?1 ORDER BY effective_date, recorded_at",
    )?;
    let mut events = Vec::new();
    let mut moved = Vec::new();
    for row in stmt.query_map([asset_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?))
    })? {
        let (date, raw, acquire) = row?;
        let delta = parse_decimal(&raw).map_err(|_| EventError::BadQuantity(raw))?;
        moved.push((if acquire { new_date.to_string() } else { date.clone() }, delta));
        events.push((date, delta));
    }
    // A stable sort: same-day events keep their recorded order.
    moved.sort_by(|a, b| a.0.cmp(&b.0));
    let (before, _) = lowest_holding(&events);
    let (after, date) = lowest_holding(&moved);
    if after < Decimal::ZERO && after < before {
        return Err(EventError::AcquisitionAfterRemoval { date: date.unwrap_or_default() });
    }
    Ok(())
}

/// One entry in the cost replay.
struct CostStep {
    date: String,
    recorded_at: String,
    kind: EventType,
    delta: Decimal,
    amount: Option<(i64, String)>,
}

/// The owner's latest statement of what the holding cost.
struct CostStatement {
    date: String,
    recorded_at: String,
    amount: Option<(i64, String)>,
    covers_holding: bool,
}

/// The position's cost as the history says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedCost {
    /// `None`: unknown. Zero is a known cost (a gift).
    pub amount: Option<(i64, String)>,
    /// False when part of the holding arrived at an unknown cost.
    pub complete: bool,
}

/// Replay the log, in effective-date order, into the position's cost.
///
/// Starts from the latest cost statement (the owner saying "what I hold cost
/// this much"), or from nothing. Changes dated after the statement apply in
/// date order; changes dated before it but *recorded* after it apply right
/// after it, because the statement could not have accounted for them. With
/// no statement, every change applies in date order from the acquisition.
pub fn derive_cost(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<DerivedCost, EventError> {
    let mut stmt = conn.prepare(
        "SELECT effective_date, recorded_at, event_type, quantity_delta, amount_minor, currency
         FROM asset_events WHERE asset_id = ?1 ORDER BY effective_date, recorded_at, rowid",
    )?;
    let raw = stmt
        .query_map([asset_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    let mut steps = Vec::with_capacity(raw.len());
    for (date, recorded_at, kind, delta, minor, currency) in raw {
        steps.push(CostStep {
            date,
            recorded_at,
            kind: EventType::parse(&kind).unwrap_or(EventType::Correct),
            delta: parse_decimal(&delta).map_err(|_| EventError::BadQuantity(delta))?,
            amount: minor.zip(currency),
        });
    }

    let statement: Option<CostStatement> = conn
        .query_row(
            "SELECT effective_date, recorded_at, amount_minor, currency, covers_holding
             FROM cost_statements WHERE asset_id = ?1
             ORDER BY effective_date DESC, recorded_at DESC LIMIT 1",
            [asset_id],
            |r| {
                let minor: Option<i64> = r.get(2)?;
                let currency: Option<String> = r.get(3)?;
                Ok(CostStatement {
                    date: r.get(0)?,
                    recorded_at: r.get(1)?,
                    amount: minor.zip(currency),
                    covers_holding: r.get::<_, i64>(4)? != 0,
                })
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;

    let mut cost = DerivedCost { amount: None, complete: true };
    let mut held = Decimal::ZERO;
    let sequence: Vec<&CostStep> = match &statement {
        None => steps.iter().collect(),
        Some(s) => {
            cost = DerivedCost { amount: s.amount.clone(), complete: s.covers_holding };
            let mut late = Vec::new();
            let mut after = Vec::new();
            for step in &steps {
                if step.date > s.date {
                    after.push(step);
                } else if step.recorded_at > s.recorded_at {
                    late.push(step);
                } else {
                    // Known when the statement was made: it is in the total.
                    held += step.delta;
                }
            }
            late.into_iter().chain(after).collect()
        }
    };

    for step in sequence {
        let before = held;
        held += step.delta;
        match step.kind {
            EventType::Acquire | EventType::Add if before <= Decimal::ZERO => {
                // Nothing was held, so this starts a fresh position: its cost
                // is what was paid now, not added to a sold one's.
                cost = DerivedCost { amount: step.amount.clone(), complete: true };
            }
            EventType::Acquire | EventType::Add => match (&cost.amount, &step.amount) {
                (Some((total, code)), Some((paid, paid_code))) if code == paid_code => {
                    let sum = total.checked_add(*paid).ok_or(EventError::AmountOverflow)?;
                    cost.amount = Some((sum, code.clone()));
                }
                // A known cost, and units whose cost is unknown or in another
                // currency: adding would produce a figure that looks exact and
                // is not, so the cost stands but no longer covers everything.
                (Some(_), _) => cost.complete = false,
                // No recorded cost: still unknown.
                (None, _) => {}
            },
            EventType::Remove => {
                if let Some((total, code)) = &cost.amount {
                    if before > Decimal::ZERO && held >= Decimal::ZERO {
                        let scaled = (Decimal::from(*total) * held / before)
                            .round_dp_with_strategy(
                                0,
                                rust_decimal::RoundingStrategy::MidpointNearestEven,
                            );
                        let scaled: i64 = scaled.try_into().unwrap_or(*total);
                        cost.amount = Some((scaled, code.clone()));
                    }
                }
            }
            EventType::Dispose | EventType::Correct => {}
        }
    }
    Ok(cost)
}

/// Write the replayed cost into the asset's cached cost columns.
pub(crate) fn rebuild_cost_in(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<DerivedCost, EventError> {
    let cost = derive_cost(conn, asset_id)?;
    let (minor, currency) = match &cost.amount {
        Some((m, c)) => (Some(*m), Some(c.as_str())),
        None => (None, None),
    };
    conn.execute(
        "UPDATE assets SET acquired_amount_minor = ?1, acquired_currency = ?2, cost_complete = ?3
         WHERE asset_id = ?4",
        rusqlite::params![minor, currency, i64::from(cost.complete), asset_id],
    )?;
    Ok(cost)
}

/// The owner states what everything held cost in total — a correction of the
/// purchase price, or a restated total after buying more at an unknown price.
///
/// With nothing but the acquisition on record, the acquisition's price is
/// simply corrected. Otherwise a statement is recorded, dated no earlier than
/// the latest change, so it covers everything known when it was made.
pub fn restate_cost_in(
    conn: &rusqlite::Connection,
    asset_id: &str,
    cost: Option<(i64, String)>,
    note: &str,
    now: &str,
) -> Result<DerivedCost, EventError> {
    let (others, statements, latest): (i64, i64, Option<String>) = conn.query_row(
        "SELECT (SELECT count(*) FROM asset_events WHERE asset_id = ?1 AND event_type <> 'acquire'),
                (SELECT count(*) FROM cost_statements WHERE asset_id = ?1),
                (SELECT max(effective_date) FROM asset_events WHERE asset_id = ?1)",
        [asset_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let (minor, currency) = match &cost {
        Some((m, c)) => (Some(*m), Some(c.clone())),
        None => (None, None),
    };
    if others == 0 && statements == 0 {
        conn.execute(
            "UPDATE asset_events SET amount_minor = ?1, currency = ?2
             WHERE asset_id = ?3 AND event_type = 'acquire'",
            rusqlite::params![minor, currency, asset_id],
        )?;
    } else {
        let today = now.get(..10).unwrap_or(now).to_string();
        let date = latest.filter(|d| *d > today).unwrap_or(today);
        conn.execute(
            "INSERT INTO cost_statements
               (statement_id, asset_id, effective_date, amount_minor, currency, covers_holding,
                note, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)",
            rusqlite::params![uuid_v4(), asset_id, date, minor, currency, note, now],
        )?;
    }
    rebuild_cost_in(conn, asset_id)
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
    let tx = crate::atomic::begin(vault.conn())?;
    let total = refresh_quantity_cache_in(&tx, asset_id, now)?;
    tx.commit()?;
    Ok(total)
}

pub(crate) fn refresh_quantity_cache_in(
    tx: &rusqlite::Connection,
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

    // A fully disposed holding is marked sold rather than deleted, so its
    // past contribution survives while it stops counting toward today's
    // total.
    crate::lifecycle::refresh_status_in(tx, asset_id)?;

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
    fn a_backdated_sale_must_fit_what_was_held_then() {
        // January: 10. September: 10 more. A sale of 15 dated February fits
        // today's 20 — but only 10 were held in February.
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-15"), NOW).unwrap();
        record(&v, &event(EventType::Add, "10", "2026-09-01"), NOW).unwrap();

        let err = record(&v, &event(EventType::Remove, "-15", "2026-02-01"), NOW).unwrap_err();
        assert!(matches!(err, EventError::WouldGoNegative { .. }), "got: {err}");
        assert!(err.to_string().contains("currently 10"), "{err}");
        assert_eq!(quantity_as_of(&v, "a1", Some("2026-03-01")).unwrap(), Decimal::from(10));

        // Dated after the second purchase, the same sale is fine.
        record(&v, &event(EventType::Remove, "-15", "2026-09-02"), NOW).unwrap();
    }

    #[test]
    fn a_backdated_sale_cannot_take_units_a_later_sale_already_used() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();
        record(&v, &event(EventType::Remove, "-8", "2026-06-01"), NOW).unwrap();

        // Fits March on its own (10 held), but June's sale of 8 then finds 5.
        let err = record(&v, &event(EventType::Remove, "-5", "2026-03-01"), NOW).unwrap_err();
        assert!(
            matches!(err, EventError::WouldGoNegativeLater { ref date } if date == "2026-06-01")
        );
        assert_eq!(
            quantity_as_of(&v, "a1", None).unwrap(),
            Decimal::from(2),
            "nothing written"
        );
    }

    #[test]
    fn same_day_events_apply_in_the_order_recorded() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "3", "2026-01-01"), NOW).unwrap();
        // Bought two more and sold four, both on one day: in that order it fits.
        record(&v, &event(EventType::Add, "2", "2026-02-01"), NOW).unwrap();
        record(&v, &event(EventType::Remove, "-4", "2026-02-01"), "2026-09-19T00:00:01Z")
            .unwrap();
        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::ONE);
    }

    #[test]
    fn the_acquisition_cannot_move_after_a_sale_of_what_it_bought() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "5", "2026-01-01"), NOW).unwrap();
        record(&v, &event(EventType::Remove, "-2", "2026-03-01"), NOW).unwrap();

        let err = check_acquisition_move(v.conn(), "a1", "2026-04-01").unwrap_err();
        assert!(
            matches!(err, EventError::AcquisitionAfterRemoval { ref date } if date == "2026-03-01")
        );
        assert!(check_acquisition_move(v.conn(), "a1", "2026-02-28").is_ok());
        assert!(check_acquisition_move(v.conn(), "a1", "2025-12-01").is_ok());
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

    fn cost(v: &Vault) -> Option<i64> {
        v.conn()
            .query_row(
                "SELECT acquired_amount_minor FROM assets WHERE asset_id='a1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn cost_complete(v: &Vault) -> bool {
        v.conn()
            .query_row("SELECT cost_complete FROM assets WHERE asset_id='a1'", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
            == 1
    }

    /// What the acquisition cost. Cost is replayed from the log, so it is
    /// recorded on the acquire event, as `assets::create` does.
    fn set_cost(v: &Vault, minor: i64) {
        v.conn()
            .execute(
                "UPDATE asset_events SET amount_minor=?1, currency='USD'
                 WHERE asset_id='a1' AND event_type='acquire'",
                [minor],
            )
            .unwrap();
        rebuild_cost_in(v.conn(), "a1").unwrap();
    }

    #[test]
    fn timestamps_are_stored_as_the_date_they_name() {
        // The same-day bug: a stored timestamp sorts after its own date, so
        // the holding looked absent on the day it was bought.
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "1", "2026-09-22T14:03:11Z"), NOW).unwrap();

        assert_eq!(history(&v, "a1").unwrap()[0].effective_date, "2026-09-22");
        assert_eq!(
            quantity_as_of(&v, "a1", Some("2026-09-22")).unwrap(),
            Decimal::ONE,
            "held on the day it was acquired"
        );
    }

    #[test]
    fn malformed_dates_are_refused() {
        assert_eq!(normalize_date("2026-02-03").unwrap(), "2026-02-03");
        assert_eq!(normalize_date(" 2026-02-03T00:00:00Z ").unwrap(), "2026-02-03");
        assert_eq!(normalize_date("2024-02-29").unwrap(), "2024-02-29", "a leap day");
        for bad in [
            "",
            "yesterday",
            "2026-13-01",
            "2026-00-10",
            "2026/02/03",
            "26-02-03",
            "2026-02-29",
            "2026-02-30",
            "2026-06-31",
            "2026-01-00",
            "2026-01-32",
            "é026-01-01",
        ] {
            assert!(normalize_date(bad).is_err(), "accepted {bad:?}");
        }

        let (_d, v) = setup();
        assert!(matches!(
            record(&v, &event(EventType::Acquire, "1", "soon"), NOW),
            Err(EventError::BadDate(_))
        ));
    }

    #[test]
    fn buying_more_adds_what_was_paid_to_the_cost() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();
        set_cost(&v, 30_000);

        let mut add = event(EventType::Add, "5", "2026-02-01");
        add.amount_minor = Some(16_000);
        add.currency = Some("USD".into());
        record(&v, &add, NOW).unwrap();

        assert_eq!(cost(&v), Some(46_000));
    }

    #[test]
    fn buying_more_at_an_unknown_price_leaves_the_cost_alone() {
        // Treating a missing price as zero would overstate the gain.
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();
        set_cost(&v, 30_000);
        assert!(cost_complete(&v));
        record(&v, &event(EventType::Add, "5", "2026-02-01"), NOW).unwrap();
        assert_eq!(cost(&v), Some(30_000));
        assert!(!cost_complete(&v), "the cost now covers ten of fifteen units");

        // Nor does a price in another currency get added as if it were USD.
        set_cost(&v, 30_000);
        v.conn().execute("UPDATE assets SET cost_complete = 1", []).unwrap();
        let mut eur = event(EventType::Add, "1", "2026-03-01");
        eur.amount_minor = Some(5_000);
        eur.currency = Some("EUR".into());
        record(&v, &eur, NOW).unwrap();
        assert_eq!(cost(&v), Some(30_000));
        assert!(!cost_complete(&v));
    }

    #[test]
    fn a_priced_purchase_keeps_a_complete_cost_complete() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "1", "2026-01-01"), NOW).unwrap();
        set_cost(&v, 10_000);
        let mut add = event(EventType::Add, "1", "2026-02-01");
        add.amount_minor = Some(12_000);
        add.currency = Some("USD".into());
        record(&v, &add, NOW).unwrap();
        assert!(cost_complete(&v));
        assert_eq!(cost(&v), Some(22_000));
    }

    fn priced(kind: EventType, delta: &str, date: &str, minor: i64) -> NewEvent {
        let mut e = event(kind, delta, date);
        e.amount_minor = Some(minor);
        e.currency = Some("USD".into());
        e
    }

    #[test]
    fn cost_follows_effective_dates_not_typing_order() {
        // January: 10 for $100. September: 10 more for $200. A February sale
        // of 5, entered last. In date order: $100 → $50 after the sale →
        // $250 after September. Applied in typing order it came to $225.
        let (_d, v) = setup();
        record(&v, &priced(EventType::Acquire, "10", "2026-01-10", 10_000), NOW).unwrap();
        record(&v, &priced(EventType::Add, "10", "2026-09-01", 20_000), NOW).unwrap();
        record(&v, &event(EventType::Remove, "-5", "2026-02-01"), "2026-09-19T00:00:05Z")
            .unwrap();
        assert_eq!(cost(&v), Some(25_000));
        assert!(cost_complete(&v));
    }

    #[test]
    fn any_typing_order_gives_the_same_cost() {
        let changes = [
            priced(EventType::Add, "4", "2026-03-01", 8_000),
            event(EventType::Remove, "-6", "2026-05-01"),
            priced(EventType::Add, "2", "2026-07-01", 5_000),
            event(EventType::Remove, "-3", "2026-08-01"),
        ];
        let mut results = Vec::new();
        for order in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1]] {
            let (_d, v) = setup();
            record(&v, &priced(EventType::Acquire, "10", "2026-01-01", 10_000), NOW).unwrap();
            for (n, &i) in order.iter().enumerate() {
                // Some orders are refused on the way (a sale before the
                // purchase it needs); what is accepted must agree.
                let _ = record(&v, &changes[i], &format!("2026-09-19T00:00:{n:02}Z"));
            }
            if quantity_as_of(&v, "a1", None).unwrap() == Decimal::from(7) {
                results.push(cost(&v));
            }
        }
        assert!(results.len() >= 2, "at least two orders record everything");
        assert!(results.windows(2).all(|w| w[0] == w[1]), "{results:?}");
    }

    #[test]
    fn selling_everything_then_buying_again_starts_a_fresh_cost() {
        let (_d, v) = setup();
        record(&v, &priced(EventType::Acquire, "2", "2026-01-01", 10_000), NOW).unwrap();
        record(&v, &event(EventType::Dispose, "-2", "2026-02-01"), NOW).unwrap();
        record(&v, &priced(EventType::Add, "1", "2026-03-01", 7_000), NOW).unwrap();
        assert_eq!(cost(&v), Some(7_000), "not the sold position's cost plus this");
    }

    #[test]
    fn a_restated_total_holds_and_later_backdated_changes_apply_after_it() {
        let (_d, v) = setup();
        record(&v, &priced(EventType::Acquire, "10", "2026-01-01", 10_000), NOW).unwrap();
        record(&v, &event(EventType::Add, "10", "2026-06-01"), NOW).unwrap();
        assert!(!cost_complete(&v), "added at an unknown price");

        // The owner states what all 20 cost.
        restate_cost_in(
            v.conn(),
            "a1",
            Some((30_000, "USD".into())),
            "",
            "2026-09-19T00:00:01Z",
        )
        .unwrap();
        assert_eq!(cost(&v), Some(30_000));
        assert!(cost_complete(&v));

        // A sale of 5 in March, only now remembered: it applies to the stated
        // total at average cost, 15 of 20.
        record(&v, &event(EventType::Remove, "-5", "2026-03-01"), "2026-09-19T00:00:02Z")
            .unwrap();
        assert_eq!(cost(&v), Some(22_500));
    }

    #[test]
    fn restating_with_only_the_purchase_on_record_corrects_its_price() {
        let (_d, v) = setup();
        record(&v, &priced(EventType::Acquire, "1", "2026-01-01", 10_000), NOW).unwrap();
        restate_cost_in(v.conn(), "a1", Some((12_000, "USD".into())), "", NOW).unwrap();
        let (statements, acquire): (i64, i64) = v
            .conn()
            .query_row(
                "SELECT (SELECT count(*) FROM cost_statements),
                        (SELECT amount_minor FROM asset_events WHERE event_type = 'acquire')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((statements, acquire), (0, 12_000), "a typo fix, not a new statement");
        assert_eq!(cost(&v), Some(12_000));
    }

    #[test]
    fn an_overflowing_cost_is_refused_rather_than_clamped() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "1", "2026-01-01"), NOW).unwrap();
        set_cost(&v, i64::MAX - 10);
        let mut add = event(EventType::Add, "1", "2026-02-01");
        add.amount_minor = Some(100);
        add.currency = Some("USD".into());
        assert!(matches!(record(&v, &add, NOW), Err(EventError::AmountOverflow)));
        assert_eq!(cost(&v), Some(i64::MAX - 10), "nothing written");
        assert_eq!(quantity_as_of(&v, "a1", None).unwrap(), Decimal::ONE, "not even the event");
    }

    #[test]
    fn a_partial_sale_reduces_cost_in_proportion() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();
        set_cost(&v, 30_000);
        record(&v, &event(EventType::Remove, "-4", "2026-02-01"), NOW).unwrap();

        assert_eq!(cost(&v), Some(18_000), "six of ten remain, so 60% of the cost");
    }

    #[test]
    fn a_disposal_records_the_sale() {
        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "2", "2026-01-01"), NOW).unwrap();

        let mut sale = event(EventType::Dispose, "-2", "2026-05-04");
        sale.amount_minor = Some(99_900);
        sale.currency = Some("USD".into());
        record(&v, &sale, NOW).unwrap();

        let (date, amount, currency): (String, i64, String) = v
            .conn()
            .query_row(
                "SELECT sold_date, sold_amount_minor, sold_currency FROM assets
                 WHERE asset_id='a1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((date.as_str(), amount, currency.as_str()), ("2026-05-04", 99_900, "USD"));
    }

    #[test]
    fn a_partial_sale_scales_the_cached_current_value() {
        use crate::valuations::{record_valuation, Basis, NewValuation, Provenance};
        use am_core::{Currency, Money};

        let (_d, v) = setup();
        record(&v, &event(EventType::Acquire, "10", "2026-01-01"), NOW).unwrap();
        record_valuation(
            &v,
            &NewValuation {
                asset_id: "a1".into(),
                quote_id: None,
                value: Money::new(300_000, Currency::new("USD").unwrap()),
                quantity_at_time: Decimal::from(10),
                basis: Basis::EstimatedResale,
                provenance: Provenance::Manual,
                inputs: serde_json::json!({}),
                asof: "2026-01-15".into(),
            },
            NOW,
        )
        .unwrap();
        record(&v, &event(EventType::Remove, "-5", "2026-02-01"), NOW).unwrap();

        let current: i64 = v
            .conn()
            .query_row("SELECT current_amount_minor FROM assets WHERE asset_id='a1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(current, 150_000, "half the holding is worth half the value");
    }
}
