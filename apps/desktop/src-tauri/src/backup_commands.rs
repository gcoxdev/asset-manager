//! The backup centre: making backups, remembering where they went, proving
//! they restore, and saying when it is time for another.
//!
//! A copy that has never been restored is a hope, not a backup. So besides
//! "copied", each backup can be *verified*: restored into a scratch folder
//! with the credential it opens with, every photo and document decrypted,
//! and the scratch copy removed. The date of the last good verification is
//! what the owner should look at, not the date of the last copy.
//!
//! Nothing here runs in the background or talks to a cloud service: a
//! stopped app makes no backups, and the centre says so rather than
//! promising otherwise.

use std::path::{Path, PathBuf};

use am_storage::header::Credential;
use am_storage::vault::Vault;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};

use crate::ipc::{bad_input, now, storage, IpcResult};
use crate::paths::vault_root;
use crate::session::{IpcError, Session, SessionError};

const LOG_SETTING: &str = "backup_log";
const FOLDER_SETTING: &str = "backup_folder";
const KEEP_SETTING: &str = "backup_keep";
const REMINDER_SETTING: &str = "backup_reminder_days";
/// Remind after this many days without a backup unless the owner says
/// otherwise.
const DEFAULT_REMINDER_DAYS: u32 = 30;
const MAX_LOG: usize = 100;
const FOLDER_PREFIX: &str = "Asset Manager backup ";

fn other(message: String) -> IpcError {
    IpcError { kind: "error".into(), message }
}

/// One backup the app made or checked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub path: String,
    pub created_at: String,
    pub objects: usize,
    #[serde(default)]
    pub verified_at: Option<String>,
    /// The outcome of the last verification: true restored in full.
    #[serde(default)]
    pub verified_ok: Option<bool>,
    #[serde(default)]
    pub verify_note: Option<String>,
    /// Removed by the retention rule.
    #[serde(default)]
    pub pruned: bool,
}

fn log(vault: &Vault) -> Vec<BackupEntry> {
    am_storage::settings::get(vault, LOG_SETTING)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

fn save_log(vault: &Vault, mut entries: Vec<BackupEntry>) -> Result<(), SessionError> {
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    entries.truncate(MAX_LOG);
    let json = serde_json::to_string(&entries).map_err(storage)?;
    am_storage::settings::set(vault, LOG_SETTING, &json).map_err(storage)
}

pub(crate) fn reminder_days(vault: &Vault) -> u32 {
    am_storage::settings::get(vault, REMINDER_SETTING)
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_REMINDER_DAYS)
}

/// A folder name for a new backup, dated so a directory of them sorts.
fn backup_folder_name(now: &str) -> String {
    let stamp: String = now[..19].chars().map(|c| if c == ':' { '-' } else { c }).collect();
    format!("{FOLDER_PREFIX}{}", stamp.replace('T', " "))
}

#[derive(Serialize)]
pub struct BackupResult {
    pub path: String,
    pub created_at: String,
    pub objects: usize,
    /// Older backups of this vault removed by the retention rule.
    pub pruned: Vec<String>,
}

/// Back up the vault into a new folder inside `directory`, or inside the
/// remembered backup folder when none is given.
///
/// The copy is ciphertext throughout — header, database and objects — so it
/// is as safe to keep on an external drive or a synced folder as the vault
/// itself. It opens with the passphrase or recovery key that were current
/// when it was made.
#[tauri::command]
pub fn backup_vault<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    directory: Option<String>,
) -> IpcResult<BackupResult> {
    session.touch();
    let root = vault_root(&app).map_err(other)?;
    let directory = match directory {
        Some(d) => d,
        None => session
            .with_vault(|vault| {
                am_storage::settings::get(vault, FOLDER_SETTING).map_err(storage)
            })
            .map_err(IpcError::from)?
            .ok_or_else(|| bad_input("choose a folder for backups first"))?,
    };
    let parent = PathBuf::from(&directory);
    if !parent.is_dir() {
        return Err(bad_input(format!(
            "{directory} is not available — is the drive connected?"
        )));
    }
    if parent.starts_with(&root) {
        return Err(bad_input("choose a folder outside the vault itself"));
    }

    let timestamp = now();
    let dest = parent.join(backup_folder_name(&timestamp));
    let manifest = session.backup(&dest, &timestamp).map_err(IpcError::from)?;

    // Bookkeeping after the backup has succeeded: a failure here must not
    // turn a good backup into a reported error.
    let pruned = session
        .with_vault(|vault| {
            let _ = am_storage::settings::set(
                vault,
                crate::commands::LAST_BACKUP_SETTING,
                &timestamp,
            );
            let _ = am_storage::settings::set(vault, FOLDER_SETTING, &directory);
            let mut entries = log(vault);
            entries.push(BackupEntry {
                path: dest.display().to_string(),
                created_at: manifest.created_at.clone(),
                objects: manifest.objects.len(),
                verified_at: None,
                verified_ok: None,
                verify_note: None,
                pruned: false,
            });
            let keep = am_storage::settings::get(vault, KEEP_SETTING)
                .ok()
                .flatten()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            let removed = if keep > 0 {
                prune(&parent, &dest, &manifest.vault_id, keep)
            } else {
                Vec::new()
            };
            for entry in &mut entries {
                if removed.contains(&entry.path) {
                    entry.pruned = true;
                }
            }
            let _ = save_log(vault, entries);
            Ok(removed)
        })
        .unwrap_or_default();

    Ok(BackupResult {
        path: dest.display().to_string(),
        created_at: manifest.created_at,
        objects: manifest.objects.len(),
        pruned,
    })
}

