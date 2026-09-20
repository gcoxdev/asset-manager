//! IPC for collectible types.
//!
//! Validation happens **here**, in the backend, not only in the form. A form
//! can be bypassed; this is the path every write actually takes.

use am_core::{collectibles, Attributes, Grader, COLLECTIBLE_TYPES};
use am_storage::vault::VaultError;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::session::{IpcError, Session, SessionError};

type IpcResult<T> = Result<T, IpcError>;

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn storage(e: impl std::fmt::Display) -> SessionError {
    SessionError::Vault(VaultError::Other(e.to_string()))
}

#[derive(Serialize)]
pub struct CollectibleTypeInfo {
    pub id: String,
    pub label: String,
    pub required: Vec<String>,
    pub optional: Vec<String>,
}

#[tauri::command]
pub fn collectible_types() -> Vec<CollectibleTypeInfo> {
    COLLECTIBLE_TYPES
        .iter()
        .map(|t| CollectibleTypeInfo {
            id: t.id.to_string(),
            label: t.label.to_string(),
            required: t.required.iter().map(|f| (*f).to_string()).collect(),
            optional: t.optional.iter().map(|f| (*f).to_string()).collect(),
        })
        .collect()
}

#[derive(Serialize)]
pub struct GraderInfo {
    pub id: String,
    pub label: String,
    pub min: String,
    pub max: String,
    pub step: String,
    /// False for ungraded, which carries no numeric grade.
    pub numeric: bool,
}

/// Graders with their scales, so the form can show what a valid grade is
/// rather than rejecting one after the fact.
#[tauri::command]
pub fn graders() -> Vec<GraderInfo> {
    Grader::ALL
        .iter()
        .map(|g| {
            let (min, max, step) = g.scale();
            GraderInfo {
                id: g.as_str().to_string(),
                label: g.display_name().to_string(),
                min: min.normalize().to_string(),
                max: max.normalize().to_string(),
                step: step.normalize().to_string(),
                numeric: *g != Grader::Raw,
            }
        })
        .collect()
}

#[derive(Deserialize)]
pub struct CollectibleInput {
    pub type_id: String,
    pub attrs: std::collections::BTreeMap<String, String>,
    pub quantity: Option<String>,
    pub storage_location: Option<String>,
    pub notes: Option<String>,
}

#[derive(Serialize)]
pub struct ValidationResult {
    pub ok: bool,
    pub error: Option<String>,
    /// Label the asset would get, so the form can preview it.
    pub label: Option<String>,
}

/// Check attributes without saving.
///
/// Lets the form report a bad grade while the user is still looking at it,
/// while the same validation still runs on save.
#[tauri::command]
pub fn validate_collectible(input: CollectibleInput) -> ValidationResult {
    match collectibles::validate(&input.type_id, &input.attrs) {
        Ok(cleaned) => ValidationResult {
            ok: true,
            error: None,
            label: Some(collectibles::describe(&input.type_id, &cleaned)),
        },
        Err(e) => ValidationResult { ok: false, error: Some(e.to_string()), label: None },
    }
}

