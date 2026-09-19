//! Where the vault lives.
//!
//! The path is resolved in the backend, never passed in from the frontend.
//! Accepting a path over IPC would let the UI (or anything that reached it)
//! point vault operations at an arbitrary location — including reading a
//! header from somewhere unexpected or writing a vault over another file.
//!
//! A single fixed location also means "does a vault exist?" has one answer,
//! rather than depending on the process's working directory.

use std::path::PathBuf;

use tauri::Manager;

/// Subdirectory under the platform app-data directory.
///
/// Linux:   ~/.local/share/dev.gcox.assetmanager/vault
/// macOS:   ~/Library/Application Support/dev.gcox.assetmanager/vault
/// Windows: %APPDATA%\dev.gcox.assetmanager\vault
const VAULT_DIR: &str = "vault";

/// Overridable for development and tests. Never consulted in a release build,
/// so a stray environment variable cannot redirect a real user's vault.
#[cfg(debug_assertions)]
const VAULT_OVERRIDE_ENV: &str = "AM_VAULT_DIR";

pub fn vault_root<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var(VAULT_OVERRIDE_ENV) {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }

    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not determine the application data directory: {e}"))?;
    Ok(base.join(VAULT_DIR))
}
