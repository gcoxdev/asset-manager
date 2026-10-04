//! Care and service history for an asset, and what is due across them.

use am_storage::care::{self, CareEntry, CareError, NewCare};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::ipc::{
    base_currency, currency_or, format_money, now, parse_money_opt, storage, today, IpcResult,
};
use crate::session::{IpcError, Session, SessionError};

fn care_err(e: CareError) -> SessionError {
    match e {
        CareError::Sqlite(e) => storage(e),
        CareError::Invalid(m) => SessionError::Vault(am_storage::vault::VaultError::Other(m)),
    }
}

#[derive(Serialize)]
pub struct CareView {
    #[serde(flatten)]
    pub entry: CareEntry,
    pub cost_display: Option<String>,
}

fn view(entry: CareEntry) -> CareView {
    CareView { cost_display: format_money(entry.cost_minor, entry.currency.as_deref()), entry }
}

#[tauri::command]
pub fn list_care(session: State<'_, Session>, asset_id: String) -> IpcResult<Vec<CareView>> {
    session.touch();
    session
        .with_vault(|vault| {
            Ok(care::history(vault, &asset_id)
                .map_err(care_err)?
                .into_iter()
                .map(view)
                .collect())
        })
        .map_err(IpcError::from)
}

/// An entry as the form sends it: the cost as typed, in major units.
#[derive(Deserialize)]
pub struct CareForm {
    #[serde(flatten)]
    pub care: NewCare,
    #[serde(default)]
    pub cost: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
}

#[tauri::command]
pub fn add_care(session: State<'_, Session>, entry: CareForm) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            let currency = currency_or(&entry.currency, &base_currency(vault))
                .map_err(|e| storage(e.message))?;
            let mut care = entry.care;
            care.cost =
                parse_money_opt(&entry.cost, &currency).map_err(|e| storage(e.message))?;
            care::add(vault, &care, &today(), &timestamp).map_err(care_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_care(session: State<'_, Session>, care_id: String) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| care::delete(vault, &care_id).map_err(care_err))
        .map_err(IpcError::from)
}

#[derive(Serialize)]
pub struct DueView {
    #[serde(flatten)]
    pub entry: CareView,
    pub asset_name: String,
    pub overdue: bool,
}

/// Care due within `within_days` (60 if not given), overdue first.
#[tauri::command]
pub fn care_due(
    session: State<'_, Session>,
    within_days: Option<i64>,
) -> IpcResult<Vec<DueView>> {
    session.touch();
    let within = within_days.unwrap_or(60).clamp(0, 3650);
    session
        .with_vault(|vault| {
            Ok(care::due(vault, &today(), within)
                .map_err(care_err)?
                .into_iter()
                .map(|d| DueView {
                    entry: view(d.entry),
                    asset_name: d.asset_name,
                    overdue: d.overdue,
                })
                .collect())
        })
        .map_err(IpcError::from)
}