#[tauri::command]
pub fn create_collectible(
    session: State<'_, Session>,
    input: CollectibleInput,
) -> IpcResult<String> {
    session.touch();

    // Validated before anything is written. The form calls this too, but a
    // form is a convenience, not a control.
    let cleaned: Attributes = collectibles::validate(&input.type_id, &input.attrs)
        .map_err(|e| IpcError { kind: "invalid_input".into(), message: e.to_string() })?;

    let label = collectibles::describe(&input.type_id, &cleaned);
    let attrs_json = serde_json::to_string(&cleaned)
        .map_err(|e| IpcError { kind: "error".into(), message: e.to_string() })?;

    let asset_id = uuid::Uuid::new_v4().to_string();
    let timestamp = now();
    let quantity = input.quantity.clone().unwrap_or_else(|| "1".into());

    session
        .with_vault(|vault| {
            let sort = quantity.parse::<f64>().unwrap_or(1.0);
            let tx = vault.conn().unchecked_transaction().map_err(storage)?;

            tx.execute(
                "INSERT INTO assets
                   (asset_id, type_id, name, quantity, quantity_sort, quantity_unit,
                    storage_location, notes, attrs, schema_version, created_at, updated_at)
                 VALUES (?1, 'generic', ?2, ?3, ?4, 'item', ?5, ?6, ?7, ?8, ?9, ?9)",
                rusqlite::params![
                    &asset_id,
                    &label,
                    &quantity,
                    sort,
                    &input.storage_location,
                    input.notes.as_deref().unwrap_or(""),
                    &attrs_json,
                    am_core::collectibles::COLLECTIBLE_SCHEMA_VERSION,
                    &timestamp,
                ],
            )
            .map_err(storage)?;

            // Ownership starts as an event here too, so the history is
            // uniform regardless of which form created the asset.
            tx.execute(
                "INSERT INTO asset_events
                   (event_id, asset_id, event_type, effective_date, quantity_delta, recorded_at)
                 VALUES (?1, ?2, 'acquire', ?3, ?4, ?3)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    &asset_id,
                    &timestamp,
                    &quantity
                ],
            )
            .map_err(storage)?;

            tx.commit().map_err(storage)?;
            Ok(())
        })
        .map_err(IpcError::from)?;

    Ok(asset_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(type_id: &str, pairs: &[(&str, &str)]) -> CollectibleInput {
        CollectibleInput {
            type_id: type_id.into(),
            attrs: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            quantity: None,
            storage_location: None,
            notes: None,
        }
    }

    #[test]
    fn every_type_is_exposed_with_its_fields() {
        let types = collectible_types();
        assert_eq!(types.len(), COLLECTIBLE_TYPES.len());

        let comic = types.iter().find(|t| t.id == "comic").unwrap();
        assert!(comic.required.contains(&"title".to_string()));
        assert!(comic.optional.contains(&"grader".to_string()));
    }

    #[test]
    fn graders_expose_their_scales_so_the_form_can_guide_entry() {
        let all = graders();

        let psa = all.iter().find(|g| g.id == "psa").unwrap();
        assert_eq!(
            (psa.min.as_str(), psa.max.as_str(), psa.step.as_str()),
            ("0.5", "10", "0.1")
        );
        assert!(psa.numeric);

        let pcgs = all.iter().find(|g| g.id == "pcgs").unwrap();
        assert_eq!((pcgs.min.as_str(), pcgs.max.as_str()), ("1", "70"), "Sheldon scale");

        let raw = all.iter().find(|g| g.id == "raw").unwrap();
        assert!(!raw.numeric, "ungraded carries no numeric grade");
    }

    #[test]
    fn validation_previews_the_label() {
        let result = validate_collectible(input(
            "comic",
            &[
                ("title", "Amazing Fantasy"),
                ("issue", "15"),
                ("grader", "cgc"),
                ("grade", "9.8"),
            ],
        ));
        assert!(result.ok);
        assert_eq!(result.label.as_deref(), Some("Amazing Fantasy #15 — CGC 9.8"));
    }

    #[test]
    fn validation_explains_a_bad_grade_rather_than_just_refusing() {
        let result = validate_collectible(input(
            "comic",
            &[("title", "X"), ("issue", "1"), ("grader", "psa"), ("grade", "65")],
        ));
        assert!(!result.ok);
        let message = result.error.unwrap();
        assert!(message.contains("PSA"), "the message should name the grader");
        assert!(message.contains("10"), "...and the valid range");
    }

    #[test]
    fn a_missing_required_field_is_reported_by_name() {
        let result = validate_collectible(input("comic", &[("title", "X")]));
        assert!(!result.ok);
        assert!(result.error.unwrap().contains("issue"));
    }
}
