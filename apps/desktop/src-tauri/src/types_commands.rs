//! Item types the owner defines.

use am_storage::custom_types::{self, CustomType, CustomTypeError};
use tauri::State;

use crate::ipc::{now, storage, IpcResult};
use crate::session::{IpcError, Session, SessionError};

fn type_err(e: CustomTypeError) -> SessionError {
    match e {
        CustomTypeError::Sqlite(e) => storage(e),
        CustomTypeError::Invalid(m) => {
            SessionError::Vault(am_storage::vault::VaultError::Other(m))
        }
    }
}

#[tauri::command]
pub fn list_custom_types(session: State<'_, Session>) -> IpcResult<Vec<CustomType>> {
    session.touch();
    session
        .with_vault(|vault| custom_types::list(vault).map_err(type_err))
        .map_err(IpcError::from)
}

/// Create a type, or update one when its `type_id` is set.
#[tauri::command]
pub fn save_custom_type(
    session: State<'_, Session>,
    definition: CustomType,
) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            custom_types::save(vault, &definition, &timestamp).map_err(type_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_custom_type(session: State<'_, Session>, type_id: String) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| custom_types::delete(vault, &type_id).map_err(type_err))
        .map_err(IpcError::from)
}
