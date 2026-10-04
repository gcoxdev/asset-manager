//! Finding and arranging things: tags, locations, edits to many assets at
//! once, duplicates, and saved views.
//!
//! Bulk edits go through the same revision history as a single edit, so a
//! change to two hundred items can still be undone item by item.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::assets::{self, AssetError};
use crate::vault::Vault;

/// A tag and how many assets in the catalog carry it.
#[derive(Debug, Clone, Serialize)]
pub struct TagCount {
    pub name: String,
    pub count: i64,
}

/// Longest tag name accepted.
pub const MAX_TAG_LEN: usize = 60;

fn clean_tag(name: &str) -> Result<String, AssetError> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(AssetError::Other("a tag needs a name".into()));
    }
    if name.chars().count() > MAX_TAG_LEN {
        return Err(AssetError::TooLong { field: "tag", max: MAX_TAG_LEN });
    }
    Ok(name)
}

/// Every tag in use, alphabetically, with counts over the catalog (trashed
/// assets not counted).
pub fn tags(vault: &Vault) -> Result<Vec<TagCount>, AssetError> {
    let mut stmt = vault.conn().prepare(
        "SELECT t.name, count(a.asset_id) FROM tags t
         LEFT JOIN asset_tags at ON at.tag_id = t.tag_id
         LEFT JOIN assets a ON a.asset_id = at.asset_id AND a.deleted_at IS NULL
         GROUP BY t.tag_id ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |r| Ok(TagCount { name: r.get(0)?, count: r.get(1)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn tag_id(conn: &rusqlite::Connection, name: &str, now: &str) -> Result<String, AssetError> {
    let name = clean_tag(name)?;
    if let Ok(id) =
        conn.query_row("SELECT tag_id FROM tags WHERE name = ?1", [&name], |r| r.get(0))
    {
        return Ok(id);
    }
    let id = new_id();
    conn.execute(
        "INSERT INTO tags (tag_id, name, created_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![&id, &name, now],
    )?;
    Ok(id)
}

/// Tags no asset carries any more are removed, so the list stays a list of
/// what is in use.
fn prune_tags(conn: &rusqlite::Connection) -> Result<(), AssetError> {
    conn.execute("DELETE FROM tags WHERE tag_id NOT IN (SELECT tag_id FROM asset_tags)", [])?;
    Ok(())
}

/// Replace one asset's tags.
pub fn set_tags(
    vault: &Vault,
    asset_id: &str,
    names: &[String],
    now: &str,
) -> Result<(), AssetError> {
    assets::get(vault, asset_id)?;
    let unit = crate::atomic::begin(vault.conn())?;
    unit.execute("DELETE FROM asset_tags WHERE asset_id = ?1", [asset_id])?;
    for name in names {
        let id = tag_id(&unit, name, now)?;
        unit.execute(
            "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id) VALUES (?1, ?2)",
            [asset_id, &id],
        )?;
    }
    prune_tags(&unit)?;
    unit.commit()?;
    Ok(())
}

/// Rename a tag everywhere. Renaming onto an existing tag merges the two.
pub fn rename_tag(vault: &Vault, from: &str, to: &str, now: &str) -> Result<(), AssetError> {
    let unit = crate::atomic::begin(vault.conn())?;
    let old: String = unit
        .query_row("SELECT tag_id FROM tags WHERE name = ?1", [from.trim()], |r| r.get(0))
        .map_err(|_| AssetError::Other(format!("no tag called {from:?}")))?;
    let to = clean_tag(to)?;
    let existing: Option<String> = unit
        .query_row(
            "SELECT tag_id FROM tags WHERE name = ?1 AND tag_id <> ?2",
            [&to, &old],
            |r| r.get(0),
        )
        .ok();
    match existing {
        Some(target) => {
            unit.execute(
                "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id)
                 SELECT asset_id, ?1 FROM asset_tags WHERE tag_id = ?2",
                [&target, &old],
            )?;
            unit.execute("DELETE FROM tags WHERE tag_id = ?1", [&old])?;
        }
        None => {
            unit.execute("UPDATE tags SET name = ?1 WHERE tag_id = ?2", [&to, &old])?;
        }
    }
    let _ = now;
    unit.commit()?;
    Ok(())
}

/// Every storage location in use, with counts, alphabetically.
pub fn locations(vault: &Vault) -> Result<Vec<TagCount>, AssetError> {
    let mut stmt = vault.conn().prepare(
        "SELECT storage_location, count(*) FROM assets
         WHERE deleted_at IS NULL AND storage_location IS NOT NULL AND storage_location <> ''
         GROUP BY storage_location ORDER BY storage_location COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |r| Ok(TagCount { name: r.get(0)?, count: r.get(1)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// What to change on every selected asset. `None` leaves a field alone.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BulkChange {
    /// `Some(None)` clears the location.
    #[serde(default)]
    pub storage_location: Option<Option<String>>,
    /// `Some(None)` turns reminders off.
    #[serde(default)]
    pub review_every_days: Option<Option<i64>>,
    #[serde(default)]
    pub add_tags: Vec<String>,
    #[serde(default)]
    pub remove_tags: Vec<String>,
}

/// Apply one change to many assets, all or nothing. Each asset's previous
/// version is kept, as for a single edit. Returns how many changed.
pub fn bulk_edit(
    vault: &Vault,
    asset_ids: &[String],
    change: &BulkChange,
    now: &str,
) -> Result<usize, AssetError> {
    if asset_ids.len() > 10_000 {
        return Err(AssetError::Other("too many assets in one change".into()));
    }
    let unit = crate::atomic::begin(vault.conn())?;
    let add: Vec<String> =
        change.add_tags.iter().map(|t| tag_id(&unit, t, now)).collect::<Result<_, _>>()?;
    for id in asset_ids {
        let record = assets::get(vault, id)?;
        if record.deleted_at.is_some() {
            return Err(AssetError::Other(format!("{} is in the trash", record.name)));
        }
        if change.storage_location.is_some() || change.review_every_days.is_some() {
            let mut edit = assets::Snapshot::of_record(&record).to_edit()?;
            if let Some(location) = &change.storage_location {
                edit.storage_location = location.clone();
            }
            if let Some(review) = change.review_every_days {
                edit.review_every_days = review;
            }
            // Status is not touched here; keep it as stored.
            edit.status = record.status.clone();
            assets::update(vault, id, &edit, now)?;
        }
        for tag in &add {
            unit.execute(
                "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id) VALUES (?1, ?2)",
                [id, tag],
            )?;
        }
        for name in &change.remove_tags {
            unit.execute(
                "DELETE FROM asset_tags WHERE asset_id = ?1
                 AND tag_id = (SELECT tag_id FROM tags WHERE name = ?2)",
                [id, &name.trim().to_string()],
            )?;
        }
    }
    prune_tags(&unit)?;
    unit.commit()?;
    Ok(asset_ids.len())
}

/// Rename a location wherever it is used, including places inside it:
/// renaming "Safe" to "Bank box" turns "Safe / Top shelf" into
/// "Bank box / Top shelf". This is moving a box: its contents come along.
pub fn rename_location(
    vault: &Vault,
    from: &str,
    to: &str,
    now: &str,
) -> Result<usize, AssetError> {
    let from = from.trim();
    let to = to.trim();
    if from.is_empty() || to.is_empty() {
        return Err(AssetError::Other("a location needs a name".into()));
    }
    let nested = format!("{from} / ");
    let ids: Vec<(String, String)> = {
        let mut stmt = vault.conn().prepare(
            "SELECT asset_id, storage_location FROM assets
             WHERE deleted_at IS NULL AND (storage_location = ?1 OR substr(storage_location, 1, ?2) = ?3)",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![from, nested.chars().count() as i64, &nested], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let unit = crate::atomic::begin(vault.conn())?;
    for (id, location) in &ids {
        let renamed = format!("{to}{}", &location[from.len()..]);
        let record = assets::get(vault, id)?;
        let mut edit = assets::Snapshot::of_record(&record).to_edit()?;
        edit.storage_location = Some(renamed);
        edit.status = record.status.clone();
        assets::update(vault, id, &edit, now)?;
    }
    unit.commit()?;
    Ok(ids.len())
}

/// Keys that identify one particular item. A duplicate is a different item,
/// so they are never copied: two records with one serial number would claim
/// to be the same watch.
pub const IDENTIFYING_KEYS: &[&str] =
    &["serial_number", "cert_number", "watch_address", "lot_number"];

/// Copy an asset's description into a new asset: type, details, location,
/// tags — not its history, value, photos, documents or anything that
/// identifies the original. Returns the new asset's ID.
pub fn duplicate(
    vault: &Vault,
    asset_id: &str,
    today: &str,
    now: &str,
) -> Result<String, AssetError> {
    let r = assets::get(vault, asset_id)?;
    let attrs: BTreeMap<String, String> =
        r.attrs.into_iter().filter(|(k, _)| !IDENTIFYING_KEYS.contains(&k.as_str())).collect();
    let unit = crate::atomic::begin(vault.conn())?;
    let id = assets::create(
        vault,
        &assets::NewAsset {
            type_id: r.type_id,
            name: format!("{} (copy)", r.name),
            quantity: am_core::Decimal::ONE,
            quantity_unit: r.quantity_unit,
            acquired_date: None,
            effective_date: Some(today.to_string()),
            acquired_cost: None,
            acquired_from: r.acquired_from,
            storage_location: r.storage_location,
            notes: String::new(),
            insured: None,
            attrs,
            pricing: if r.pricing == "market" {
                assets::Pricing::Market
            } else {
                assets::Pricing::Manual
            },
            review_every_days: r.review_every_days,
        },
        now,
    )?;
    unit.execute(
        "INSERT INTO asset_tags (asset_id, tag_id) SELECT ?1, tag_id FROM asset_tags WHERE asset_id = ?2",
        [&id, asset_id],
    )?;
    unit.commit()?;
    Ok(id)
}

/// A named holdings view: filters and sort, as the frontend defines them.
/// Stored opaquely but bounded, inside the vault — a saved search can be
/// as revealing as the catalog itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedView {
    pub name: String,
    pub view: serde_json::Value,
}

const VIEWS_KEY: &str = "saved_views";
const MAX_VIEWS: usize = 50;

pub fn saved_views(vault: &Vault) -> Result<Vec<SavedView>, AssetError> {
    let raw = crate::settings::get(vault, VIEWS_KEY)?;
    Ok(raw.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default())
}

/// Save a view under a name, replacing one of the same name.
pub fn save_view(vault: &Vault, name: &str, view: serde_json::Value) -> Result<(), AssetError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AssetError::Other("a view needs a name of up to 80 characters".into()));
    }
    let encoded = view.to_string();
    if encoded.len() > 8 * 1024 {
        return Err(AssetError::Other("that view is too large to save".into()));
    }
    let mut views = saved_views(vault)?;
    views.retain(|v| !v.name.eq_ignore_ascii_case(name));
    if views.len() >= MAX_VIEWS {
        return Err(AssetError::Other(format!("up to {MAX_VIEWS} saved views")));
    }
    views.push(SavedView { name: name.to_string(), view });
    views.sort_by_key(|v| v.name.to_lowercase());
    let json = serde_json::to_string(&views).map_err(|e| AssetError::Other(e.to_string()))?;
    crate::settings::set(vault, VIEWS_KEY, &json)?;
    Ok(())
}

pub fn delete_view(vault: &Vault, name: &str) -> Result<(), AssetError> {
    let mut views = saved_views(vault)?;
    views.retain(|v| v.name != name);
    let json = serde_json::to_string(&views).map_err(|e| AssetError::Other(e.to_string()))?;
    crate::settings::set(vault, VIEWS_KEY, &json)?;
    Ok(())
}

fn new_id() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{create, get, list, revisions, NewAsset, Pricing};
    use am_core::Decimal;
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

    fn asset(v: &Vault, name: &str, location: Option<&str>) -> String {
        create(
            v,
            &NewAsset {
                type_id: "watch".into(),
                name: name.into(),
                quantity: Decimal::ONE,
                quantity_unit: "item".into(),
                acquired_date: None,
                effective_date: None,
                acquired_cost: None,
                acquired_from: None,
                storage_location: location.map(str::to_string),
                notes: String::new(),
                insured: None,
                attrs: [
                    ("serial_number".to_string(), "SN1".to_string()),
                    ("model".to_string(), "Submariner".to_string()),
                ]
                .into_iter()
                .collect(),
                pricing: Pricing::Manual,
                review_every_days: None,
            },
            NOW,
        )
        .unwrap()
    }

    fn names(tags: &[TagCount]) -> Vec<(String, i64)> {
        tags.iter().map(|t| (t.name.clone(), t.count)).collect()
    }

    #[test]
    fn tags_are_case_insensitive_searchable_and_pruned() {
        let (_d, v) = setup();
        let a = asset(&v, "A", None);
        let b = asset(&v, "B", None);
        set_tags(&v, &a, &["Insured rider".into(), "  for   sale ".into()], NOW).unwrap();
        set_tags(&v, &b, &["insured RIDER".into()], NOW).unwrap();

        assert_eq!(
            names(&tags(&v).unwrap()),
            vec![("for sale".into(), 1), ("Insured rider".into(), 2)]
        );
        assert_eq!(get(&v, &a).unwrap().tags, vec!["for sale", "Insured rider"]);
        let mut found = crate::assets::search(&v, "rider").unwrap();
        found.sort();
        let mut expected = vec![a.clone(), b.clone()];
        expected.sort();
        assert_eq!(found, expected);

        set_tags(&v, &a, &[], NOW).unwrap();
        assert_eq!(
            names(&tags(&v).unwrap()),
            vec![("Insured rider".into(), 1)],
            "unused tags go"
        );
    }

    #[test]
    fn renaming_a_tag_onto_another_merges_them() {
        let (_d, v) = setup();
        let a = asset(&v, "A", None);
        let b = asset(&v, "B", None);
        set_tags(&v, &a, &["Heirloom".into()], NOW).unwrap();
        set_tags(&v, &b, &["heirlooms".into()], NOW).unwrap();
        rename_tag(&v, "heirlooms", "Heirloom", NOW).unwrap();
        assert_eq!(names(&tags(&v).unwrap()), vec![("Heirloom".into(), 2)]);
    }

    #[test]
    fn a_bulk_edit_changes_every_selected_asset_and_each_can_be_undone() {
        let (_d, v) = setup();
        let a = asset(&v, "A", Some("Desk"));
        let b = asset(&v, "B", Some("Desk"));
        let untouched = asset(&v, "C", Some("Desk"));

        let change = BulkChange {
            storage_location: Some(Some("Safe".into())),
            review_every_days: Some(Some(90)),
            add_tags: vec!["Moved".into()],
            remove_tags: vec![],
        };
        assert_eq!(bulk_edit(&v, &[a.clone(), b.clone()], &change, NOW).unwrap(), 2);
        for id in [&a, &b] {
            let r = get(&v, id).unwrap();
            assert_eq!(r.storage_location.as_deref(), Some("Safe"));
            assert_eq!(r.review_every_days, Some(90));
            assert_eq!(r.tags, vec!["Moved"]);
            assert_eq!(revisions(&v, id).unwrap().len(), 1, "undoable like any edit");
        }
        assert_eq!(get(&v, &untouched).unwrap().storage_location.as_deref(), Some("Desk"));

        let remove = BulkChange { remove_tags: vec!["moved".into()], ..Default::default() };
        bulk_edit(&v, std::slice::from_ref(&a), &remove, NOW).unwrap();
        assert!(get(&v, &a).unwrap().tags.is_empty());
    }

    #[test]
    fn moving_a_location_moves_what_is_inside_it() {
        let (_d, v) = setup();
        let a = asset(&v, "A", Some("Safe"));
        let b = asset(&v, "B", Some("Safe / Top shelf"));
        let c = asset(&v, "C", Some("Safekeeping box"));
        assert_eq!(rename_location(&v, "Safe", "Bank box", NOW).unwrap(), 2);
        assert_eq!(get(&v, &a).unwrap().storage_location.as_deref(), Some("Bank box"));
        assert_eq!(
            get(&v, &b).unwrap().storage_location.as_deref(),
            Some("Bank box / Top shelf")
        );
        assert_eq!(
            get(&v, &c).unwrap().storage_location.as_deref(),
            Some("Safekeeping box"),
            "a different place that merely starts the same"
        );
        assert_eq!(names(&locations(&v).unwrap()).len(), 3);
    }

    #[test]
    fn a_duplicate_never_copies_what_identifies_the_original() {
        let (_d, v) = setup();
        let original = asset(&v, "Rolex", Some("Safe"));
        set_tags(&v, &original, &["Watches".into()], NOW).unwrap();
        let copy = duplicate(&v, &original, "2026-09-19", NOW).unwrap();
        let r = get(&v, &copy).unwrap();
        assert_eq!(r.name, "Rolex (copy)");
        assert_eq!(r.attrs.get("model").map(String::as_str), Some("Submariner"));
        assert!(!r.attrs.contains_key("serial_number"), "a second item is not the same serial");
        assert_eq!(r.tags, vec!["Watches"]);
        assert_eq!(r.storage_location.as_deref(), Some("Safe"));
        assert_eq!(r.current_amount_minor, None, "no value, no history");
        assert_eq!(list(&v).unwrap().len(), 2);
    }

    #[test]
    fn saved_views_replace_by_name_and_are_bounded() {
        let (_d, v) = setup();
        save_view(&v, "Uninsured watches", serde_json::json!({ "category": "valuables" }))
            .unwrap();
        save_view(&v, "uninsured WATCHES", serde_json::json!({ "category": "all" })).unwrap();
        let views = saved_views(&v).unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].view["category"], "all");
        assert!(save_view(&v, " ", serde_json::json!({})).is_err());
        let huge = serde_json::json!({ "q": "x".repeat(9000) });
        assert!(save_view(&v, "Huge", huge).is_err());
        delete_view(&v, "uninsured WATCHES").unwrap();
        assert!(saved_views(&v).unwrap().is_empty());
    }
}
