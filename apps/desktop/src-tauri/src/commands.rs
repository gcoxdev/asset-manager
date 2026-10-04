//! Vault lifecycle, credentials, backup and settings.
//!
//! Every command that touches vault data goes through [`Session::with_vault`],
//! which rejects while locked. There is no read path around it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use am_storage::header::Credential;
use am_storage::vault::VaultError;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::ipc::{bad_input, base_currency, now, storage, IpcResult, CURRENCY_SETTING};
use crate::paths::vault_root;
use crate::session::{default_params, IpcError, Session, SessionError};

/// Argon2id slows guessing; it cannot compensate for a short passphrase.
/// QiRing settled on the same floor.
pub const MIN_PASSPHRASE_CHARS: usize = 12;

const AUTO_LOCK_SETTING: &str = "auto_lock_minutes";
const METALS_AUTO_SETTING: &str = "metals_auto_refresh";
/// Set when a recovery key is issued, cleared when the ceremony confirms it
/// was saved. Still set after unlock means the ceremony never finished — the
/// vault locked, or the app closed, while the key was on screen — and that
/// key is gone for good, so the owner is asked to issue another.
const RECOVERY_UNCONFIRMED_SETTING: &str = "recovery_key_unconfirmed";

fn other(message: String) -> IpcError {
    IpcError { kind: "error".into(), message }
}

fn check_passphrase_strength(passphrase: &str) -> IpcResult<()> {
    // Enforced in the backend: the UI can be bypassed, this cannot.
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(IpcError {
            kind: "weak_passphrase".into(),
            message: format!(
                "Use at least {MIN_PASSPHRASE_CHARS} characters. \
                 Argon2id slows guessing but cannot rescue a short passphrase."
            ),
        });
    }
    Ok(())
}

#[derive(Serialize)]
pub struct VaultStatus {
    pub unlocked: bool,
    pub exists: bool,
}

#[derive(Serialize)]
pub struct CreatedVault {
    /// Shown once, in the recovery ceremony. Never stored anywhere.
    pub recovery_key: String,
    pub fingerprint: String,
}

#[tauri::command]
pub fn vault_status<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
) -> IpcResult<VaultStatus> {
    let root = vault_root(&app).map_err(other)?;
    let exists = root.join(am_storage::vault::HEADER_FILE).exists();
    Ok(VaultStatus { unlocked: session.is_unlocked(), exists })
}

#[tauri::command]
pub fn create_vault<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    passphrase: String,
) -> IpcResult<CreatedVault> {
    check_passphrase_strength(&passphrase)?;
    let root = vault_root(&app).map_err(other)?;
    let recovery_key = session
        .create(&root, &passphrase, &default_params(), &now())
        .map_err(IpcError::from)?;
    mark_recovery_unconfirmed(&session)?;
    let fingerprint = am_crypto::recovery_fingerprint(&recovery_key);
    Ok(CreatedVault { recovery_key, fingerprint })
}

fn mark_recovery_unconfirmed(session: &Session) -> IpcResult<()> {
    session
        .with_vault(|vault| {
            am_storage::settings::set(vault, RECOVERY_UNCONFIRMED_SETTING, "true")
                .map_err(storage)
        })
        .map_err(IpcError::from)
}

/// The recovery ceremony finished: the key was acknowledged and retyped.
#[tauri::command]
pub fn confirm_recovery_saved(session: State<'_, Session>) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| {
            am_storage::settings::set(vault, RECOVERY_UNCONFIRMED_SETTING, "false")
                .map_err(storage)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn unlock_vault<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    secret: String,
    use_recovery_key: bool,
) -> IpcResult<()> {
    let root = vault_root(&app).map_err(other)?;
    let credential =
        if use_recovery_key { Credential::RecoveryKey } else { Credential::Passphrase };
    session.unlock(&root, credential, &secret).map_err(IpcError::from)?;
    apply_session_settings(&session);
    maybe_poll_metals(app);
    Ok(())
}

/// Apply per-vault settings that live in the session, not the database.
fn apply_session_settings(session: &Session) {
    let minutes = session
        .with_vault(|vault| {
            Ok(am_storage::settings::get(vault, AUTO_LOCK_SETTING).ok().flatten())
        })
        .ok()
        .flatten()
        .and_then(|m| m.parse::<u64>().ok());
    session.set_idle_limit(match minutes {
        Some(m) => Duration::from_secs(m * 60),
        None => crate::session::AUTO_LOCK_IDLE,
    });
}