/// Keep the newest `keep` backups of this vault in `parent`; remove older
/// ones. Only folders this app named, whose manifest says they are this
/// vault's, are ever considered — nothing else in the folder is touched —
/// and the backup just made is never removed.
fn prune(parent: &Path, just_made: &Path, vault_id: &str, keep: usize) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(parent) else { return Vec::new() };
    let mut ours: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| e.file_name().to_string_lossy().starts_with(FOLDER_PREFIX))
        .filter_map(|e| {
            let manifest = am_storage::vault::read_manifest(&e.path()).ok()?;
            (manifest.vault_id == vault_id).then(|| (manifest.created_at, e.path()))
        })
        .collect();
    ours.sort_by(|a, b| b.0.cmp(&a.0));
    let mut removed = Vec::new();
    for (_, path) in ours.into_iter().skip(keep) {
        if path == just_made {
            continue;
        }
        if std::fs::remove_dir_all(&path).is_ok() {
            removed.push(path.display().to_string());
        }
    }
    removed
}

#[derive(Serialize)]
pub struct BackupCentre {
    pub folder: Option<String>,
    pub folder_available: bool,
    /// Backups of this vault kept in the folder; 0 keeps them all.
    pub keep: usize,
    pub reminder_days: u32,
    pub last_backup_at: Option<String>,
    /// The most recent verification that succeeded.
    pub last_verified_at: Option<String>,
    pub history: Vec<BackupView>,
}

#[derive(Serialize)]
pub struct BackupView {
    #[serde(flatten)]
    pub entry: BackupEntry,
    /// Whether the folder is still where it was (a drive may be unplugged).
    pub present: bool,
}

#[tauri::command]
pub fn backup_centre(session: State<'_, Session>) -> IpcResult<BackupCentre> {
    session.touch();
    session
        .with_vault(|vault| {
            let get = |k: &str| am_storage::settings::get(vault, k).ok().flatten();
            let entries = log(vault);
            let folder = get(FOLDER_SETTING);
            Ok(BackupCentre {
                folder_available: folder.as_deref().is_some_and(|f| Path::new(f).is_dir()),
                folder,
                keep: get(KEEP_SETTING).and_then(|v| v.parse().ok()).unwrap_or(0),
                reminder_days: reminder_days(vault),
                last_backup_at: get(crate::commands::LAST_BACKUP_SETTING),
                last_verified_at: entries
                    .iter()
                    .filter(|e| e.verified_ok == Some(true))
                    .filter_map(|e| e.verified_at.clone())
                    .max(),
                history: entries
                    .into_iter()
                    .map(|entry| BackupView {
                        present: !entry.pruned
                            && Path::new(&entry.path).join("manifest.json").exists(),
                        entry,
                    })
                    .collect(),
            })
        })
        .map_err(IpcError::from)
}

#[derive(Deserialize)]
pub struct BackupPreferences {
    pub folder: Option<String>,
    pub keep: usize,
    pub reminder_days: u32,
}

