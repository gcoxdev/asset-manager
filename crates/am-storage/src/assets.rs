//! Asset records: create, read, edit, delete, search.
//!
//! These used to be SQL strings inside Tauri commands, where nothing could
//! test them without a WebView. They live here so the rules they enforce are
//! asserted like the rest of the storage layer.
//!
//! Two rules worth stating up front:
//!
//! - **Quantity is never edited here.** It is a cache of the event log; the
//!   only way to change it is an event (`events::record`), so an edit form
//!   cannot make the history disagree with today.
//! - **Current value is never edited here either.** It is a cache of the
//!   valuations table. Setting a price means recording a valuation.

use std::collections::BTreeMap;
use std::path::Path;

use am_core::{parse_decimal, sort_key, Decimal, Money};
use serde::Serialize;

use crate::events::{normalize_date, EventError};
use crate::vault::Vault;

/// Cap on free-text fields, so a pasted document cannot bloat a row.
pub const MAX_TEXT_LEN: usize = 20_000;
pub const MAX_NAME_LEN: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("unknown asset: {0}")]
    UnknownAsset(String),
    #[error("unknown asset type: {0}")]
    UnknownType(String),
    #[error("an asset needs a name")]
    MissingName,
    #[error("{field} is too long (limit {max} characters)")]
    TooLong { field: &'static str, max: usize },
    #[error("{0:?} is not a valid quantity")]
    BadQuantity(String),
    #[error("quantity cannot be negative")]
    NegativeQuantity,
    #[error("{0}")]
    BadStatus(String),
    #[error("review interval must be a positive number of days")]
    BadReviewInterval,
    #[error(transparent)]
    Event(#[from] EventError),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Other(String),
}

/// Serialize minor units as text. JavaScript numbers are 53-bit, so an i64
/// crossing IPC as a number can corrupt silently.
fn minor_as_string<S: serde::Serializer>(v: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(n) => s.serialize_some(&n.to_string()),
        None => s.serialize_none(),
    }
}

