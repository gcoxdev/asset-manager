//! Splitting a holding, sets, and dividing a purchase among items.

use am_storage::assets::AssetError;
use am_storage::bundles::{self, SetMember, SetSummary};
use tauri::State;

use crate::ipc::{
    atomically, bad_input, base_currency, currency_or, now, parse_money, storage, today,
    IpcResult,
};
use crate::session::{IpcError, Session, SessionError};

fn asset_err(e: AssetError) -> SessionError {
    match e {
        AssetError::Sqlite(e) => storage(e),
        other => SessionError::Vault(am_storage::vault::VaultError::Other(other.to_string())),
    }
}

/// Make part of a holding an item of its own. Returns the new item's ID.
#[tauri::command]
pub fn split_asset(
    session: State<'_, Session>,
    asset_id: String,
    quantity: String,
    name: String,
) -> IpcResult<String> {
    session.touch();
    let quantity = am_storage::assets::parse_quantity(quantity.trim())
        .map_err(|e| bad_input(e.to_string()))?;
    let timestamp = now();
    session
        .with_vault(|vault| {
            atomically(vault, || {
                let id =
                    bundles::split(vault, &asset_id, quantity, &name, &today(), &timestamp)
                        .map_err(asset_err)?;
                // A market-priced holding values both parts from the market
                // at their new quantities.
                for asset in [&asset_id, &id] {
                    am_storage::pricing::revalue_asset(vault, asset, &timestamp, &today())
                        .map_err(storage)?;
                }
                Ok(id)
            })
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn list_sets(session: State<'_, Session>) -> IpcResult<Vec<SetSummary>> {
    session.touch();
    session
        .with_vault(|vault| {
            bundles::sets(vault, &base_currency(vault), &today()).map_err(asset_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn set_members(session: State<'_, Session>, set_id: String) -> IpcResult<Vec<SetMember>> {
    session.touch();
    session
        .with_vault(|vault| bundles::members(vault, &set_id).map_err(asset_err))
        .map_err(IpcError::from)
}

/// Create a set (no ID) or update one, then add any members given.
#[tauri::command]
pub fn save_set(
    session: State<'_, Session>,
    set_id: Option<String>,
    name: String,
    target_count: Option<i64>,
    notes: Option<String>,
    add: Option<Vec<String>>,
) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    let notes = notes.unwrap_or_default();
    session
        .with_vault(|vault| {
            atomically(vault, || {
                let id = match &set_id {
                    Some(id) => {
                        bundles::update_set(vault, id, &name, target_count, &notes)
                            .map_err(asset_err)?;
                        id.clone()
                    }
                    None => bundles::create_set(vault, &name, target_count, &notes, &timestamp)
                        .map_err(asset_err)?,
                };
                if let Some(ids) = &add {
                    bundles::add_members(vault, &id, ids, &timestamp).map_err(asset_err)?;
                }
                Ok(id)
            })
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn remove_from_set(
    session: State<'_, Session>,
    set_id: String,
    asset_id: String,
) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| {
            bundles::remove_member(vault, &set_id, &asset_id).map_err(asset_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_set(session: State<'_, Session>, set_id: String) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| bundles::delete_set(vault, &set_id).map_err(asset_err))
        .map_err(IpcError::from)
}

/// Divide one purchase's price among the items it bought. Returns each
/// item's share in minor units.
#[tauri::command]
pub fn allocate_purchase(
    session: State<'_, Session>,
    asset_ids: Vec<String>,
    total: String,
    currency: Option<String>,
    by_value: bool,
) -> IpcResult<Vec<(String, String)>> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            let currency = currency_or(&currency, &base_currency(vault))
                .map_err(|e| storage(e.message))?;
            let total = parse_money(&total, &currency).map_err(|e| storage(e.message))?;
            let shares =
                bundles::allocate_cost(vault, &asset_ids, &total, by_value, &timestamp)
                    .map_err(asset_err)?;
            Ok(shares
                .into_iter()
                .map(|(id, minor)| {
                    (id, am_core::Money::new(minor, total.currency.clone()).format())
                })
                .collect())
        })
        .map_err(IpcError::from)
}
