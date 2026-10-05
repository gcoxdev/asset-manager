//! Things wanted. Never assets, never in a total.

use am_core::Money;
use serde::{Deserialize, Serialize};

use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum WishError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Deserialize)]
pub struct WishInput {
    #[serde(default)]
    pub wish_id: Option<String>,
    pub name: String,
    pub type_id: String,
    #[serde(skip)]
    pub target: Option<Money>,
    #[serde(default)]
    pub quantity: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Wish {
    pub wish_id: String,
    pub name: String,
    pub type_id: String,
    pub type_label: String,
    pub category: String,
    pub target_minor: Option<i64>,
    pub currency: Option<String>,
    pub quantity: String,
    pub priority: String,
    pub notes: String,
    pub acquired_asset_id: Option<String>,
    pub acquired_at: Option<String>,
    pub created_at: String,
    /// Items already in the catalog with the same name — "you may already
    /// own this". A suggestion, never proof.
    pub owned_matches: Vec<OwnedMatch>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnedMatch {
    pub asset_id: String,
    pub name: String,
}

/// Create or update a wish. Returns its ID.
pub fn save(vault: &Vault, input: &WishInput, now: &str) -> Result<String, WishError> {
    let invalid = |m: &str| WishError::Invalid(m.to_string());
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 300 {
        return Err(invalid("a wish needs a name"));
    }
    let known: i64 = vault.conn().query_row(
        "SELECT count(*) FROM asset_types WHERE type_id = ?1",
        [&input.type_id],
        |r| r.get(0),
    )?;
    if known == 0 {
        return Err(invalid("unknown type"));
    }
    let quantity = match input.quantity.as_deref().map(str::trim) {
        None | Some("") => "1".to_string(),
        Some(q) => {
            let d = am_core::parse_decimal(q)
                .map_err(|_| invalid("the quantity is not a number"))?;
            if d <= am_core::Decimal::ZERO {
                return Err(invalid("the quantity must be more than zero"));
            }
            d.normalize().to_string()
        }
    };
    let priority = input.priority.as_deref().unwrap_or("normal");
    if !["low", "normal", "high"].contains(&priority) {
        return Err(invalid("priority is low, normal or high"));
    }
    if input.notes.chars().count() > 5_000 {
        return Err(invalid("the notes are too long"));
    }
    let (minor, currency) = match &input.target {
        Some(m) => (Some(m.amount_minor), Some(m.currency.code().to_string())),
        None => (None, None),
    };
    match &input.wish_id {
        Some(id) => {
            let n = vault.conn().execute(
                "UPDATE wishes SET name = ?1, type_id = ?2, target_minor = ?3, currency = ?4,
                   quantity = ?5, priority = ?6, notes = ?7, updated_at = ?8 WHERE wish_id = ?9",
                rusqlite::params![name, &input.type_id, minor, currency, quantity, priority, input.notes.trim(), now, id],
            )?;
            if n == 0 {
                return Err(invalid("that wish no longer exists"));
            }
            Ok(id.clone())
        }
        None => {
            let id = new_id();
            vault.conn().execute(
                "INSERT INTO wishes (wish_id, name, type_id, target_minor, currency, quantity,
                   priority, notes, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                rusqlite::params![
                    &id,
                    name,
                    &input.type_id,
                    minor,
                    currency,
                    quantity,
                    priority,
                    input.notes.trim(),
                    now
                ],
            )?;
            Ok(id)
        }
    }
}

pub fn delete(vault: &Vault, wish_id: &str) -> Result<(), WishError> {
    let n = vault.conn().execute("DELETE FROM wishes WHERE wish_id = ?1", [wish_id])?;
    if n == 0 {
        return Err(WishError::Invalid("that wish no longer exists".into()));
    }
    Ok(())
}

/// Record that a wish was bought, as the given asset.
pub fn mark_acquired(
    vault: &Vault,
    wish_id: &str,
    asset_id: &str,
    now: &str,
) -> Result<(), WishError> {
    let exists: i64 = vault.conn().query_row(
        "SELECT count(*) FROM assets WHERE asset_id = ?1",
        [asset_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(WishError::Invalid("unknown asset".into()));
    }
    let n = vault.conn().execute(
        "UPDATE wishes SET acquired_asset_id = ?1, acquired_at = ?2, updated_at = ?2 WHERE wish_id = ?3",
        rusqlite::params![asset_id, now, wish_id],
    )?;
    if n == 0 {
        return Err(WishError::Invalid("that wish no longer exists".into()));
    }
    Ok(())
}

/// Every wish, wanted ones first (by priority, then newest), then got ones.
pub fn list(vault: &Vault) -> Result<Vec<Wish>, WishError> {
    let mut stmt = vault.conn().prepare(
        "SELECT w.wish_id, w.name, w.type_id, t.display_name, t.category, w.target_minor,
                w.currency, w.quantity, w.priority, w.notes, w.acquired_asset_id, w.acquired_at,
                w.created_at
         FROM wishes w JOIN asset_types t ON t.type_id = w.type_id
         ORDER BY w.acquired_at IS NOT NULL, CASE w.priority WHEN 'high' THEN 0 WHEN 'normal' THEN 1 ELSE 2 END,
                  w.created_at DESC",
    )?;
    let mut wishes = stmt
        .query_map([], |r| {
            Ok(Wish {
                wish_id: r.get(0)?,
                name: r.get(1)?,
                type_id: r.get(2)?,
                type_label: r.get(3)?,
                category: r.get(4)?,
                target_minor: r.get(5)?,
                currency: r.get(6)?,
                quantity: r.get(7)?,
                priority: r.get(8)?,
                notes: r.get(9)?,
                acquired_asset_id: r.get(10)?,
                acquired_at: r.get(11)?,
                created_at: r.get(12)?,
                owned_matches: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    let mut owned = vault.conn().prepare(
        "SELECT asset_id, name FROM assets
         WHERE deleted_at IS NULL AND status = 'active' AND lower(name) = lower(?1) LIMIT 5",
    )?;
    for wish in wishes.iter_mut().filter(|w| w.acquired_asset_id.is_none()) {
        wish.owned_matches = owned
            .query_map([&wish.name], |r| {
                Ok(OwnedMatch { asset_id: r.get(0)?, name: r.get(1)? })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(wishes)
}

fn new_id() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_crypto::KdfParams;

    const NOW: &str = "2026-09-19T00:00:00Z";

    fn setup() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let fast = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
        let (v, _r) =
            Vault::create(&dir.path().join("v"), "correct horse battery staple", &fast, NOW)
                .unwrap();
        (dir, v)
    }

    fn wish(name: &str) -> WishInput {
        WishInput {
            wish_id: None,
            name: name.into(),
            type_id: "watch".into(),
            target: Some(Money::new(500_000, am_core::Currency::new("USD").unwrap())),
            quantity: None,
            priority: Some("high".into()),
            notes: String::new(),
        }
    }

    #[test]
    fn a_wish_never_counts_and_is_marked_when_bought() {
        let (_d, v) = setup();
        let id = save(&v, &wish("Speedmaster"), NOW).unwrap();
        let d = crate::summary::dashboard(
            &v,
            &am_core::Currency::new("USD").unwrap(),
            "2026-09-19",
        )
        .unwrap();
        assert_eq!((d.active_count, d.total.minor), (0, 0), "wanted is not owned");

        v.conn()
            .execute("INSERT INTO assets (asset_id, type_id, name, created_at, updated_at) VALUES ('a','watch','speedmaster',?1,?1)", [NOW])
            .unwrap();
        assert_eq!(list(&v).unwrap()[0].owned_matches.len(), 1, "you may already own this");

        mark_acquired(&v, &id, "a", NOW).unwrap();
        let w = &list(&v).unwrap()[0];
        assert_eq!(w.acquired_asset_id.as_deref(), Some("a"));
        assert!(w.owned_matches.is_empty(), "got ones are not flagged");
    }

    #[test]
    fn wishes_are_validated() {
        let (_d, v) = setup();
        let mut bad = wish(" ");
        assert!(save(&v, &bad, NOW).is_err());
        bad = wish("x");
        bad.type_id = "spaceship".into();
        assert!(save(&v, &bad, NOW).is_err());
        bad = wish("x");
        bad.quantity = Some("0".into());
        assert!(save(&v, &bad, NOW).is_err());
        bad = wish("x");
        bad.priority = Some("urgent".into());
        assert!(save(&v, &bad, NOW).is_err());
    }
}
