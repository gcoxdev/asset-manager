//! Who has it: loans, consignments, repairs, outside storage, shipping.
//!
//! An asset is away from the latest handoff until a later "returned". This
//! is about where a thing is, not who owns it — value and history are
//! untouched.

use serde::{Deserialize, Serialize};

use crate::events::normalize_date;
use crate::vault::Vault;

pub const AWAY_KINDS: &[&str] = &["lent", "consigned", "repair", "storage", "shipped"];

#[derive(Debug, thiserror::Error)]
pub enum CustodyError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewCustody {
    pub asset_id: String,
    /// One of [`AWAY_KINDS`], or "returned".
    pub kind: String,
    #[serde(default)]
    pub party: Option<String>,
    #[serde(default)]
    pub contact: Option<String>,
    pub date: String,
    #[serde(default)]
    pub due_back: Option<String>,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub object_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CustodyEntry {
    pub custody_id: String,
    pub asset_id: String,
    pub kind: String,
    pub party: Option<String>,
    pub contact: Option<String>,
    pub date: String,
    pub due_back: Option<String>,
    pub reference: Option<String>,
    pub note: String,
    pub object_id: Option<String>,
    pub document_title: Option<String>,
    pub recorded_at: String,
}

const SELECT: &str = "
    SELECT c.custody_id, c.asset_id, c.kind, c.party, c.contact, c.date, c.due_back,
           c.reference, c.note, c.object_id,
           (SELECT coalesce(m.title, m.doc_kind) FROM asset_media m
             WHERE m.asset_id = c.asset_id AND m.object_id = c.object_id),
           c.recorded_at
    FROM custody_events c";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CustodyEntry> {
    Ok(CustodyEntry {
        custody_id: r.get(0)?,
        asset_id: r.get(1)?,
        kind: r.get(2)?,
        party: r.get(3)?,
        contact: r.get(4)?,
        date: r.get(5)?,
        due_back: r.get(6)?,
        reference: r.get(7)?,
        note: r.get(8)?,
        object_id: r.get(9)?,
        document_title: r.get(10)?,
        recorded_at: r.get(11)?,
    })
}

