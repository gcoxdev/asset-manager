//! The wishlist: things wanted, never counted as owned.

use am_storage::wishlist::{self, Wish, WishError, WishInput};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::ipc::{
    base_currency, currency_or, format_money, now, parse_money_opt, storage, IpcResult,
};
use crate::session::{IpcError, Session, SessionError};

fn wish_err(e: WishError) -> SessionError {
    match e {
        WishError::Sqlite(e) => storage(e),
        WishError::Invalid(m) => SessionError::Vault(am_storage::vault::VaultError::Other(m)),
    }
}

#[derive(Serialize)]
pub struct WishView {
    #[serde(flatten)]
    pub wish: Wish,
    pub target_display: Option<String>,
}

#[tauri::command]
pub fn list_wishes(session: State<'_, Session>) -> IpcResult<Vec<WishView>> {
    session.touch();
    session
        .with_vault(|vault| {
            Ok(wishlist::list(vault)
                .map_err(wish_err)?
                .into_iter()
                .map(|wish| WishView {
                    target_display: format_money(wish.target_minor, wish.currency.as_deref()),
                    wish,
                })
                .collect())
        })
        .map_err(IpcError::from)
}

#[derive(Deserialize)]
pub struct WishForm {
    #[serde(flatten)]
    pub wish: WishInput,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
}

#[tauri::command]
pub fn save_wish(session: State<'_, Session>, form: WishForm) -> IpcResult<String> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            let currency = currency_or(&form.currency, &base_currency(vault))
                .map_err(|e| storage(e.message))?;
            let mut wish = form.wish;
            wish.target =
                parse_money_opt(&form.target, &currency).map_err(|e| storage(e.message))?;
            wishlist::save(vault, &wish, &timestamp).map_err(wish_err)
        })
        .map_err(IpcError::from)
}

#[tauri::command]
pub fn delete_wish(session: State<'_, Session>, wish_id: String) -> IpcResult<()> {
    session.touch();
    session
        .with_vault(|vault| wishlist::delete(vault, &wish_id).map_err(wish_err))
        .map_err(IpcError::from)
}

/// The wish was bought, and catalogued as `asset_id`.
#[tauri::command]
pub fn wish_acquired(
    session: State<'_, Session>,
    wish_id: String,
    asset_id: String,
) -> IpcResult<()> {
    session.touch();
    let timestamp = now();
    session
        .with_vault(|vault| {
            wishlist::mark_acquired(vault, &wish_id, &asset_id, &timestamp).map_err(wish_err)
        })
        .map_err(IpcError::from)
}