/// Everything stored about one asset, plus its type metadata.
#[derive(Debug, Clone, Serialize)]
pub struct AssetRecord {
    pub asset_id: String,
    pub type_id: String,
    pub type_label: String,
    pub archetype: String,
    pub category: String,
    pub name: String,
    pub status: String,
    pub quantity: String,
    pub quantity_unit: String,
    pub acquired_date: Option<String>,
    #[serde(serialize_with = "minor_as_string")]
    pub acquired_amount_minor: Option<i64>,
    pub acquired_currency: Option<String>,
    /// False when part of the holding was added at an unknown cost, so the
    /// recorded cost covers only some of it. Gain is not computed then.
    pub cost_complete: bool,
    pub acquired_from: Option<String>,
    pub storage_location: Option<String>,
    pub notes: String,
    #[serde(serialize_with = "minor_as_string")]
    pub current_amount_minor: Option<i64>,
    pub current_currency: Option<String>,
    pub value_source: Option<String>,
    pub value_asof: Option<String>,
    #[serde(serialize_with = "minor_as_string")]
    pub insured_amount_minor: Option<i64>,
    pub insured_currency: Option<String>,
    pub sold_date: Option<String>,
    #[serde(serialize_with = "minor_as_string")]
    pub sold_amount_minor: Option<i64>,
    pub sold_currency: Option<String>,
    pub attrs: BTreeMap<String, String>,
    pub pricing: String,
    pub review_every_days: Option<i64>,
    pub primary_photo: Option<String>,
    pub photo_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

const SELECT: &str = "
    SELECT a.asset_id, a.type_id, t.display_name, t.archetype, t.category, a.name, a.status,
           a.quantity, a.quantity_unit, a.acquired_date, a.acquired_amount_minor,
           a.acquired_currency, a.acquired_from, a.storage_location, a.notes,
           a.current_amount_minor, a.current_currency, a.value_source, a.value_asof,
           a.insured_amount_minor, a.insured_currency, a.sold_date, a.sold_amount_minor,
           a.sold_currency, a.attrs, a.pricing, a.review_every_days,
           (SELECT m.object_id FROM asset_media m JOIN objects o ON o.object_id = m.object_id
             WHERE m.asset_id = a.asset_id AND o.gc_state = 'live'
             ORDER BY m.is_primary DESC, m.sort_order, m.created_at LIMIT 1),
           (SELECT count(*) FROM asset_media m JOIN objects o ON o.object_id = m.object_id
             WHERE m.asset_id = a.asset_id AND o.gc_state = 'live'),
           a.created_at, a.updated_at, a.cost_complete
    FROM assets a JOIN asset_types t ON t.type_id = a.type_id";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AssetRecord> {
    let attrs_json: String = r.get(24)?;
    // Values are strings by construction; anything else (a number written by
    // a future version) is kept as its JSON text rather than dropped.
    let attrs = match serde_json::from_str::<serde_json::Value>(&attrs_json) {
        Ok(serde_json::Value::Object(map)) => map
            .into_iter()
            .map(|(k, v)| {
                let text = match v {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                (k, text)
            })
            .collect(),
        _ => BTreeMap::new(),
    };

    Ok(AssetRecord {
        asset_id: r.get(0)?,
        type_id: r.get(1)?,
        type_label: r.get(2)?,
        archetype: r.get(3)?,
        category: r.get(4)?,
        name: r.get(5)?,
        status: r.get(6)?,
        quantity: r.get(7)?,
        quantity_unit: r.get(8)?,
        acquired_date: r.get(9)?,
        acquired_amount_minor: r.get(10)?,
        acquired_currency: r.get(11)?,
        cost_complete: r.get::<_, i64>(31)? != 0,
        acquired_from: r.get(12)?,
        storage_location: r.get(13)?,
        notes: r.get(14)?,
        current_amount_minor: r.get(15)?,
        current_currency: r.get(16)?,
        value_source: r.get(17)?,
        value_asof: r.get(18)?,
        insured_amount_minor: r.get(19)?,
        insured_currency: r.get(20)?,
        sold_date: r.get(21)?,
        sold_amount_minor: r.get(22)?,
        sold_currency: r.get(23)?,
        attrs,
        pricing: r.get(25)?,
        review_every_days: r.get(26)?,
        primary_photo: r.get(27)?,
        photo_count: r.get(28)?,
        created_at: r.get(29)?,
        updated_at: r.get(30)?,
    })
}

/// Every asset, most recently changed first.
pub fn list(vault: &Vault) -> Result<Vec<AssetRecord>, AssetError> {
    let mut stmt = vault.conn().prepare(&format!("{SELECT} ORDER BY a.updated_at DESC"))?;
    let rows = stmt.query_map([], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(vault: &Vault, asset_id: &str) -> Result<AssetRecord, AssetError> {
    vault
        .conn()
        .query_row(&format!("{SELECT} WHERE a.asset_id = ?1"), [asset_id], from_row)
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => AssetError::UnknownAsset(asset_id.into()),
            other => other.into(),
        })
}

/// A type the catalog knows.
#[derive(Debug, Clone, Serialize)]
pub struct AssetType {
    pub type_id: String,
    pub label: String,
    pub archetype: String,
    pub category: String,
}

pub fn types(vault: &Vault) -> Result<Vec<AssetType>, AssetError> {
    let mut stmt = vault.conn().prepare(
        "SELECT type_id, display_name, archetype, category FROM asset_types
         ORDER BY category, display_name",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(AssetType {
                type_id: r.get(0)?,
                label: r.get(1)?,
                archetype: r.get(2)?,
                category: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn check_type(vault: &Vault, type_id: &str) -> Result<(), AssetError> {
    let n: i64 = vault.conn().query_row(
        "SELECT count(*) FROM asset_types WHERE type_id = ?1",
        [type_id],
        |r| r.get(0),
    )?;
    if n == 0 {
        return Err(AssetError::UnknownType(type_id.to_string()));
    }
    Ok(())
}

fn clean_name(name: &str) -> Result<String, AssetError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AssetError::MissingName);
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(AssetError::TooLong { field: "name", max: MAX_NAME_LEN });
    }
    Ok(name.to_string())
}

