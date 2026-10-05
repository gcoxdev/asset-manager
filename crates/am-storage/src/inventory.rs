//! Physical inventory checks, and the labels that speed them up.
//!
//! A label carries only an opaque code — the asset's random ID — so a
//! photographed or lost label reveals nothing about value, location or
//! serial. Scanning one during a check marks that item present.

use am_core::Decimal;
use serde::Serialize;

use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

fn invalid(m: impl Into<String>) -> InventoryError {
    InventoryError::Invalid(m.into())
}

/// What a label encodes: a fixed prefix and the asset's ID. Nothing else.
pub fn label_payload(asset_id: &str) -> String {
    format!("AM:{asset_id}")
}

/// The short code printed beside the QR, for typing when a scanner is not
/// to hand: the first eight characters of the ID.
pub fn short_code(asset_id: &str) -> String {
    asset_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect::<String>()
        .to_uppercase()
}

/// Find the asset a scanned label or typed short code names. Only assets
/// in the catalog; an ambiguous short code finds nothing.
pub fn resolve(vault: &Vault, scanned: &str) -> Result<Option<String>, InventoryError> {
    let text = scanned.trim();
    let id = text.strip_prefix("AM:").or_else(|| text.strip_prefix("am:")).unwrap_or(text);
    if id.len() >= 32 {
        let found: Option<String> = vault
            .conn()
            .query_row(
                "SELECT asset_id FROM assets WHERE asset_id = ?1 AND deleted_at IS NULL",
                [id],
                |r| r.get(0),
            )
            .ok();
        return Ok(found);
    }
    let code: String =
        id.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_lowercase();
    if code.len() != 8 {
        return Ok(None);
    }
    let mut stmt = vault.conn().prepare(
        "SELECT asset_id FROM assets WHERE deleted_at IS NULL AND substr(replace(asset_id, '-', ''), 1, 8) = ?1 LIMIT 2",
    )?;
    let ids: Vec<String> = stmt.query_map([&code], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(if ids.len() == 1 { ids.into_iter().next() } else { None })
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub check_id: String,
    pub name: String,
    pub scope_location: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub expected: i64,
    pub marked: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckItem {
    pub asset_id: String,
    pub name: String,
    pub type_label: String,
    pub storage_location: Option<String>,
    pub quantity: String,
    pub quantity_unit: String,
    /// Away with someone (lent, at repair…): expected not to be here.
    pub away: Option<String>,
    pub result: Option<String>,
    pub counted: Option<String>,
}

pub fn start(
    vault: &Vault,
    name: &str,
    scope_location: Option<&str>,
    now: &str,
) -> Result<String, InventoryError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(invalid("give the check a name"));
    }
    let id = new_id();
    vault.conn().execute(
        "INSERT INTO inventory_checks (check_id, name, scope_location, started_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![&id, name, scope_location.map(str::trim).filter(|s| !s.is_empty()), now],
    )?;
    Ok(id)
}

