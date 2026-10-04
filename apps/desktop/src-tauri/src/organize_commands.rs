//! Tags, locations, saved views, duplicates, and changes to many assets.

use am_storage::assets::{self, AssetError};
use am_storage::organize::{self, BulkChange, SavedView, TagCount};
use tauri::State;

use crate::ipc::{atomically, now, storage, today, IpcResult};
use crate::session::{IpcError, Session, SessionError};

fn asset_err(e: AssetError) -> SessionError {
    match e {
        AssetError::Sqlite(e) => storage(e),
        other => SessionError::Vault(am_storage::vault::VaultError::Other(other.to_string())),
    }
}

#[tauri::command]
pub fn list_tags(session: State<'_, Session>) -> IpcResult<Vec<TagCount>> {
    session.touch();
    session.with_vault(|vault| organize::tags(vault).map_err(asset_err)).map_err(IpcError::from)
}

#[tauri::command]
pub fn set_asset_tags(
    session: State<'_, Session>,
    asset_id: String,
    tags: Vec<String>,
) -> IpcResult<()> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            organize::set_tags(vault, &asset_id, &tags, &timestamp).map_err(asset_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn rename_tag(session: State<'_, Session>, from: String, to: String) -> IpcResult<()> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            organize::rename_tag(vault, &from, &to, &timestamp).map_err(asset_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn list_locations(session: State<'_, Session>) -> IpcResult<Vec<TagCount>> {
    session.touch();
    session
        .with_vault(|vault| organize::locations(vault).map_err(asset_err))
        .map_err(IpcError::from)
}

/// Move everything at a location — and in places inside it — to a new name.
#[tauri::command]
pub fn rename_location(
    session: State<'_, Session>,
    from: String,
    to: String,
) -> IpcResult<usize> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            organize::rename_location(vault, &from, &to, &timestamp).map_err(asset_err)
        })
        .map_err(IpcError::from)
}

/// One change to many assets: all of them, or none.
#[tauri::command]
pub fn bulk_edit(
    session: State<'_, Session>,
    asset_ids: Vec<String>,
    change: BulkChange,
) -> IpcResult<usize> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            organize::bulk_edit(vault, &asset_ids, &change, &timestamp).map_err(asset_err)
        })
        .map_err(IpcError::from)
}

/// Move many assets to the trash at once.
#[tauri::command]
pub fn bulk_trash(session: State<'_, Session>, asset_ids: Vec<String>) -> IpcResult<usize> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            atomically(vault, || {
                for id in &asset_ids {
                    assets::trash(vault, id, &timestamp).map_err(asset_err)?;
                }
                Ok(asset_ids.len())
            })
        })
        .map_err(IpcError::from)
}

/// A new asset described like an existing one — never with its serial or
/// certificate number, value, history or files.
#[tauri::command]
pub fn duplicate_asset(session: State<'_, Session>, asset_id: String) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            atomically(vault, || {
                let id = organize::duplicate(vault, &asset_id, &today(), &timestamp)
                    .map_err(asset_err)?;
                am_storage::pricing::revalue_asset(vault, &id, &timestamp, &today())
                    .map_err(storage)?;
                Ok(id)
            })
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn list_saved_views(session: State<'_, Session>) -> IpcResult<Vec<SavedView>> {
    session.touch();
    session
        .with_vault(|vault| organize::saved_views(vault).map_err(asset_err))
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn save_view(
    session: State<'_, Session>,
    name: String,
    view: serde_json::Value,
) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| organize::save_view(vault, &name, view).map_err(asset_err))
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_saved_view(session: State<'_, Session>, name: String) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| organize::delete_view(vault, &name).map_err(asset_err))
        .map_err(IpcError::from)
}