/// Trim, drop empties to `None`, and bound length.
fn clean_optional(
    value: &Option<String>,
    field: &'static str,
) -> Result<Option<String>, AssetError> {
    let Some(v) = value else { return Ok(None) };
    let v = v.trim();
    if v.is_empty() {
        return Ok(None);
    }
    if v.chars().count() > MAX_TEXT_LEN {
        return Err(AssetError::TooLong { field, max: MAX_TEXT_LEN });
    }
    Ok(Some(v.to_string()))
}

fn clean_notes(notes: &str) -> Result<String, AssetError> {
    if notes.chars().count() > MAX_TEXT_LEN {
        return Err(AssetError::TooLong { field: "notes", max: MAX_TEXT_LEN });
    }
    Ok(notes.trim().to_string())
}

fn clean_review(days: Option<i64>) -> Result<Option<i64>, AssetError> {
    match days {
        Some(d) if d <= 0 => Err(AssetError::BadReviewInterval),
        other => Ok(other),
    }
}

fn attrs_json(attrs: &BTreeMap<String, String>) -> Result<String, AssetError> {
    serde_json::to_string(attrs).map_err(|e| AssetError::Other(e.to_string()))
}

fn split_money(m: &Option<Money>) -> (Option<i64>, Option<String>) {
    match m {
        Some(m) => (Some(m.amount_minor), Some(m.currency.code().to_string())),
        None => (None, None),
    }
}

/// How an asset's value is determined. See migration 003.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pricing {
    /// Valued by hand only.
    Manual,
    /// Revalued from stored quotes — spot metal or coin prices.
    Market,
}

impl Pricing {
    pub fn as_str(self) -> &'static str {
        match self {
            Pricing::Manual => "manual",
            Pricing::Market => "market",
        }
    }
}

/// A new asset.
#[derive(Debug, Clone)]
pub struct NewAsset {
    pub type_id: String,
    pub name: String,
    pub quantity: Decimal,
    pub quantity_unit: String,
    /// When it was acquired. Also the effective date of the acquire event,
    /// so the chart shows it held from then. Defaults to today.
    pub acquired_date: Option<String>,
    /// What the whole position cost.
    pub acquired_cost: Option<Money>,
    pub acquired_from: Option<String>,
    pub storage_location: Option<String>,
    pub notes: String,
    pub insured: Option<Money>,
    pub attrs: BTreeMap<String, String>,
    pub pricing: Pricing,
    pub review_every_days: Option<i64>,
}

/// Create an asset and its acquire event, atomically.
pub fn create(vault: &Vault, asset: &NewAsset, now: &str) -> Result<String, AssetError> {
    check_type(vault, &asset.type_id)?;
    let name = clean_name(&asset.name)?;
    if asset.quantity.is_sign_negative() {
        return Err(AssetError::NegativeQuantity);
    }
    let acquired_date = match &asset.acquired_date {
        Some(d) if !d.trim().is_empty() => Some(normalize_date(d)?),
        _ => None,
    };
    let effective = acquired_date.clone().unwrap_or_else(|| now[..10].to_string());
    let (cost_minor, cost_currency) = split_money(&asset.acquired_cost);
    let (insured_minor, insured_currency) = split_money(&asset.insured);
    let unit = match asset.quantity_unit.trim() {
        "" => "item".to_string(),
        u => u.to_string(),
    };

    let asset_id = uuid_v4();
    let tx = vault.conn().unchecked_transaction()?;
    tx.execute(
        "INSERT INTO assets
           (asset_id, type_id, name, quantity, quantity_sort, quantity_unit,
            acquired_date, acquired_amount_minor, acquired_currency, acquired_from,
            storage_location, notes, insured_amount_minor, insured_currency,
            attrs, pricing, review_every_days, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?18)",
        rusqlite::params![
            &asset_id,
            &asset.type_id,
            &name,
            asset.quantity.to_string(),
            sort_key(asset.quantity),
            unit,
            acquired_date,
            cost_minor,
            cost_currency,
            clean_optional(&asset.acquired_from, "acquired from")?,
            clean_optional(&asset.storage_location, "storage location")?,
            clean_notes(&asset.notes)?,
            insured_minor,
            insured_currency,
            attrs_json(&asset.attrs)?,
            asset.pricing.as_str(),
            clean_review(asset.review_every_days)?,
            now,
        ],
    )?;

    // Ownership starts as an event, so history is uniform however the asset
    // was created. The date is a plain date: see `events::normalize_date`.
    tx.execute(
        "INSERT INTO asset_events
           (event_id, asset_id, event_type, effective_date, quantity_delta,
            amount_minor, currency, recorded_at)
         VALUES (?1, ?2, 'acquire', ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            uuid_v4(),
            &asset_id,
            effective,
            asset.quantity.to_string(),
            cost_minor,
            cost_currency,
            now
        ],
    )?;
    tx.commit()?;
    Ok(asset_id)
}

