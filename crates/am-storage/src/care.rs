//! Care and service history, and what is due next.
//!
//! "Due" is per asset and kind: the most recent entry of a kind says when
//! the next one is due, and recording that next one settles it. So servicing
//! a watch every five years needs one entry per service, each naming the
//! next — no separate reminder to keep in step.

use am_core::Money;
use serde::{Deserialize, Serialize};

use crate::events::normalize_date;
use crate::vault::Vault;

pub const KINDS: &[&str] = &[
    "service",
    "repair",
    "inspection",
    "cleaning",
    "appraisal",
    "battery",
    "warranty",
    "other",
];

#[derive(Debug, thiserror::Error)]
pub enum CareError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

/// A new entry, as the owner gives it.
#[derive(Debug, Clone, Deserialize)]
pub struct NewCare {
    pub asset_id: String,
    pub kind: String,
    /// When it was done. May be absent for a date that is only due — a
    /// warranty's expiry, an appraisal to book.
    #[serde(default)]
    pub performed_on: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(skip)]
    pub cost: Option<Money>,
    #[serde(default)]
    pub note: String,
    /// One of this asset's attachments — the invoice, the warranty card.
    #[serde(default)]
    pub object_id: Option<String>,
    #[serde(default)]
    pub next_due: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CareEntry {
    pub care_id: String,
    pub asset_id: String,
    pub kind: String,
    pub performed_on: Option<String>,
    pub provider: Option<String>,
    pub cost_minor: Option<i64>,
    pub currency: Option<String>,
    pub note: String,
    pub object_id: Option<String>,
    /// The linked document's title, for display.
    pub document_title: Option<String>,
    pub next_due: Option<String>,
    pub recorded_at: String,
}

fn optional_date(raw: &Option<String>) -> Result<Option<String>, CareError> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(d) => normalize_date(d).map(Some).map_err(|e| CareError::Invalid(e.to_string())),
    }
}

pub fn add(vault: &Vault, care: &NewCare, today: &str, now: &str) -> Result<String, CareError> {
    if !KINDS.contains(&care.kind.as_str()) {
        return Err(CareError::Invalid(format!("unknown kind of care: {}", care.kind)));
    }
    let performed_on = optional_date(&care.performed_on)?;
    let next_due = optional_date(&care.next_due)?;
    if performed_on.is_none() && next_due.is_none() {
        return Err(CareError::Invalid("give the date it was done, or when it is due".into()));
    }
    if performed_on.as_deref().is_some_and(|d| d > today) {
        return Err(CareError::Invalid(
            "the date it was done cannot be in the future — use “next due” for that".into(),
        ));
    }
    if let (Some(done), Some(due)) = (&performed_on, &next_due) {
        if due <= done {
            return Err(CareError::Invalid("the next one is due after this one".into()));
        }
    }
    let provider = care.provider.as_deref().map(str::trim).filter(|p| !p.is_empty());
    if provider.is_some_and(|p| p.chars().count() > 200) || care.note.chars().count() > 2_000 {
        return Err(CareError::Invalid("that text is too long".into()));
    }
    let exists: i64 = vault.conn().query_row(
        "SELECT count(*) FROM assets WHERE asset_id = ?1 AND deleted_at IS NULL",
        [&care.asset_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(CareError::Invalid("unknown asset".into()));
    }
    if let Some(object_id) = &care.object_id {
        let attached: i64 = vault.conn().query_row(
            "SELECT count(*) FROM asset_media WHERE asset_id = ?1 AND object_id = ?2",
            [&care.asset_id, object_id],
            |r| r.get(0),
        )?;
        if attached == 0 {
            return Err(CareError::Invalid(
                "that document is not attached to this asset".into(),
            ));
        }
    }
    let id = new_id();
    vault.conn().execute(
        "INSERT INTO care_events
           (care_id, asset_id, kind, performed_on, provider, cost_minor, currency, note,
            object_id, next_due, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            &id,
            &care.asset_id,
            &care.kind,
            &performed_on,
            provider,
            care.cost.as_ref().map(|m| m.amount_minor),
            care.cost.as_ref().map(|m| m.currency.code().to_string()),
            care.note.trim(),
            &care.object_id,
            &next_due,
            now
        ],
    )?;
    Ok(id)
}

pub fn delete(vault: &Vault, care_id: &str) -> Result<(), CareError> {
    let n = vault.conn().execute("DELETE FROM care_events WHERE care_id = ?1", [care_id])?;
    if n == 0 {
        return Err(CareError::Invalid("that entry no longer exists".into()));
    }
    Ok(())
}

const SELECT: &str = "
    SELECT c.care_id, c.asset_id, c.kind, c.performed_on, c.provider, c.cost_minor, c.currency,
           c.note, c.object_id,
           (SELECT coalesce(m.title, m.doc_kind) FROM asset_media m
             WHERE m.asset_id = c.asset_id AND m.object_id = c.object_id),
           c.next_due, c.recorded_at
    FROM care_events c";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CareEntry> {
    Ok(CareEntry {
        care_id: r.get(0)?,
        asset_id: r.get(1)?,
        kind: r.get(2)?,
        performed_on: r.get(3)?,
        provider: r.get(4)?,
        cost_minor: r.get(5)?,
        currency: r.get(6)?,
        note: r.get(7)?,
        object_id: r.get(8)?,
        document_title: r.get(9)?,
        next_due: r.get(10)?,
        recorded_at: r.get(11)?,
    })
}