/// Refresh metal prices in the background after unlock, if the owner opted
/// in and the budget allows it.
///
/// Runs on its own thread so unlock is never slowed by the network, and the
/// fetch holds no vault lock while it waits. If the vault locks before the
/// response arrives, the write fails with `Locked` and is simply dropped —
/// a late result cannot repopulate a locked session.
fn maybe_poll_metals<R: Runtime>(app: AppHandle<R>) {
    let session = app.state::<Session>();
    let wanted = session
        .with_vault(|vault| {
            let enabled =
                am_storage::settings::get(vault, METALS_AUTO_SETTING).ok().flatten().as_deref()
                    == Some("true");
            if !enabled || !crate::metals_provider::has_api_key() {
                return Ok(false);
            }
            am_storage::spot::should_poll_automatically(
                vault,
                am_core::Metal::Gold,
                crate::metals_commands::METALS_PROVIDER,
                &now(),
            )
            .map_err(storage)
        })
        .unwrap_or(false);
    if !wanted {
        return;
    }

    std::thread::spawn(move || {
        let session = app.state::<Session>();
        if crate::metals_commands::refresh_metals(&session, true).is_ok() {
            let _ = app.emit("prices-updated", ());
        }
    });
}

#[tauri::command]
pub fn lock_vault(session: State<'_, Session>) {
    session.lock();
}

/// Whether the vault is unlocked, and how long it has been idle. The frontend
/// polls this so it can warn before an auto-lock rather than dropping the user
/// out mid-edit.
#[derive(Serialize)]
pub struct SessionState {
    pub unlocked: bool,
    pub idle_seconds: Option<u64>,
    pub auto_lock_seconds: u64,
}

#[tauri::command]
pub fn session_state(session: State<'_, Session>) -> SessionState {
    SessionState {
        unlocked: session.is_unlocked(),
        idle_seconds: session.idle_for().map(|d| d.as_secs()),
        auto_lock_seconds: session.idle_limit().as_secs(),
    }
}

/// Reset the idle clock on user activity that does not otherwise reach the
/// backend — scrolling, reading, typing into a form.
#[tauri::command]
pub fn keep_alive(session: State<'_, Session>) {
    session.touch();
}

// ------------------------------------------------------------ credentials

/// Change the passphrase. The current one is required even though the vault
/// is open: an unlocked, unattended session must not be enough to take it
/// over.
#[tauri::command]
pub fn change_passphrase(
    session: State<'_, Session>,
    current: String,
    new_passphrase: String,
) -> IpcResult<()> {
    session.touch();
    check_passphrase_strength(&new_passphrase)?;
    if current == new_passphrase {
        return Err(bad_input("the new passphrase is the same as the current one"));
    }
    session
        .with_vault_mut(|vault| {
            if !vault.verify(Credential::Passphrase, &current) {
                return Err(SessionError::Vault(VaultError::Other(
                    "the current passphrase is not correct".into(),
                )));
            }
            vault.change_passphrase(&new_passphrase, &default_params())?;
            Ok(())
        })
        .map_err(IpcError::from)
}

/// Issue a new recovery key. The old one stops working for this vault — but
/// not for older backups, which still hold the old wrapper.
#[tauri::command]
pub fn rotate_recovery_key(
    session: State<'_, Session>,
    passphrase: String,
) -> IpcResult<CreatedVault> {
    session.touch();
    session
        .with_vault_mut(|vault| {
            if !vault.verify(Credential::Passphrase, &passphrase) {
                return Err(SessionError::Vault(VaultError::Other(
                    "the passphrase is not correct".into(),
                )));
            }
            let recovery_key = vault.rotate_recovery_key(&default_params())?;
            am_storage::settings::set(vault, RECOVERY_UNCONFIRMED_SETTING, "true")
                .map_err(storage)?;
            let fingerprint = am_crypto::recovery_fingerprint(&recovery_key);
            Ok(CreatedVault { recovery_key, fingerprint })
        })
        .map_err(IpcError::from)
}

// ------------------------------------------------------------ backup

const LAST_BACKUP_SETTING: &str = "last_backup_at";

#[derive(Serialize)]
pub struct BackupResult {
    pub path: String,
    pub created_at: String,
    pub objects: usize,
}

/// A folder name for a new backup, dated so a directory of them sorts.
fn backup_folder_name(now: &str) -> String {
    let stamp: String = now[..19].chars().map(|c| if c == ':' { '-' } else { c }).collect();
    format!("Asset Manager backup {}", stamp.replace('T', " "))
}