/// The editable fields of an asset, replaced as a whole.
///
/// Whole-record rather than a patch: the edit form always has every field,
/// and a patch format would need a third state for "clear" on every one.
#[derive(Debug, Clone)]
pub struct AssetEdit {
    pub type_id: String,
    pub name: String,
    pub status: String,
    pub quantity_unit: String,
    pub acquired_date: Option<String>,
    pub acquired_cost: Option<Money>,
    pub acquired_from: Option<String>,
    pub storage_location: Option<String>,
    pub notes: String,
    pub insured: Option<Money>,
    pub attrs: BTreeMap<String, String>,
    pub review_every_days: Option<i64>,
    /// The owner confirms the cost covers everything held, even though the
    /// figure did not change — the unpriced units were a gift, say.
    pub cost_covers_holding: bool,
}

/// Apply an edit.
///
/// Status is constrained: `sold` is what the event log says after a full
/// disposal, so it cannot be set or unset by hand — that would make status
/// and quantity disagree. Lost and retired are the owner's to set.
///
/// Changing the acquisition date moves the acquire event with it, so the
/// chart shows the asset held from the corrected date.
pub fn update(
    vault: &Vault,
    asset_id: &str,
    edit: &AssetEdit,
    now: &str,
) -> Result<(), AssetError> {
    let current = get(vault, asset_id)?;
    check_type(vault, &edit.type_id)?;
    let name = clean_name(&edit.name)?;

    let status = match (current.status.as_str(), edit.status.as_str()) {
        ("sold", "sold") => "sold",
        ("sold", _) => {
            return Err(AssetError::BadStatus(
                "this asset was sold — record a purchase to hold it again".into(),
            ))
        }
        (_, "sold") => {
            return Err(AssetError::BadStatus(
                "record a sale instead, so the history shows when it left".into(),
            ))
        }
        (_, s @ ("active" | "lost" | "retired")) => s,
        (_, other) => return Err(AssetError::BadStatus(format!("unknown status: {other}"))),
    };

    let acquired_date = match &edit.acquired_date {
        Some(d) if !d.trim().is_empty() => Some(normalize_date(d)?),
        _ => None,
    };
    let (cost_minor, cost_currency) = split_money(&edit.acquired_cost);
    let (insured_minor, insured_currency) = split_money(&edit.insured);
    let unit = match edit.quantity_unit.trim() {
        "" => "item".to_string(),
        u => u.to_string(),
    };

    let tx = vault.conn().unchecked_transaction()?;
    tx.execute(
        "UPDATE assets SET
           type_id = ?1, name = ?2, status = ?3, quantity_unit = ?4, acquired_date = ?5,
           acquired_amount_minor = ?6, acquired_currency = ?7, acquired_from = ?8,
           storage_location = ?9, notes = ?10, insured_amount_minor = ?11,
           insured_currency = ?12, attrs = ?13, review_every_days = ?14, updated_at = ?15
         WHERE asset_id = ?16",
        rusqlite::params![
            &edit.type_id,
            &name,
            status,
            unit,
            &acquired_date,
            cost_minor,
            cost_currency,
            clean_optional(&edit.acquired_from, "acquired from")?,
            clean_optional(&edit.storage_location, "storage location")?,
            clean_notes(&edit.notes)?,
            insured_minor,
            insured_currency,
            attrs_json(&edit.attrs)?,
            clean_review(edit.review_every_days)?,
            now,
            asset_id,
        ],
    )?;

    // Restating the total paid is how a partial cost is reconciled: the
    // owner has now given the cost of the whole position. Saving the same
    // figure again — a notes-only edit — leaves a partial cost partial.
    let cost_changed = cost_minor != current.acquired_amount_minor
        || cost_currency.as_deref() != current.acquired_currency.as_deref();
    if cost_changed || edit.cost_covers_holding {
        tx.execute("UPDATE assets SET cost_complete = 1 WHERE asset_id = ?1", [asset_id])?;
    }

    if let Some(date) = &acquired_date {
        if current.acquired_date.as_deref() != Some(date.as_str()) {
            crate::events::check_acquisition_move(&tx, asset_id, date)?;
            tx.execute(
                "UPDATE asset_events SET effective_date = ?1
                 WHERE asset_id = ?2 AND event_type = 'acquire'",
                rusqlite::params![date, asset_id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Switch between market and manual pricing.
pub fn set_pricing(
    vault: &Vault,
    asset_id: &str,
    pricing: Pricing,
    now: &str,
) -> Result<(), AssetError> {
    let n = vault.conn().execute(
        "UPDATE assets SET pricing = ?1, updated_at = ?2 WHERE asset_id = ?3",
        rusqlite::params![pricing.as_str(), now, asset_id],
    )?;
    if n == 0 {
        return Err(AssetError::UnknownAsset(asset_id.into()));
    }
    Ok(())
}

/// Delete an asset outright, with its history and photos.
///
/// For records entered by mistake. Something sold should be *disposed* —
/// that keeps its past contribution to the chart; deleting erases it.
///
/// Photos are detached first so shared objects keep their other references,
/// then anything left unreferenced is swept from disk.
pub fn delete(vault: &Vault, root: &Path, asset_id: &str) -> Result<(), AssetError> {
    get(vault, asset_id)?;

    let objects: Vec<String> = {
        let mut stmt =
            vault.conn().prepare("SELECT object_id FROM asset_media WHERE asset_id = ?1")?;
        let rows = stmt.query_map([asset_id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for object_id in &objects {
        crate::objects::detach_from_asset(vault, asset_id, object_id)
            .map_err(|e| AssetError::Other(e.to_string()))?;
    }

    // Events, valuations and media rows cascade.
    vault.conn().execute("DELETE FROM assets WHERE asset_id = ?1", [asset_id])?;
    crate::objects::sweep_deleted(vault, root).map_err(|e| AssetError::Other(e.to_string()))?;
    Ok(())
}

/// Make one photo the asset's primary.
pub fn set_primary_photo(
    vault: &Vault,
    asset_id: &str,
    object_id: &str,
) -> Result<(), AssetError> {
    let tx = vault.conn().unchecked_transaction()?;
    let attached: i64 = tx.query_row(
        "SELECT count(*) FROM asset_media WHERE asset_id = ?1 AND object_id = ?2",
        [asset_id, object_id],
        |r| r.get(0),
    )?;
    if attached == 0 {
        return Err(AssetError::Other("that photo is not attached to this asset".into()));
    }
    // Clear first: a unique index allows one primary per asset.
    tx.execute("UPDATE asset_media SET is_primary = 0 WHERE asset_id = ?1", [asset_id])?;
    tx.execute(
        "UPDATE asset_media SET is_primary = 1 WHERE asset_id = ?1 AND object_id = ?2",
        [asset_id, object_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Turn what someone typed into a safe FTS5 query.
///
/// Raw input reaches FTS5's query grammar, where `#15`, `PSA-10` or an
/// unbalanced quote is a syntax error rather than a search. Each run of
/// letters and digits becomes a quoted prefix term, and the terms are ANDed —
/// which is what a search box is expected to do. `None` means there was
/// nothing searchable.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(16)
        .map(|t| format!("\"{t}\"*"))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// Asset IDs matching a search, best match first.
pub fn search(vault: &Vault, input: &str) -> Result<Vec<String>, AssetError> {
    let Some(query) = fts_query(input) else { return Ok(Vec::new()) };
    let mut stmt = vault.conn().prepare(
        "SELECT a.asset_id FROM assets_fts f JOIN assets a ON a.rowid = f.rowid
         WHERE assets_fts MATCH ?1 ORDER BY rank LIMIT 1000",
    )?;
    let rows = stmt.query_map([query], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Parse a quantity typed by a person.
pub fn parse_quantity(text: &str) -> Result<Decimal, AssetError> {
    let q = parse_decimal(text).map_err(|_| AssetError::BadQuantity(text.to_string()))?;
    if q.is_sign_negative() {
        return Err(AssetError::NegativeQuantity);
    }
    Ok(q)
}

fn uuid_v4() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw[6] = (raw[6] & 0x0f) | 0x40;
    raw[8] = (raw[8] & 0x3f) | 0x80;
    let h: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{self, EventType, NewEvent};
    use am_core::Currency;
    use am_crypto::KdfParams;

    const PASS: &str = "correct horse battery staple";
    const NOW: &str = "2026-09-19T10:00:00Z";

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    fn setup() -> (tempfile::TempDir, std::path::PathBuf, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        (dir, root, vault)
    }

    fn usd(minor: i64) -> Money {
        Money::new(minor, Currency::new("USD").unwrap())
    }

    fn new_asset(name: &str) -> NewAsset {
        NewAsset {
            type_id: "generic".into(),
            name: name.into(),
            quantity: Decimal::ONE,
            quantity_unit: "item".into(),
            acquired_date: None,
            acquired_cost: None,
            acquired_from: None,
            storage_location: None,
            notes: String::new(),
            insured: None,
            attrs: BTreeMap::new(),
            pricing: Pricing::Manual,
            review_every_days: None,
        }
    }

    fn edit_of(r: &AssetRecord) -> AssetEdit {
        AssetEdit {
            type_id: r.type_id.clone(),
            name: r.name.clone(),
            status: r.status.clone(),
            quantity_unit: r.quantity_unit.clone(),
            acquired_date: r.acquired_date.clone(),
            acquired_cost: None,
            acquired_from: r.acquired_from.clone(),
            storage_location: r.storage_location.clone(),
            notes: r.notes.clone(),
            insured: None,
            attrs: r.attrs.clone(),
            review_every_days: r.review_every_days,
            cost_covers_holding: false,
        }
    }

    #[test]
    fn a_partial_cost_is_completed_by_restating_or_confirming_it() {
        let (_d, _root, v) = setup();
        let mut a = new_asset("Coins");
        a.acquired_cost = Some(usd(10_000));
        let id = create(&v, &a, NOW).unwrap();
        let partial = || {
            v.conn().execute("UPDATE assets SET cost_complete = 0", []).unwrap();
        };

        // Saving the same figure — a notes-only edit — leaves it partial.
        partial();
        let mut edit = edit_of(&get(&v, &id).unwrap());
        edit.acquired_cost = Some(usd(10_000));
        edit.notes = "just a note".into();
        update(&v, &id, &edit, NOW).unwrap();
        assert!(!get(&v, &id).unwrap().cost_complete);

        // A restated total completes it.
        edit.acquired_cost = Some(usd(14_000));
        update(&v, &id, &edit, NOW).unwrap();
        assert!(get(&v, &id).unwrap().cost_complete);

        // So does confirming the unchanged figure covers everything.
        partial();
        edit.cost_covers_holding = true;
        update(&v, &id, &edit, NOW).unwrap();
        assert!(get(&v, &id).unwrap().cost_complete);
    }

    #[test]
    fn create_records_the_asset_and_its_acquisition() {
        let (_d, _root, v) = setup();
        let mut a = new_asset("  Rolex Submariner  ");
        a.type_id = "watch".into();
        a.acquired_date = Some("2024-03-02".into());
        a.acquired_cost = Some(usd(850_000));
        a.storage_location = Some("Safe".into());
        let id = create(&v, &a, NOW).unwrap();

        let r = get(&v, &id).unwrap();
        assert_eq!(r.name, "Rolex Submariner", "names are trimmed");
        assert_eq!(r.type_label, "Watch");
        assert_eq!(r.category, "valuables");
        assert_eq!(r.acquired_amount_minor, Some(850_000));
        assert_eq!(r.acquired_currency.as_deref(), Some("USD"));

        let history = events::history(&v, &id).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].effective_date, "2024-03-02", "held from when it was bought");
        assert_eq!(history[0].amount_minor, Some(850_000));
    }

    #[test]
    fn an_acquire_without_a_date_is_dated_today_as_a_plain_date() {
        let (_d, _root, v) = setup();
        let id = create(&v, &new_asset("Thing"), NOW).unwrap();
        assert_eq!(events::history(&v, &id).unwrap()[0].effective_date, "2026-09-19");
    }

    #[test]
    fn create_refuses_bad_input() {
        let (_d, _root, v) = setup();
        assert!(matches!(create(&v, &new_asset("   "), NOW), Err(AssetError::MissingName)));

        let mut bad_type = new_asset("x");
        bad_type.type_id = "spaceship".into();
        assert!(matches!(create(&v, &bad_type, NOW), Err(AssetError::UnknownType(_))));

        let mut negative = new_asset("x");
        negative.quantity = Decimal::NEGATIVE_ONE;
        assert!(matches!(create(&v, &negative, NOW), Err(AssetError::NegativeQuantity)));

        let mut bad_date = new_asset("x");
        bad_date.acquired_date = Some("last tuesday".into());
        assert!(create(&v, &bad_date, NOW).is_err());

        let mut bad_review = new_asset("x");
        bad_review.review_every_days = Some(0);
        assert!(matches!(create(&v, &bad_review, NOW), Err(AssetError::BadReviewInterval)));
    }

    #[test]
    fn editing_the_acquired_date_moves_the_acquire_event() {
        let (_d, _root, v) = setup();
        let id = create(&v, &new_asset("Painting"), NOW).unwrap();

        let mut edit = edit_of(&get(&v, &id).unwrap());
        edit.acquired_date = Some("2020-05-01".into());
        update(&v, &id, &edit, NOW).unwrap();

        assert_eq!(events::history(&v, &id).unwrap()[0].effective_date, "2020-05-01");
        assert_eq!(
            events::quantity_as_of(&v, &id, Some("2021-01-01")).unwrap(),
            Decimal::ONE,
            "held in 2021 once the purchase date says so"
        );
    }

    #[test]
    fn sold_is_set_by_the_event_log_not_by_hand() {
        let (_d, _root, v) = setup();
        let id = create(&v, &new_asset("Card"), NOW).unwrap();

        let mut edit = edit_of(&get(&v, &id).unwrap());
        edit.status = "sold".into();
        assert!(matches!(update(&v, &id, &edit, NOW), Err(AssetError::BadStatus(_))));

        edit.status = "lost".into();
        update(&v, &id, &edit, NOW).unwrap();
        assert_eq!(get(&v, &id).unwrap().status, "lost");

        // Once disposed, the status belongs to the log.
        edit.status = "active".into();
        update(&v, &id, &edit, NOW).unwrap();
        events::record(
            &v,
            &NewEvent {
                asset_id: id.clone(),
                event_type: EventType::Dispose,
                effective_date: "2026-09-19".into(),
                quantity_delta: Decimal::NEGATIVE_ONE,
                amount_minor: None,
                currency: None,
                note: String::new(),
            },
            NOW,
        )
        .unwrap();
        let mut sold = edit_of(&get(&v, &id).unwrap());
        assert_eq!(sold.status, "sold");
        sold.status = "active".into();
        assert!(matches!(update(&v, &id, &sold, NOW), Err(AssetError::BadStatus(_))));
        sold.status = "sold".into();
        sold.notes = "Sold at the show".into();
        update(&v, &id, &sold, NOW).unwrap();
    }

    #[test]
    fn editing_never_touches_quantity_or_value() {
        let (_d, _root, v) = setup();
        let mut a = new_asset("Silver");
        a.quantity = Decimal::from(10);
        let id = create(&v, &a, NOW).unwrap();

        let mut edit = edit_of(&get(&v, &id).unwrap());
        edit.name = "Silver Eagles".into();
        update(&v, &id, &edit, NOW).unwrap();

        let r = get(&v, &id).unwrap();
        assert_eq!(r.quantity, "10");
        assert_eq!(r.current_amount_minor, None);
    }

    #[test]
    fn search_tolerates_punctuation_and_finds_attributes() {
        let (_d, _root, v) = setup();
        let mut comic = new_asset("Amazing Fantasy #15 — CGC 9.8");
        comic.type_id = "comic".into();
        comic.attrs.insert("cert_number".into(), "0012345".into());
        let comic_id = create(&v, &comic, NOW).unwrap();
        create(&v, &new_asset("Something else"), NOW).unwrap();

        // Each of these would be an FTS5 syntax error if passed through raw.
        for query in ["#15", "amaz", "\"Fantasy", "CGC-9.8", "0012345", "fant*"] {
            let hits = search(&v, query).unwrap();
            assert_eq!(hits, vec![comic_id.clone()], "query {query:?}");
        }
        assert!(search(&v, "!!!").unwrap().is_empty(), "nothing searchable");
        assert!(search(&v, "nonexistent").unwrap().is_empty());
    }

    #[test]
    fn fts_query_quotes_each_term() {
        assert_eq!(fts_query("psa 10").as_deref(), Some("\"psa\"* \"10\"*"));
        assert_eq!(fts_query("a\"b").as_deref(), Some("\"a\"* \"b\"*"));
        assert_eq!(fts_query("  -- "), None);
    }

    #[test]
    fn delete_removes_history_and_unshared_photos_only() {
        let (_d, root, v) = setup();
        let a = create(&v, &new_asset("A"), NOW).unwrap();
        let b = create(&v, &new_asset("B"), NOW).unwrap();

        // One photo shared by both, one belonging only to A.
        let shared = crate::objects::import_object(&v, &root, &png(1), NOW).unwrap();
        let only_a = crate::objects::import_object(&v, &root, &png(2), NOW).unwrap();
        crate::objects::attach_to_asset(&v, &a, &shared.object_id, NOW).unwrap();
        crate::objects::attach_to_asset(&v, &b, &shared.object_id, NOW).unwrap();
        crate::objects::attach_to_asset(&v, &a, &only_a.object_id, NOW).unwrap();

        delete(&v, &root, &a).unwrap();

        assert!(matches!(get(&v, &a), Err(AssetError::UnknownAsset(_))));
        let events: i64 = v
            .conn()
            .query_row("SELECT count(*) FROM asset_events WHERE asset_id = ?1", [&a], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(events, 0, "history goes with it");

        assert!(
            crate::objects::load_object(&v, &root, &shared.object_id).is_ok(),
            "B still uses the shared photo"
        );
        assert!(
            crate::objects::load_object(&v, &root, &only_a.object_id).is_err(),
            "A's own photo is swept"
        );
        assert_eq!(get(&v, &b).unwrap().photo_count, 1);
    }

    #[test]
    fn primary_photo_can_be_chosen() {
        let (_d, root, v) = setup();
        let id = create(&v, &new_asset("A"), NOW).unwrap();
        let first = crate::objects::import_object(&v, &root, &png(1), NOW).unwrap();
        let second = crate::objects::import_object(&v, &root, &png(2), NOW).unwrap();
        crate::objects::attach_to_asset(&v, &id, &first.object_id, NOW).unwrap();
        crate::objects::attach_to_asset(&v, &id, &second.object_id, NOW).unwrap();

        assert_eq!(get(&v, &id).unwrap().primary_photo, Some(first.object_id.clone()));
        set_primary_photo(&v, &id, &second.object_id).unwrap();
        assert_eq!(get(&v, &id).unwrap().primary_photo, Some(second.object_id.clone()));

        assert!(set_primary_photo(&v, &id, "0123456789abcdef0123456789abcdef").is_err());
    }

    /// A tiny valid PNG whose pixel differs by `seed`, so hashes differ.
    fn png(seed: u8) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([seed, 0, 0]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }
}
