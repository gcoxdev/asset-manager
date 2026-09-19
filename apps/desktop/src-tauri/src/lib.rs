//! Asset Manager desktop shell.

pub mod commands;
pub mod session;

use session::Session;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Session::new())
        .invoke_handler(tauri::generate_handler![
            commands::vault_status,
            commands::create_vault,
            commands::unlock_vault,
            commands::lock_vault,
            commands::list_assets,
            commands::create_asset,
            commands::search_assets,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Asset Manager");
}
