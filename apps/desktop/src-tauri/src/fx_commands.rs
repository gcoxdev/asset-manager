//! Exchange rates the owner records. No rate is ever fetched: asking a
//! provider would tell it which currencies the owner holds.

use am_storage::fx::{self, FxError, Rate};
use tauri::State;

use crate::ipc::{now, storage, today, IpcResult};
use crate::session::{IpcError, Session, SessionError};

fn fx_err(e: FxError) -> SessionError {
    match e {
        FxError::Sqlite(e) => storage(e),
        FxError::Invalid(m) => SessionError::Vault(am_storage::vault::VaultError::Other(m)),
    }
}

#[tauri::command]
pub fn list_rates(session: State<'_, Session>) -> IpcResult<Vec<Rate>> {
    session.touch();
    session.with_vault(|vault| fx::list(vault).map_err(fx_err)).map_err(IpcError::from)
}

/// 1 `from` = `rate` `to`, as of a date (today if none).
#[tauri::command]
pub fn record_rate(
    session: State<'_, Session>,
    from: String,
    to: String,
    rate: String,
    asof: Option<String>,
) -> IpcResult<String> {
    session.touch();
    let today = today();
    let asof = asof.filter(|d| !d.trim().is_empty()).unwrap_or_else(|| today.clone());
    let timestamp = now();
    session
        .with_vault(|vault| {
            fx::record(vault, &from, &to, &rate, &asof, &today, &timestamp).map_err(fx_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_rate(session: State<'_, Session>, rate_id: String) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| fx::delete(vault, &rate_id).map_err(fx_err))
        .map_err(IpcError::from)
}