/// Back up the vault into a new folder inside `directory`.
///
/// The copy is ciphertext throughout — header, database and objects — so it
/// is as safe to keep on an external drive or a synced folder as the vault
/// itself. It opens with the passphrase or recovery key that were current
/// when it was made.
#[tauri::command]
pub fn backup_vault<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    directory: String,
) -> IpcResult<BackupResult> {
    session.touch();
    let root = vault_root(&app).map_err(other)?;
    let parent = PathBuf::from(&directory);
    if !parent.is_dir() {
        return Err(bad_input("choose an existing folder for the backup"));
    }
    if parent.starts_with(&root) {
        return Err(bad_input("choose a folder outside the vault itself"));
    }

    let timestamp = now();
    let dest = parent.join(backup_folder_name(&timestamp));
    let manifest = session.backup(&dest, &timestamp).map_err(IpcError::from)?;

    // Recorded so settings can say how long it has been. Best effort: the
    // backup itself already succeeded.
    let _ = session.with_vault(|vault| {
        am_storage::settings::set(vault, LAST_BACKUP_SETTING, &timestamp).map_err(storage)
    });

    Ok(BackupResult {
        path: dest.display().to_string(),
        created_at: manifest.created_at,
        objects: manifest.objects.len(),
    })
}

#[derive(Serialize)]
pub struct RestoreResult {
    pub created_at: String,
    pub objects: usize,
    /// Whether the restored vault was opened. False when the session
    /// auto-locked while a long restore was being verified: the screen has
    /// already gone to the lock screen, so the restored vault waits there too.
    pub opened: bool,
}

/// Replace the vault with a backup, and open it.
///
/// Takes the backup's own passphrase or recovery key: the copy is unlocked,
/// integrity-checked and its photos verified *before* anything is replaced,
/// so a damaged backup — or one the owner cannot open — never displaces a
/// working vault. Only then does the session lock and the swap happen. The
/// previous vault is kept beside it as `vault.pre-restore` until the next
/// restore, so a wrong choice here is still recoverable.
#[tauri::command]
pub fn restore_vault<R: Runtime>(
    app: AppHandle<R>,
    session: State<'_, Session>,
    directory: String,
    secret: String,
    use_recovery_key: bool,
) -> IpcResult<RestoreResult> {
    let root = vault_root(&app).map_err(other)?;
    let backup = PathBuf::from(&directory);
    if backup.starts_with(&root) || root.starts_with(&backup) {
        return Err(bad_input("that folder is the vault itself, not a backup of it"));
    }
    if !is_backup_folder(&backup) {
        return Err(bad_input(
            "that folder is not an Asset Manager backup — choose the folder that \
             contains manifest.json",
        ));
    }
    if secret.is_empty() {
        return Err(bad_input("enter the passphrase or recovery key the backup opens with"));
    }
    session.touch();
    let was_unlocked = session.is_unlocked();
    let credential =
        if use_recovery_key { Credential::RecoveryKey } else { Credential::Passphrase };

    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent).map_err(|e| other(e.to_string()))?;
    }
    let staged = am_storage::vault::stage_restore(&backup, &root, credential, &secret)
        .map_err(|e| IpcError::from(SessionError::Vault(e)))?;

    // Restoring underneath an open vault would leave the session pointing at
    // files that no longer exist.
    let locked_meanwhile = was_unlocked && !session.is_unlocked();
    session.lock();
    let report = staged.commit().map_err(|e| IpcError::from(SessionError::Vault(e)))?;
    if !locked_meanwhile {
        session.unlock(&root, credential, &secret).map_err(IpcError::from)?;
        apply_session_settings(&session);
    }
    Ok(RestoreResult {
        created_at: report.manifest.created_at,
        objects: report.objects,
        opened: !locked_meanwhile,
    })
}

// ------------------------------------------------------------ settings

#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub currency: String,
    pub auto_lock_minutes: u64,
    pub metals_auto_refresh: bool,
    pub balance_lookup: bool,
    /// Read-only here; written by `backup_vault`.
    #[serde(default)]
    pub last_backup_at: Option<String>,
    /// Read-only here: the last recovery key issued was never confirmed as
    /// saved. See [`RECOVERY_UNCONFIRMED_SETTING`].
    #[serde(default)]
    pub recovery_unconfirmed: bool,
}

