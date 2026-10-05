//! Preferences that belong to this computer, not to a vault.
//!
//! The theme has to be known on the unlock screen, before any vault is open,
//! so it cannot live in the vault's encrypted settings. It is kept in a small
//! `preferences.json` beside the vault folder. Nothing in it is sensitive —
//! only how the app looks — and nothing sensitive may ever be added to it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

use crate::ipc::{bad_input, IpcResult};
use crate::paths::vault_root;
use crate::session::IpcError;

const FILE: &str = "preferences.json";
const THEMES: &[&str] = &["system", "light", "dark"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Preferences {
    /// "system" (follow the computer), "light" or "dark".
    #[serde(default = "system")]
    pub theme: String,
}

fn system() -> String {
    "system".into()
}

impl Default for Preferences {
    fn default() -> Self {
        Self { theme: system() }
    }
}

fn path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, IpcError> {
    let vault =
        vault_root(app).map_err(|e| IpcError { kind: "internal".into(), message: e })?;
    let dir = vault.parent().map(PathBuf::from).unwrap_or(vault);
    Ok(dir.join(FILE))
}

/// The saved preferences; defaults when there are none or the file is
/// unreadable — a damaged preferences file must never stop the app opening.
fn load(path: &std::path::Path) -> Preferences {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok())
        .filter(|p| THEMES.contains(&p.theme.as_str()))
        .unwrap_or_default()
}

/// Write whole-or-not-at-all: a temporary file renamed over the old one.
fn store(path: &std::path::Path, prefs: &Preferences) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(prefs).expect("preferences serialize"))?;
    std::fs::rename(&temp, path)
}

/// Readable whether or not a vault is open.
#[tauri::command]
pub fn get_preferences<R: Runtime>(app: AppHandle<R>) -> IpcResult<Preferences> {
    Ok(load(&path(&app)?))
}

#[tauri::command]
pub fn set_theme<R: Runtime>(app: AppHandle<R>, theme: String) -> IpcResult<Preferences> {
    if !THEMES.contains(&theme.as_str()) {
        return Err(bad_input(format!("unknown theme {theme:?}")));
    }
    let path = path(&app)?;
    // Keep whatever else is saved; change only the theme.
    let mut prefs = load(&path);
    prefs.theme = theme;
    store(&path, &prefs)
        .map_err(|e| IpcError { kind: "unwritable_file".into(), message: e.to_string() })?;
    Ok(prefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_damaged_preferences_fall_back_to_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        assert_eq!(load(&file), Preferences::default(), "no file");
        std::fs::write(&file, b"{ not json").unwrap();
        assert_eq!(load(&file), Preferences::default(), "unreadable");
        std::fs::write(&file, br#"{ "theme": "neon" }"#).unwrap();
        assert_eq!(load(&file), Preferences::default(), "unknown theme");
    }

    #[test]
    fn a_saved_theme_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join(FILE);
        store(&file, &Preferences { theme: "dark".into() }).unwrap();
        assert_eq!(load(&file).theme, "dark");
        assert!(!file.with_extension("json.tmp").exists(), "no temporary file left behind");
    }
}
