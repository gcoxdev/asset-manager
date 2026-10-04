//! IPC for collectible types.
//!
//! Validation happens **here**, in the backend, not only in the form. A form
//! can be bypassed; this is the path every write actually takes.

use am_core::{collectibles, Grader, COLLECTIBLE_TYPES};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::ipc::IpcResult;
use crate::session::{IpcError, Session};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn input(type_id: &str, pairs: &[(&str, &str)]) -> CollectibleInput {
        CollectibleInput {
            type_id: type_id.into(),
            attrs: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
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

/// Read a barcode from a photo of a slab label.
///
/// A decoded barcode is a **claim printed on a label**, not proof of
/// anything: it says which certificate to look up, not that the item matches
/// it. The result is offered as a suggestion for the form, never applied
/// silently.
#[tauri::command]
pub fn scan_slab_label(
    session: State<'_, Session>,
    path: String,
) -> IpcResult<am_storage::scanning::ScanResult> {
    session.touch();
    // An unlocked vault is required: scanning is part of cataloguing, and a
    // locked app should do nothing with your data.
    session.with_vault(|_| Ok(())).map_err(IpcError::from)?;

    let bytes = std::fs::read(&path)
        .map_err(|e| IpcError { kind: "unreadable_file".into(), message: e.to_string() })?;

    am_storage::scanning::scan_image(&bytes).map_err(|e| IpcError {
        kind: match e {
            am_storage::scanning::ScanError::NotFound => "no_barcode",
            _ => "scan_failed",
        }
        .into(),
        message: e.to_string(),
    })
}