#[tauri::command]
pub fn update_backup_preferences(
    app: AppHandle<impl Runtime>,
    session: State<'_, Session>,
    preferences: BackupPreferences,
) -> IpcResult<()> {
    session.touch();
    if preferences.keep > 1000 || preferences.reminder_days > 3650 {
        return Err(bad_input("those numbers are out of range"));
    }
    if preferences.keep == 1 {
        return Err(bad_input(
            "keep at least two, so a backup that turns out bad is not the only one",
        ));
    }
    let root = vault_root(&app).map_err(other)?;
    if let Some(folder) = &preferences.folder {
        let path = PathBuf::from(folder);
        if !path.is_dir() {
            return Err(bad_input("choose an existing folder"));
        }
        if path.starts_with(&root) {
            return Err(bad_input("choose a folder outside the vault itself"));
        }
    }
    session
        .with_vault(|vault| {
            let set =
                |k: &str, v: &str| am_storage::settings::set(vault, k, v).map_err(storage);
            match &preferences.folder {
                Some(f) => set(FOLDER_SETTING, f)?,
                None => {
                    vault
                        .conn()
                        .execute("DELETE FROM app_settings WHERE key = ?1", [FOLDER_SETTING])
                        .map_err(storage)?;
                }
            }
            set(KEEP_SETTING, &preferences.keep.to_string())?;
            set(REMINDER_SETTING, &preferences.reminder_days.to_string())?;
            Ok(())
        })
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct Verification {
    pub path: String,
    pub created_at: String,
    pub objects: usize,
    pub decrypted: usize,
    pub assets: i64,
    /// Whether the backup is of the vault that is open now.
    pub this_vault: bool,
    pub verified_at: String,
}

/// Restore a backup into a scratch folder, decrypt every photo and document,
/// count what is there, and throw the copy away. The vault in use is not
/// touched. The result is recorded in the backup history.
#[tauri::command]
pub fn verify_backup<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    directory: String,
    secret: String,
    use_recovery_key: bool,
) -> IpcResult<Verification> {
    session.touch();
    let root = vault_root(&app).map_err(other)?;
    let backup = PathBuf::from(&directory);
    if backup.starts_with(&root) || root.starts_with(&backup) {
        return Err(bad_input("that folder is the vault itself, not a backup of it"));
    }
    if !crate::commands::is_backup_folder(&backup) {
        return Err(bad_input("that folder is not an Asset Manager backup"));
    }
    let credential =
        if use_recovery_key { Credential::RecoveryKey } else { Credential::Passphrase };
    let scratch = root.with_extension("verify");
    let verified_at = now();
    let outcome = am_storage::vault::verify_backup(&backup, &scratch, credential, &secret);

    // Record the result either way: a backup that fails to verify is the
    // most important line in the history.
    let manifest = am_storage::vault::read_manifest(&backup).ok();
    let this_vault = session
        .with_vault(|vault| {
            let id: String = vault.vault_id().iter().map(|b| format!("{b:02x}")).collect();
            let mut entries = log(vault);
            let created_at =
                manifest.as_ref().map(|m| m.created_at.clone()).unwrap_or_default();
            let path = backup.display().to_string();
            let entry = match entries.iter_mut().find(|e| e.path == path) {
                Some(e) => e,
                None => {
                    entries.push(BackupEntry {
                        path: path.clone(),
                        created_at,
                        objects: manifest.as_ref().map(|m| m.objects.len()).unwrap_or(0),
                        verified_at: None,
                        verified_ok: None,
                        verify_note: None,
                        pruned: false,
                    });
                    entries.last_mut().expect("just pushed")
                }
            };
            entry.verified_at = Some(verified_at.clone());
            entry.verified_ok = Some(outcome.is_ok());
            entry.verify_note = outcome.as_ref().err().map(|e| match e {
                am_storage::vault::VaultError::Unlock(_) => {
                    "did not open with the credential given".to_string()
                }
                other => other.to_string(),
            });
            let ours = manifest.as_ref().is_some_and(|m| m.vault_id == id);
            save_log(vault, entries)?;
            Ok(ours)
        })
        .map_err(IpcError::from)?;

    let report = outcome.map_err(|e| IpcError::from(SessionError::Vault(e)))?;
    Ok(Verification {
        path: backup.display().to_string(),
        created_at: report.manifest.created_at,
        objects: report.objects,
        decrypted: report.decrypted,
        assets: report.assets,
        this_vault,
        verified_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_folders_are_dated_and_filesystem_safe() {
        let name = backup_folder_name("2026-09-22T14:03:11Z");
        assert_eq!(name, "Asset Manager backup 2026-09-22 14-03-11");
        assert!(!name.contains(':'), "colons are invalid in Windows paths");
    }

    #[test]
    fn pruning_touches_only_this_vaults_backups_and_never_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str, vault_id: &str, created_at: &str| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(&path).unwrap();
            let manifest = serde_json::json!({
                "format": am_storage::header::FORMAT_TAG, "manifest_version": 2,
                "format_version": 1, "schema_version": 1, "vault_id": vault_id,
                "key_epoch": 1, "created_at": created_at, "objects": []
            });
            std::fs::write(path.join("manifest.json"), manifest.to_string()).unwrap();
            path
        };
        let ours = "aa".repeat(16);
        let theirs = "bb".repeat(16);
        let old = make("Asset Manager backup 1", &ours, "2026-01-01T00:00:00Z");
        let mid = make("Asset Manager backup 2", &ours, "2026-02-01T00:00:00Z");
        let new = make("Asset Manager backup 3", &ours, "2026-03-01T00:00:00Z");
        let other_vault = make("Asset Manager backup 0", &theirs, "2025-01-01T00:00:00Z");
        let unrelated = make("Holiday photos", &ours, "2020-01-01T00:00:00Z");

        let removed = prune(dir.path(), &new, &ours, 2);
        assert_eq!(removed, vec![old.display().to_string()]);
        assert!(mid.exists() && new.exists());
        assert!(other_vault.exists(), "another vault's backups are not ours to remove");
        assert!(unrelated.exists(), "only folders this app named");
    }
}