fn scope(
    vault: &Vault,
    check_id: &str,
) -> Result<(Option<String>, Option<String>), InventoryError> {
    vault
        .conn()
        .query_row(
            "SELECT scope_location, finished_at FROM inventory_checks WHERE check_id = ?1",
            [check_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| invalid("that check no longer exists"))
}

/// The items a check covers — held, in the catalog, at its location or in
/// places inside it — with what has been marked so far.
pub fn items(vault: &Vault, check_id: &str) -> Result<Vec<CheckItem>, InventoryError> {
    let (location, _) = scope(vault, check_id)?;
    let nested = location.as_ref().map(|l| format!("{l} / "));
    let mut stmt = vault.conn().prepare(
        "SELECT a.asset_id, a.name, t.display_name, a.storage_location, a.quantity, a.quantity_unit,
                (SELECT c.kind FROM custody_events c WHERE c.asset_id = a.asset_id
                  ORDER BY c.date DESC, c.recorded_at DESC LIMIT 1),
                m.result, m.counted
         FROM assets a JOIN asset_types t ON t.type_id = a.type_id
         LEFT JOIN inventory_marks m ON m.asset_id = a.asset_id AND m.check_id = ?1
         WHERE a.deleted_at IS NULL AND a.status = 'active'
           AND (?2 IS NULL OR a.storage_location = ?2 OR substr(a.storage_location, 1, length(?3)) = ?3)
         ORDER BY a.storage_location, a.name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![check_id, location, nested], |r| {
            let away: Option<String> = r.get(6)?;
            Ok(CheckItem {
                asset_id: r.get(0)?,
                name: r.get(1)?,
                type_label: r.get(2)?,
                storage_location: r.get(3)?,
                quantity: r.get(4)?,
                quantity_unit: r.get(5)?,
                away: away.filter(|k| k != "returned"),
                result: r.get(7)?,
                counted: r.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Mark an item: present, missing, or present in a different count. Marking
/// again replaces the earlier mark; `None` clears it.
pub fn mark(
    vault: &Vault,
    check_id: &str,
    asset_id: &str,
    result: Option<&str>,
    counted: Option<&str>,
    now: &str,
) -> Result<(), InventoryError> {
    let (_, finished) = scope(vault, check_id)?;
    if finished.is_some() {
        return Err(invalid("that check is finished"));
    }
    let Some(result) = result else {
        vault.conn().execute(
            "DELETE FROM inventory_marks WHERE check_id = ?1 AND asset_id = ?2",
            [check_id, asset_id],
        )?;
        return Ok(());
    };
    let counted = match result {
        "present" | "missing" => None,
        "count" => {
            let raw = counted
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .ok_or_else(|| invalid("enter how many are there"))?;
            let d = am_core::parse_decimal(raw)
                .map_err(|_| invalid("the count is not a number"))?;
            if d < Decimal::ZERO {
                return Err(invalid("the count cannot be negative"));
            }
            Some(d.normalize().to_string())
        }
        other => return Err(invalid(format!("unknown result: {other}"))),
    };
    vault.conn().execute(
        "INSERT INTO inventory_marks (check_id, asset_id, result, counted, marked_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(check_id, asset_id) DO UPDATE SET result = excluded.result, counted = excluded.counted,
           marked_at = excluded.marked_at",
        rusqlite::params![check_id, asset_id, result, counted, now],
    )?;
    Ok(())
}

pub fn finish(vault: &Vault, check_id: &str, now: &str) -> Result<(), InventoryError> {
    scope(vault, check_id)?;
    vault.conn().execute(
        "UPDATE inventory_checks SET finished_at = ?1 WHERE check_id = ?2 AND finished_at IS NULL",
        [now, check_id],
    )?;
    Ok(())
}

pub fn delete(vault: &Vault, check_id: &str) -> Result<(), InventoryError> {
    vault.conn().execute("DELETE FROM inventory_checks WHERE check_id = ?1", [check_id])?;
    Ok(())
}

/// Every check, open ones first.
pub fn list(vault: &Vault) -> Result<Vec<Check>, InventoryError> {
    let mut checks: Vec<Check> = {
        let mut stmt = vault.conn().prepare(
            "SELECT check_id, name, scope_location, started_at, finished_at FROM inventory_checks
             ORDER BY finished_at IS NOT NULL, started_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Check {
                    check_id: r.get(0)?,
                    name: r.get(1)?,
                    scope_location: r.get(2)?,
                    started_at: r.get(3)?,
                    finished_at: r.get(4)?,
                    expected: 0,
                    marked: 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for check in &mut checks {
        let all = items(vault, &check.check_id)?;
        check.expected = all.len() as i64;
        check.marked = all.iter().filter(|i| i.result.is_some()).count() as i64;
    }
    Ok(checks)
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

    fn asset(v: &Vault, name: &str, location: &str) -> String {
        create(
            v,
            &NewAsset {
                type_id: "generic".into(),
                name: name.into(),
                quantity: Decimal::from(3),
                quantity_unit: "item".into(),
                acquired_date: None,
                effective_date: None,
                acquired_cost: None,
                acquired_from: None,
                storage_location: Some(location.into()),
                notes: String::new(),
                insured: None,
                attrs: Default::default(),
                pricing: Pricing::Manual,
                review_every_days: None,
            },
            NOW,
        )
        .unwrap()
    }

    #[test]
    fn a_check_covers_its_location_and_records_each_result() {
        let (_d, v) = setup();
        let a = asset(&v, "Ring", "Safe");
        let b = asset(&v, "Coins", "Safe / Drawer");
        let _c = asset(&v, "Lamp", "Hall");
        let check = start(&v, "Safe, September", Some("Safe"), NOW).unwrap();
        assert_eq!(items(&v, &check).unwrap().len(), 2, "the safe and what is inside it");

        mark(&v, &check, &a, Some("present"), None, NOW).unwrap();
        mark(&v, &check, &b, Some("count"), Some("2"), NOW).unwrap();
        assert!(
            mark(&v, &check, &b, Some("count"), None, NOW).is_err(),
            "a count needs a number"
        );
        let listed = list(&v).unwrap();
        assert_eq!((listed[0].expected, listed[0].marked), (2, 2));
        finish(&v, &check, NOW).unwrap();
        assert!(mark(&v, &check, &a, Some("missing"), None, NOW).is_err(), "finished");
    }

    #[test]
    fn labels_carry_only_an_opaque_code_and_resolve_back() {
        let (_d, v) = setup();
        let a = asset(&v, "Ring", "Safe");
        let payload = label_payload(&a);
        assert!(!payload.contains("Ring") && !payload.contains("Safe"));
        assert_eq!(resolve(&v, &payload).unwrap().as_deref(), Some(a.as_str()));
        assert_eq!(resolve(&v, &short_code(&a)).unwrap().as_deref(), Some(a.as_str()));
        assert_eq!(resolve(&v, "AM:nonsense").unwrap(), None);
        crate::assets::trash(&v, &a, NOW).unwrap();
        assert_eq!(resolve(&v, &payload).unwrap(), None, "trashed items do not resolve");
    }
}
