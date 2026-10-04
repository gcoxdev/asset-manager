//! Loans, consignments, repairs, outside storage and shipping.

use am_storage::custody::{self, Away, CustodyEntry, CustodyError, NewCustody};
use tauri::State;

use crate::ipc::{now, storage, today, IpcResult};
use crate::session::{IpcError, Session, SessionError};

fn custody_err(e: CustodyError) -> SessionError {
    match e {
        CustodyError::Sqlite(e) => storage(e),
        CustodyError::Invalid(m) => {
            SessionError::Vault(am_storage::vault::VaultError::Other(m))
        }
    }
}

#[tauri::command]
pub fn list_custody(
    session: State<'_, Session>,
    asset_id: String,
) -> IpcResult<Vec<CustodyEntry>> {
    session.touch();
    session
        .with_vault(|vault| custody::history(vault, &asset_id).map_err(custody_err))
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn record_custody(session: State<'_, Session>, entry: NewCustody) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            custody::record(vault, &entry, &today(), &timestamp).map_err(custody_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_custody(session: State<'_, Session>, custody_id: String) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| custody::delete(vault, &custody_id).map_err(custody_err))
        .map_err(IpcError::from)
}

/// Everything away from home now, overdue returns first.
#[tauri::command]
pub fn away_list(session: State<'_, Session>) -> IpcResult<Vec<Away>> {
    session.touch();
    session
        .with_vault(|vault| custody::away(vault, &today()).map_err(custody_err))
        .map_err(IpcError::from)
}
