//! Physical inventory checks and QR labels.

use am_storage::inventory::{self, Check, CheckItem, InventoryError};
use serde::Serialize;
use tauri::State;

use crate::ipc::{now, storage, IpcResult};
use crate::session::{IpcError, Session, SessionError};

fn inv_err(e: InventoryError) -> SessionError {
    match e {
        InventoryError::Sqlite(e) => storage(e),
        InventoryError::Invalid(m) => {
            SessionError::Vault(am_storage::vault::VaultError::Other(m))
        }
    }
}

#[tauri::command]
pub fn list_checks(session: State<'_, Session>) -> IpcResult<Vec<Check>> {
    session.touch();
    session.with_vault(|vault| inventory::list(vault).map_err(inv_err)).map_err(IpcError::from)
}

#[tauri::command]
pub fn start_check(
    session: State<'_, Session>,
    name: String,
    location: Option<String>,
) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            inventory::start(vault, &name, location.as_deref(), &timestamp).map_err(inv_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn check_items(session: State<'_, Session>, check_id: String) -> IpcResult<Vec<CheckItem>> {
    session.touch();
    session
        .with_vault(|vault| inventory::items(vault, &check_id).map_err(inv_err))
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn mark_item(
    session: State<'_, Session>,
    check_id: String,
    asset_id: String,
    result: Option<String>,
    counted: Option<String>,
) -> IpcResult<()> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            inventory::mark(
                vault,
                &check_id,
                &asset_id,
                result.as_deref(),
                counted.as_deref(),
                &timestamp,
            )
            .map_err(inv_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn finish_check(session: State<'_, Session>, check_id: String) -> IpcResult<()> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| inventory::finish(vault, &check_id, &timestamp).map_err(inv_err))
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn reconcile_check_count(
    session: State<'_, Session>,
    check_id: String,
    asset_id: String,
) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| {
            inventory::reconcile(vault, &check_id, &asset_id, &crate::ipc::today(), &now())
                .map_err(inv_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_check(session: State<'_, Session>, check_id: String) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| inventory::delete(vault, &check_id).map_err(inv_err))
        .map_err(IpcError::from)
}

/// The asset a scanned label or typed short code names, if exactly one.
#[tauri::command]
pub fn resolve_label(session: State<'_, Session>, code: String) -> IpcResult<Option<String>> {
    session.touch();
    session
        .with_vault(|vault| inventory::resolve(vault, &code).map_err(inv_err))
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct Label {
    pub asset_id: String,
    pub name: String,
    /// What the QR encodes — the opaque ID and nothing else.
    pub payload: String,
    pub short_code: String,
}

#[tauri::command]
pub fn label_data(
    session: State<'_, Session>,
    asset_ids: Vec<String>,
) -> IpcResult<Vec<Label>> {
    session.touch();
    if asset_ids.len() > 2_000 {
        return Err(crate::ipc::bad_input("up to 2,000 labels at a time"));
    }
    session
        .with_vault(|vault| {
            asset_ids
                .iter()
                .map(|id| {
                    let r = am_storage::assets::get(vault, id).map_err(storage)?;
                    Ok(Label {
                        payload: inventory::label_payload(&r.asset_id),
                        short_code: inventory::short_code(&r.asset_id),
                        name: r.name,
                        asset_id: r.asset_id,
                    })
                })
                .collect()
        })
        .map_err(IpcError::from)
}
