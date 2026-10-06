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
use std::sync::OnceLock;

use tauri::Manager;

use crate::portable::Storage;

/// Decided once, at launch, from how the app was started (see portable.rs).
/// Unset — in tests — means the standard location.
static STORAGE: OnceLock<Storage> = OnceLock::new();

/// Record where this run keeps its data. Called once, before the app starts.
pub fn init_storage(storage: Storage) {
    let _ = STORAGE.set(storage);
}

/// Whether this run keeps its data beside the app.
pub fn is_portable() -> bool {
    matches!(STORAGE.get(), Some(Storage::Portable(_)))
}

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

    match STORAGE.get() {
        Some(Storage::Portable(root)) => return Ok(root.join(VAULT_DIR)),
        // Never fall back to the standard location: it would show a
        // different catalog, or none, with no sign anything was wrong.
        Some(Storage::Unavailable(message)) => return Err(message.clone()),
        Some(Storage::Standard) | None => {}
    }

    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not determine the application data directory: {e}"))?;
    Ok(base.join(VAULT_DIR))
}