#[tauri::command]
pub fn get_settings(session: State<'_, Session>) -> IpcResult<Settings> {
    session.touch();
    let limit = session.idle_limit();
    session
        .with_vault(|vault| {
            let get = |k: &str| am_storage::settings::get(vault, k).ok().flatten();
            Ok(Settings {
                currency: base_currency(vault).code().to_string(),
                auto_lock_minutes: limit.as_secs() / 60,
                metals_auto_refresh: get(METALS_AUTO_SETTING).as_deref() == Some("true"),
                balance_lookup: get(crate::crypto_commands::BALANCE_OPT_IN_KEY).as_deref()
                    == Some("true"),
                last_backup_at: get(LAST_BACKUP_SETTING),
                recovery_unconfirmed: get(RECOVERY_UNCONFIRMED_SETTING).as_deref()
                    == Some("true"),
            })
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn update_settings(session: State<'_, Session>, settings: Settings) -> IpcResult<()> {
    session.touch();
    let currency = am_core::Currency::new(&settings.currency)
        .map_err(|_| bad_input("currency must be a three-letter code such as USD"))?;
    let minutes = settings.auto_lock_minutes;
    let limit = Duration::from_secs(minutes * 60);
    if limit < crate::session::MIN_AUTO_LOCK || limit > crate::session::MAX_AUTO_LOCK {
        return Err(bad_input("auto-lock must be between 1 minute and 4 hours"));
    }

    session
        .with_vault(|vault| {
            let set =
                |k: &str, v: &str| am_storage::settings::set(vault, k, v).map_err(storage);
            set(CURRENCY_SETTING, currency.code())?;
            set(AUTO_LOCK_SETTING, &minutes.to_string())?;
            set(
                METALS_AUTO_SETTING,
                if settings.metals_auto_refresh { "true" } else { "false" },
            )?;
            set(
                crate::crypto_commands::BALANCE_OPT_IN_KEY,
                if settings.balance_lookup { "true" } else { "false" },
            )?;
            Ok(())
        })
        .map_err(IpcError::from)?;
    session.set_idle_limit(limit);
    Ok(())
}

// ------------------------------------------------------------ vault info

#[derive(Serialize)]
pub struct VaultInfo {
    pub location: String,
    pub created_at: String,
    pub recovery_fingerprint: String,
    pub key_epoch: u64,
    pub schema_version: i64,
    pub asset_count: i64,
    pub photo_count: i64,
    /// Encrypted bytes on disk for photos, as text (JS numbers are 53-bit).
    pub photo_bytes: String,
}

#[tauri::command]
pub fn vault_info(session: State<'_, Session>) -> IpcResult<VaultInfo> {
    session.touch();
    session
        .with_vault(|vault| {
            let header = vault.header();
            let count = |sql: &str| -> Result<i64, SessionError> {
                vault.conn().query_row(sql, [], |r| r.get(0)).map_err(storage)
            };
            Ok(VaultInfo {
                location: vault.root().display().to_string(),
                created_at: header.created_at.clone(),
                recovery_fingerprint: header.recovery_fingerprint.clone(),
                key_epoch: header.key_epoch,
                schema_version: am_storage::migrate::SCHEMA_VERSION,
                asset_count: count("SELECT count(*) FROM assets")?,
                photo_count: count("SELECT count(*) FROM objects WHERE gc_state = 'live'")?,
                photo_bytes: count(
                    "SELECT coalesce(sum(ciphertext_bytes), 0) FROM objects WHERE gc_state = 'live'",
                )?
                .to_string(),
            })
        })
        .map_err(IpcError::from)
}

/// Whether a path looks like a backup folder, for the restore picker.
pub fn is_backup_folder(path: &Path) -> bool {
    path.join("manifest.json").exists() && path.join(am_storage::vault::HEADER_FILE).exists()
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
    fn short_passphrases_are_refused_in_the_backend() {
        assert!(check_passphrase_strength("too short").is_err());
        assert!(check_passphrase_strength("long enough passphrase").is_ok());
        // Counted in characters, not bytes: "é" is two bytes.
        assert!(check_passphrase_strength("éééééééééééé").is_ok());
    }

    #[test]
    fn a_backup_folder_needs_both_manifest_and_header() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_backup_folder(dir.path()));
        std::fs::write(dir.path().join("manifest.json"), "{}").unwrap();
        assert!(!is_backup_folder(dir.path()));
        std::fs::write(dir.path().join(am_storage::vault::HEADER_FILE), "{}").unwrap();
        assert!(is_backup_folder(dir.path()));
    }
}