/// The handoff in effect now, if the asset is away.
pub fn current(vault: &Vault, asset_id: &str) -> Result<Option<CustodyEntry>, CustodyError> {
    let latest = vault
        .conn()
        .query_row(
            &format!(
                "{SELECT} WHERE c.asset_id = ?1 ORDER BY c.date DESC, c.recorded_at DESC LIMIT 1"
            ),
            [asset_id],
            from_row,
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(latest.filter(|e| e.kind != "returned"))
}

fn clean(
    text: &Option<String>,
    limit: usize,
    what: &str,
) -> Result<Option<String>, CustodyError> {
    let t = text.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if t.is_some_and(|t| t.chars().count() > limit) {
        return Err(CustodyError::Invalid(format!("the {what} is too long")));
    }
    Ok(t.map(str::to_string))
}

pub fn record(
    vault: &Vault,
    entry: &NewCustody,
    today: &str,
    now: &str,
) -> Result<String, CustodyError> {
    let invalid = |m: &str| CustodyError::Invalid(m.to_string());
    let returning = entry.kind == "returned";
    if !returning && !AWAY_KINDS.contains(&entry.kind.as_str()) {
        return Err(CustodyError::Invalid(format!("unknown handoff: {}", entry.kind)));
    }
    let date = normalize_date(&entry.date).map_err(|e| CustodyError::Invalid(e.to_string()))?;
    if date.as_str() > today {
        return Err(invalid("a handoff cannot be dated in the future"));
    }
    let due_back = match entry.due_back.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(d) => Some(normalize_date(d).map_err(|e| CustodyError::Invalid(e.to_string()))?),
    };
    if due_back.as_ref().is_some_and(|d| *d < date) {
        return Err(invalid("it cannot be due back before it went"));
    }
    let party = clean(&entry.party, 200, "name")?;
    let contact = clean(&entry.contact, 300, "contact")?;
    let reference = clean(&entry.reference, 200, "reference")?;
    if entry.note.chars().count() > 2_000 {
        return Err(invalid("the note is too long"));
    }
    let exists: i64 = vault.conn().query_row(
        "SELECT count(*) FROM assets WHERE asset_id = ?1 AND deleted_at IS NULL",
        [&entry.asset_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(invalid("unknown asset"));
    }
    let away = current(vault, &entry.asset_id)?;
    if returning {
        let Some(away) = &away else {
            return Err(invalid("it is not recorded as away"));
        };
        if date < away.date {
            return Err(invalid("it cannot come back before it went"));
        }
    } else {
        if party.is_none() {
            return Err(invalid("say who has it"));
        }
        if let Some(away) = &away {
            if date < away.date {
                return Err(invalid(
                    "that is before its current handoff — record the return first",
                ));
            }
        }
    }
    if let Some(object_id) = &entry.object_id {
        let attached: i64 = vault.conn().query_row(
            "SELECT count(*) FROM asset_media WHERE asset_id = ?1 AND object_id = ?2",
            [&entry.asset_id, object_id],
            |r| r.get(0),
        )?;
        if attached == 0 {
            return Err(invalid("that document is not attached to this asset"));
        }
    }
    let id = new_id();
    vault.conn().execute(
        "INSERT INTO custody_events
           (custody_id, asset_id, kind, party, contact, date, due_back, reference, note,
            object_id, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            &id,
            &entry.asset_id,
            &entry.kind,
            party,
            contact,
            &date,
            &due_back,
            reference,
            entry.note.trim(),
            &entry.object_id,
            now
        ],
    )?;
    Ok(id)
}

pub fn history(vault: &Vault, asset_id: &str) -> Result<Vec<CustodyEntry>, CustodyError> {
    let mut stmt = vault.conn().prepare(&format!(
        "{SELECT} WHERE c.asset_id = ?1 ORDER BY c.date DESC, c.recorded_at DESC"
    ))?;
    let rows = stmt.query_map([asset_id], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn delete(vault: &Vault, custody_id: &str) -> Result<(), CustodyError> {
    let n = vault
        .conn()
        .execute("DELETE FROM custody_events WHERE custody_id = ?1", [custody_id])?;
    if n == 0 {
        return Err(CustodyError::Invalid("that entry no longer exists".into()));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Away {
    pub entry: CustodyEntry,
    pub asset_name: String,
    pub overdue: bool,
}

/// Everything in the catalog that is away now, overdue first, then by due
/// date.
pub fn away(vault: &Vault, today: &str) -> Result<Vec<Away>, CustodyError> {
    let ids: Vec<(String, String)> = {
        let mut stmt = vault.conn().prepare(
            "SELECT DISTINCT a.asset_id, a.name FROM custody_events c
             JOIN assets a ON a.asset_id = c.asset_id WHERE a.deleted_at IS NULL",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let mut out = Vec::new();
    for (id, name) in ids {
        if let Some(entry) = current(vault, &id)? {
            let overdue = entry.due_back.as_deref().is_some_and(|d| d < today);
            out.push(Away { entry, asset_name: name, overdue });
        }
    }
    out.sort_by(|a, b| {
        b.overdue.cmp(&a.overdue).then_with(|| {
            a.entry
                .due_back
                .as_deref()
                .unwrap_or("9999")
                .cmp(b.entry.due_back.as_deref().unwrap_or("9999"))
        })
    });
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
    use crate::assets::{create, get, NewAsset, Pricing};
    use am_core::Decimal;
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
                type_id: "instrument".into(),
                name: "Martin D-28".into(),
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

    fn handoff(
        asset: &str,
        kind: &str,
        date: &str,
        party: Option<&str>,
        due: Option<&str>,
    ) -> NewCustody {
        NewCustody {
            asset_id: asset.into(),
            kind: kind.into(),
            party: party.map(str::to_string),
            contact: None,
            date: date.into(),
            due_back: due.map(str::to_string),
            reference: None,
            note: String::new(),
            object_id: None,
        }
    }

    #[test]
    fn away_until_returned_and_overdue_after_its_due_date() {
        let (_d, v, id) = setup();
        record(
            &v,
            &handoff(&id, "repair", "2026-08-01", Some("Luthier"), Some("2026-09-01")),
            TODAY,
            NOW,
        )
        .unwrap();
        let out = away(&v, TODAY).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].overdue);
        assert_eq!(current(&v, &id).unwrap().unwrap().party.as_deref(), Some("Luthier"));

        record(&v, &handoff(&id, "returned", "2026-09-10", None, None), TODAY, NOW).unwrap();
        assert!(current(&v, &id).unwrap().is_none());
        assert!(away(&v, TODAY).unwrap().is_empty());
        assert_eq!(history(&v, &id).unwrap().len(), 2);
        assert_eq!(get(&v, &id).unwrap().status, "active", "ownership is untouched");
    }

    #[test]
    fn handoffs_are_validated() {
        let (_d, v, id) = setup();
        assert!(
            record(&v, &handoff(&id, "returned", "2026-09-01", None, None), TODAY, NOW)
                .is_err(),
            "not away"
        );
        assert!(
            record(&v, &handoff(&id, "lent", "2026-09-01", None, None), TODAY, NOW).is_err(),
            "to whom?"
        );
        assert!(
            record(&v, &handoff(&id, "lent", "2026-10-01", Some("Sam"), None), TODAY, NOW)
                .is_err(),
            "future"
        );
        assert!(record(
            &v,
            &handoff(&id, "lent", "2026-09-01", Some("Sam"), Some("2026-08-01")),
            TODAY,
            NOW
        )
        .is_err());
        record(&v, &handoff(&id, "lent", "2026-09-01", Some("Sam"), None), TODAY, NOW).unwrap();
        assert!(
            record(&v, &handoff(&id, "returned", "2026-08-01", None, None), TODAY, NOW)
                .is_err(),
            "before it went"
        );
    }
}