/// An asset's history, most recent first.
pub fn history(vault: &Vault, asset_id: &str) -> Result<Vec<CareEntry>, CareError> {
    let mut stmt = vault.conn().prepare(&format!(
        "{SELECT} WHERE c.asset_id = ?1
         ORDER BY coalesce(c.performed_on, c.next_due) DESC, c.recorded_at DESC"
    ))?;
    let rows = stmt.query_map([asset_id], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Something due, with the asset it is for.
#[derive(Debug, Clone, Serialize)]
pub struct Due {
    pub entry: CareEntry,
    pub asset_name: String,
    pub overdue: bool,
}

/// What is due by `today + within_days`, overdue first: per held asset and
/// kind, the next-due date on the latest entry of that kind.
pub fn due(vault: &Vault, today: &str, within_days: i64) -> Result<Vec<Due>, CareError> {
    let horizon = crate::summary::add_days(today, within_days)
        .ok_or_else(|| CareError::Invalid("bad date".into()))?;
    let mut stmt = vault.conn().prepare(&format!(
        "{SELECT} JOIN assets a ON a.asset_id = c.asset_id
         WHERE a.deleted_at IS NULL AND a.status = 'active'
         ORDER BY c.asset_id, c.kind, coalesce(c.performed_on, c.next_due) DESC, c.recorded_at DESC"
    ))?;
    let entries = stmt.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    let mut out = Vec::new();
    let mut last: Option<(String, String)> = None;
    for entry in entries {
        let key = (entry.asset_id.clone(), entry.kind.clone());
        if last.as_ref() == Some(&key) {
            continue; // only the latest of each kind decides what is due
        }
        last = Some(key);
        let Some(next) = entry.next_due.clone() else { continue };
        if next > horizon {
            continue;
        }
        let asset_name: String = vault.conn().query_row(
            "SELECT name FROM assets WHERE asset_id = ?1",
            [&entry.asset_id],
            |r| r.get(0),
        )?;
        out.push(Due { overdue: next.as_str() < today, asset_name, entry });
    }
    out.sort_by(|a, b| a.entry.next_due.cmp(&b.entry.next_due));
    Ok(out)
}

fn new_id() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{create, NewAsset, Pricing};
    use am_core::{Currency, Decimal};
    use am_crypto::KdfParams;

    const NOW: &str = "2026-09-19T00:00:00Z";
    const TODAY: &str = "2026-09-19";

    fn setup() -> (tempfile::TempDir, Vault, String) {
        let dir = tempfile::tempdir().unwrap();
        let fast = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
        let (v, _r) =
            Vault::create(&dir.path().join("v"), "correct horse battery staple", &fast, NOW)
                .unwrap();
        let id = create(
            &v,
            &NewAsset {
                type_id: "watch".into(),
                name: "Speedmaster".into(),
                quantity: Decimal::ONE,
                quantity_unit: "item".into(),
                acquired_date: None,
                effective_date: None,
                acquired_cost: None,
                acquired_from: None,
                storage_location: None,
                notes: String::new(),
                insured: None,
                attrs: Default::default(),
                pricing: Pricing::Manual,
                review_every_days: None,
            },
            NOW,
        )
        .unwrap();
        (dir, v, id)
    }

    fn care(asset: &str, kind: &str, done: Option<&str>, next: Option<&str>) -> NewCare {
        NewCare {
            asset_id: asset.into(),
            kind: kind.into(),
            performed_on: done.map(str::to_string),
            provider: Some("Omega boutique".into()),
            cost: Some(Money::new(55_000, Currency::new("USD").unwrap())),
            note: String::new(),
            object_id: None,
            next_due: next.map(str::to_string),
        }
    }

    #[test]
    fn the_latest_entry_of_a_kind_says_what_is_due() {
        let (_d, v, id) = setup();
        add(&v, &care(&id, "service", Some("2021-09-01"), Some("2026-09-01")), TODAY, NOW)
            .unwrap();
        add(&v, &care(&id, "warranty", None, Some("2026-12-01")), TODAY, NOW).unwrap();

        let due_now = due(&v, TODAY, 30).unwrap();
        assert_eq!(due_now.len(), 1);
        assert!(due_now[0].overdue, "the service was due on 1 September");
        assert_eq!(due(&v, TODAY, 90).unwrap().len(), 2, "the warranty ends within 90 days");

        // Servicing it settles the overdue entry and sets the next one.
        add(&v, &care(&id, "service", Some("2026-09-15"), Some("2031-09-15")), TODAY, NOW)
            .unwrap();
        assert!(due(&v, TODAY, 30).unwrap().is_empty());
        assert_eq!(history(&v, &id).unwrap().len(), 3);
    }

    #[test]
    fn entries_are_validated() {
        let (_d, v, id) = setup();
        assert!(add(&v, &care(&id, "polish", Some("2026-01-01"), None), TODAY, NOW).is_err());
        assert!(
            add(&v, &care(&id, "service", None, None), TODAY, NOW).is_err(),
            "needs a date"
        );
        assert!(
            add(&v, &care(&id, "service", Some("2027-01-01"), None), TODAY, NOW).is_err(),
            "not in the future"
        );
        assert!(add(
            &v,
            &care(&id, "service", Some("2026-01-01"), Some("2025-01-01")),
            TODAY,
            NOW
        )
        .is_err());
        let mut linked = care(&id, "repair", Some("2026-01-01"), None);
        linked.object_id = Some("ab".repeat(16));
        assert!(add(&v, &linked, TODAY, NOW).is_err(), "only this asset's documents");
    }

    #[test]
    fn a_trashed_or_sold_asset_has_nothing_due() {
        let (_d, v, id) = setup();
        add(&v, &care(&id, "service", Some("2020-01-01"), Some("2025-01-01")), TODAY, NOW)
            .unwrap();
        crate::assets::trash(&v, &id, NOW).unwrap();
        assert!(due(&v, TODAY, 30).unwrap().is_empty());
    }
}
