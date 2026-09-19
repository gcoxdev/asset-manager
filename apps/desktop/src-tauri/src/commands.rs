//! Tauri IPC commands.
//!
//! Every command that touches vault data goes through [`Session::with_vault`],
//! which rejects while locked. There is no read path around it.

use am_storage::header::Credential;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime, State};

use crate::paths::vault_root;
use crate::session::{default_params, IpcError, Session, SessionError};

type IpcResult<T> = Result<T, IpcError>;

/// Argon2id slows guessing; it cannot compensate for a short passphrase.
/// QiRing settled on the same floor.
pub const MIN_PASSPHRASE_CHARS: usize = 12;

/// ISO-8601 UTC. Stored as text so the format is unambiguous across
/// platforms and readable in a CSV export.
fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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

#[derive(Serialize)]
pub struct AssetSummary {
    pub asset_id: String,
    pub type_id: String,
    pub name: String,
    pub status: String,
    pub quantity: String,
    pub quantity_unit: String,
    pub storage_location: Option<String>,
    /// Money crosses IPC as a string: JavaScript `number` is 53-bit and would
    /// silently corrupt large values.
    pub current_amount_minor: Option<String>,
    pub current_currency: Option<String>,
}

#[derive(Deserialize)]
pub struct NewAsset {
    pub type_id: String,
    pub name: String,
    pub quantity: String,
    pub quantity_unit: String,
    pub storage_location: Option<String>,
    pub notes: Option<String>,
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

    let root = vault_root(&app).map_err(other)?;
    let recovery_key = session
        .create(&root, &passphrase, &default_params(), &now())
        .map_err(IpcError::from)?;
    let fingerprint = am_crypto::recovery_fingerprint(&recovery_key);
    Ok(CreatedVault { recovery_key, fingerprint })
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
    session.unlock(&root, credential, &secret).map_err(IpcError::from)
}

#[tauri::command]
pub fn lock_vault(session: State<'_, Session>) {
    session.lock();
}

#[tauri::command]
pub fn list_assets(session: State<'_, Session>) -> IpcResult<Vec<AssetSummary>> {
    session.touch();
    session
        .with_vault(|vault| {
            let mut stmt = vault
                .conn()
                .prepare(
                    "SELECT asset_id, type_id, name, status, quantity, quantity_unit,
                            storage_location, current_amount_minor, current_currency
                     FROM assets
                     ORDER BY updated_at DESC",
                )
                .map_err(sqlite)?;

            let rows = stmt
                .query_map([], |r| {
                    Ok(AssetSummary {
                        asset_id: r.get(0)?,
                        type_id: r.get(1)?,
                        name: r.get(2)?,
                        status: r.get(3)?,
                        quantity: r.get(4)?,
                        quantity_unit: r.get(5)?,
                        storage_location: r.get(6)?,
                        current_amount_minor: r
                            .get::<_, Option<i64>>(7)?
                            .map(|v| v.to_string()),
                        current_currency: r.get(8)?,
                    })
                })
                .map_err(sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sqlite)?;

            Ok(rows)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn create_asset(session: State<'_, Session>, asset: NewAsset) -> IpcResult<String> {
    session.touch();
    let asset_id = uuid::Uuid::new_v4().to_string();
    let timestamp = now();

    session
        .with_vault(|vault| {
            let sort = asset.quantity.parse::<f64>().unwrap_or(0.0);

            let tx = vault.conn().unchecked_transaction().map_err(sqlite)?;
            tx.execute(
                "INSERT INTO assets
                   (asset_id, type_id, name, quantity, quantity_sort, quantity_unit,
                    storage_location, notes, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                rusqlite::params![
                    &asset_id,
                    &asset.type_id,
                    &asset.name,
                    &asset.quantity,
                    sort,
                    &asset.quantity_unit,
                    &asset.storage_location,
                    asset.notes.as_deref().unwrap_or(""),
                    &timestamp
                ],
            )
            .map_err(sqlite)?;

            // Even in Phase 1a, ownership starts as an event. Phase 2 makes
            // this the source of truth; writing it now means no backfill over
            // encrypted data later.
            tx.execute(
                "INSERT INTO asset_events
                   (event_id, asset_id, event_type, effective_date, quantity_delta, recorded_at)
                 VALUES (?1, ?2, 'acquire', ?3, ?4, ?3)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    asset_id,
                    timestamp,
                    asset.quantity
                ],
            )
            .map_err(sqlite)?;

            tx.commit().map_err(sqlite)?;
            Ok(())
        })
        .map_err(IpcError::from)?;

    Ok(asset_id)
}

#[tauri::command]
pub fn search_assets(session: State<'_, Session>, query: String) -> IpcResult<Vec<AssetSummary>> {
    session.touch();
    session
        .with_vault(|vault| {
            let mut stmt = vault
                .conn()
                .prepare(
                    "SELECT a.asset_id, a.type_id, a.name, a.status, a.quantity, a.quantity_unit,
                            a.storage_location, a.current_amount_minor, a.current_currency
                     FROM assets_fts f
                     JOIN assets a ON a.rowid = f.rowid
                     WHERE assets_fts MATCH ?1
                     ORDER BY rank",
                )
                .map_err(sqlite)?;

            let rows = stmt
                .query_map([&query], |r| {
                    Ok(AssetSummary {
                        asset_id: r.get(0)?,
                        type_id: r.get(1)?,
                        name: r.get(2)?,
                        status: r.get(3)?,
                        quantity: r.get(4)?,
                        quantity_unit: r.get(5)?,
                        storage_location: r.get(6)?,
                        current_amount_minor: r
                            .get::<_, Option<i64>>(7)?
                            .map(|v| v.to_string()),
                        current_currency: r.get(8)?,
                    })
                })
                .map_err(sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sqlite)?;

            Ok(rows)
        })
        .map_err(IpcError::from)
}

fn other(message: String) -> IpcError {
    IpcError { kind: "error".into(), message }
}

fn sqlite(e: rusqlite::Error) -> SessionError {
    SessionError::Vault(am_storage::vault::VaultError::Sqlite(e))
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
        auto_lock_seconds: crate::session::AUTO_LOCK_IDLE.as_secs(),
    }
}
