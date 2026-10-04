//! Item types the owner defines, and the validation of their fields.
//!
//! A definition is data: labels, field kinds, required flags, choices. It
//! never contains anything executed. Saving an asset of a custom type is
//! validated here, so a form that skipped a required field — or a
//! spreadsheet row — is refused the same way.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::vault::Vault;

#[derive(Debug, thiserror::Error)]
pub enum CustomTypeError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

fn invalid(m: impl Into<String>) -> CustomTypeError {
    CustomTypeError::Invalid(m.into())
}

pub const FIELD_KINDS: &[&str] = &["text", "number", "date", "yes_no", "choice"];

/// Keys the app gives meaning to; a custom field may not take them.
const RESERVED_KEYS: &[&str] = &[
    "metal",
    "coin_id",
    "symbol",
    "preset",
    "weight_per_item",
    "weight_unit",
    "weight_basis",
    "purity",
    "premium_pct",
    "watch_address",
    "watch_chain",
    "chain",
    "contract",
    "custody",
];

/// Categories a custom type can belong to.
pub const CATEGORIES: &[&str] =
    &["collectibles", "valuables", "household", "vehicles", "property", "investments", "other"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldDef {
    /// Stable storage key, derived from the first label. Renaming a field's
    /// label keeps its key, so existing values stay attached.
    #[serde(default)]
    pub key: String,
    pub label: String,
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    /// For `choice`.
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomType {
    #[serde(default)]
    pub type_id: String,
    pub label: String,
    pub category: String,
    #[serde(default)]
    pub blurb: String,
    pub fields: Vec<FieldDef>,
    /// Assets using it (trashed ones included), for "can it be deleted".
    #[serde(default)]
    pub in_use: i64,
}

/// "Model train" → "model_train".
pub fn slug(label: &str) -> String {
    let s: String = label
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    s.split('_')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("_")
        .chars()
        .take(40)
        .collect()
}

fn check(def: &CustomType) -> Result<Vec<FieldDef>, CustomTypeError> {
    let label = def.label.trim();
    if label.is_empty() || label.chars().count() > 60 {
        return Err(invalid("a type needs a name of up to 60 characters"));
    }
    if !CATEGORIES.contains(&def.category.as_str()) {
        return Err(invalid(format!("unknown category: {}", def.category)));
    }
    if def.blurb.chars().count() > 200 {
        return Err(invalid("the description is limited to 200 characters"));
    }
    if def.fields.is_empty() || def.fields.len() > 30 {
        return Err(invalid("a type has between 1 and 30 fields"));
    }
    let mut out: Vec<FieldDef> = Vec::new();
    for f in &def.fields {
        let label = f.label.trim();
        if label.is_empty() || label.chars().count() > 60 {
            return Err(invalid("each field needs a name of up to 60 characters"));
        }
        if !FIELD_KINDS.contains(&f.kind.as_str()) {
            return Err(invalid(format!("unknown field kind: {}", f.kind)));
        }
        let key = if f.key.is_empty() { slug(label) } else { f.key.clone() };
        if key.is_empty()
            || key.len() > 40
            || !key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(invalid(format!("{label:?} needs a name with some letters or digits")));
        }
        if RESERVED_KEYS.contains(&key.as_str()) {
            return Err(invalid(format!(
                "{label:?} is a name the app uses for something else"
            )));
        }
        if out.iter().any(|o| o.key == key) {
            return Err(invalid(format!("two fields are both called {label:?}")));
        }
        let options: Vec<String> =
            f.options.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
        if f.kind == "choice"
            && (options.is_empty()
                || options.len() > 30
                || options.iter().any(|o| o.chars().count() > 60))
        {
            return Err(invalid(format!("{label:?} needs between 1 and 30 choices")));
        }
        out.push(FieldDef {
            key,
            label: label.to_string(),
            kind: f.kind.clone(),
            required: f.required,
            options: if f.kind == "choice" { options } else { Vec::new() },
        });
    }
    Ok(out)
}

/// Create a type, or replace an existing one's definition when `type_id`
/// is set. Returns the type's ID.
pub fn save(vault: &Vault, def: &CustomType, now: &str) -> Result<String, CustomTypeError> {
    let fields = check(def)?;
    let label = def.label.trim();
    let unit = crate::atomic::begin(vault.conn())?;
    let clash: i64 = unit.query_row(
        "SELECT count(*) FROM asset_types WHERE lower(display_name) = lower(?1) AND type_id <> ?2",
        [label, &def.type_id],
        |r| r.get(0),
    )?;
    if clash > 0 {
        return Err(invalid(format!("there is already a type called {label:?}")));
    }
    let json = serde_json::to_string(&fields).map_err(|e| invalid(e.to_string()))?;
    let type_id = if def.type_id.is_empty() {
        let base = format!("custom_{}", slug(label));
        let mut id = base.clone();
        let mut n = 2;
        while unit.query_row(
            "SELECT count(*) FROM asset_types WHERE type_id = ?1",
            [&id],
            |r| r.get::<_, i64>(0),
        )? > 0
        {
            id = format!("{base}_{n}");
            n += 1;
        }
        unit.execute(
            "INSERT INTO asset_types (type_id, archetype, display_name, category) VALUES (?1, 'unique', ?2, ?3)",
            [&id, label, &def.category],
        )?;
        unit.execute(
            "INSERT INTO custom_types (type_id, blurb, fields, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            rusqlite::params![&id, def.blurb.trim(), json, now],
        )?;
        id
    } else {
        // Existing fields keep their keys, so values already stored stay
        // attached; a field removed from the definition leaves its values
        // on the assets, shown as "Other details".
        let n = unit.execute(
            "UPDATE custom_types SET blurb = ?1, fields = ?2, updated_at = ?3 WHERE type_id = ?4",
            rusqlite::params![def.blurb.trim(), json, now, &def.type_id],
        )?;
        if n == 0 {
            return Err(invalid("that type no longer exists"));
        }
        unit.execute(
            "UPDATE asset_types SET display_name = ?1, category = ?2 WHERE type_id = ?3",
            [label, &def.category, &def.type_id],
        )?;
        def.type_id.clone()
    };
    unit.commit()?;
    Ok(type_id)
}

pub fn list(vault: &Vault) -> Result<Vec<CustomType>, CustomTypeError> {
    let mut stmt = vault.conn().prepare(
        "SELECT c.type_id, t.display_name, t.category, c.blurb, c.fields,
                (SELECT count(*) FROM assets a WHERE a.type_id = c.type_id)
         FROM custom_types c JOIN asset_types t ON t.type_id = c.type_id
         ORDER BY t.display_name COLLATE NOCASE",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let fields: String = r.get(4)?;
            Ok(CustomType {
                type_id: r.get(0)?,
                label: r.get(1)?,
                category: r.get(2)?,
                blurb: r.get(3)?,
                fields: serde_json::from_str(&fields).unwrap_or_default(),
                in_use: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(vault: &Vault, type_id: &str) -> Result<Option<CustomType>, CustomTypeError> {
    Ok(list(vault)?.into_iter().find(|t| t.type_id == type_id))
}

/// Delete a type no asset uses — trashed ones included, since restoring one
/// would otherwise point at nothing.
pub fn delete(vault: &Vault, type_id: &str) -> Result<(), CustomTypeError> {
    let t = get(vault, type_id)?.ok_or_else(|| invalid("that type no longer exists"))?;
    if t.in_use > 0 {
        return Err(invalid(format!(
            "{} asset{} still use{} this type — change their type first",
            t.in_use,
            if t.in_use == 1 { "" } else { "s" },
            if t.in_use == 1 { "s" } else { "" }
        )));
    }
    vault.conn().execute("DELETE FROM asset_types WHERE type_id = ?1", [type_id])?;
    Ok(())
}

/// Check an asset's details against its custom type. Unknown keys are kept,
/// as for every type; defined fields must be present when required and
/// well-formed when given.
pub fn validate(
    def: &CustomType,
    attrs: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, CustomTypeError> {
    let mut cleaned = attrs.clone();
    for f in &def.fields {
        let value = cleaned.get(&f.key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let Some(value) = value else {
            if f.required {
                return Err(invalid(format!("{} is required", f.label)));
            }
            cleaned.remove(&f.key);
            continue;
        };
        let normalized = match f.kind.as_str() {
            "number" => {
                let n = value.replace([',', ' '], "");
                am_core::parse_decimal(&n)
                    .map_err(|_| invalid(format!("{} should be a number", f.label)))?
                    .normalize()
                    .to_string()
            }
            "date" => crate::events::normalize_date(&value)
                .map_err(|_| invalid(format!("{} should be a date", f.label)))?,
            "yes_no" => match value.to_lowercase().as_str() {
                "yes" | "y" | "true" => "yes".to_string(),
                "no" | "n" | "false" => "no".to_string(),
                _ => return Err(invalid(format!("{} should be yes or no", f.label))),
            },
            "choice" => f
                .options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(&value))
                .cloned()
                .ok_or_else(|| {
                    invalid(format!("{} should be one of: {}", f.label, f.options.join(", ")))
                })?,
            _ => value,
        };
        cleaned.insert(f.key.clone(), normalized);
    }
    Ok(cleaned)
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

    fn field(label: &str, kind: &str, required: bool, options: &[&str]) -> FieldDef {
        FieldDef {
            key: String::new(),
            label: label.into(),
            kind: kind.into(),
            required,
            options: options.iter().map(|o| o.to_string()).collect(),
        }
    }

    fn trains() -> CustomType {
        CustomType {
            type_id: String::new(),
            label: "Model train".into(),
            category: "collectibles".into(),
            blurb: "Locomotives and rolling stock".into(),
            fields: vec![
                field("Maker", "text", true, &[]),
                field("Scale", "choice", true, &["HO", "N", "O"]),
                field("Year made", "number", false, &[]),
                field("Boxed", "yes_no", false, &[]),
            ],
            in_use: 0,
        }
    }

    #[test]
    fn a_type_is_defined_and_its_fields_validated() {
        let (_d, v) = setup();
        let id = save(&v, &trains(), NOW).unwrap();
        assert_eq!(id, "custom_model_train");
        let def = get(&v, &id).unwrap().unwrap();
        assert_eq!(def.fields[2].key, "year_made");

        let attrs = |pairs: &[(&str, &str)]| {
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
        };
        let ok = validate(
            &def,
            &attrs(&[
                ("maker", "Märklin"),
                ("scale", "ho"),
                ("year_made", "1,957"),
                ("boxed", "Y"),
                ("extra", "kept"),
            ]),
        )
        .unwrap();
        assert_eq!(ok["scale"], "HO", "choices are matched without regard to case");
        assert_eq!(ok["year_made"], "1957");
        assert_eq!(ok["boxed"], "yes");
        assert_eq!(ok["extra"], "kept", "unknown details are kept");
        assert!(validate(&def, &attrs(&[("scale", "HO")]))
            .unwrap_err()
            .to_string()
            .contains("Maker"));
        assert!(validate(&def, &attrs(&[("maker", "x"), ("scale", "Z")])).is_err());
        assert!(validate(
            &def,
            &attrs(&[("maker", "x"), ("scale", "N"), ("year_made", "old")])
        )
        .is_err());

        // A type built in to the app is matched by name too, but can't be
        // taken by a custom type.
        let mut clash = trains();
        clash.label = "Watch".into();
        assert!(save(&v, &clash, NOW).is_err());
    }

    #[test]
    fn definitions_are_checked() {
        let (_d, v) = setup();
        let mut bad = trains();
        bad.fields.push(field("maker", "text", false, &[]));
        assert!(save(&v, &bad, NOW).unwrap_err().to_string().contains("both called"));
        let mut bad = trains();
        bad.fields.push(field("Metal", "text", false, &[]));
        assert!(save(&v, &bad, NOW).is_err(), "reserved keys");
        let mut bad = trains();
        bad.fields.push(field("Color", "choice", false, &[]));
        assert!(save(&v, &bad, NOW).is_err(), "a choice needs options");
        let mut bad = trains();
        bad.category = "spaceships".into();
        assert!(save(&v, &bad, NOW).is_err());
    }

    #[test]
    fn renaming_keeps_keys_and_a_type_in_use_cannot_be_deleted() {
        let (_d, v) = setup();
        let id = save(&v, &trains(), NOW).unwrap();
        let mut def = get(&v, &id).unwrap().unwrap();
        def.fields[0].label = "Manufacturer".into();
        def.label = "Model trains".into();
        save(&v, &def, NOW).unwrap();
        let def = get(&v, &id).unwrap().unwrap();
        assert_eq!((def.label.as_str(), def.fields[0].key.as_str()), ("Model trains", "maker"));

        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at) VALUES ('a', ?1, 'Loco', ?2, ?2)",
                [&id, NOW],
            )
            .unwrap();
        assert!(delete(&v, &id).unwrap_err().to_string().contains("still use"));
        v.conn().execute("DELETE FROM assets", []).unwrap();
        delete(&v, &id).unwrap();
        assert!(list(&v).unwrap().is_empty());
    }
}
