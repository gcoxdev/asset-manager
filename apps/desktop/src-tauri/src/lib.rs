//! Asset Manager desktop shell.

pub mod commands;
pub mod paths;
pub mod protocol;
pub mod session;

use std::time::Duration;

use tauri::{Emitter, Manager};

use session::{Session, AUTO_LOCK_IDLE};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .register_uri_scheme_protocol("asset", protocol::handle)
        .manage(Session::new())
        .setup(|app| {
            spawn_auto_lock(app.handle().clone(), AUTO_LOCK_IDLE);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::vault_status,
            commands::create_vault,
            commands::unlock_vault,
            commands::lock_vault,
            commands::list_assets,
            commands::create_asset,
            commands::search_assets,
            commands::session_state,
            commands::import_photo,
            commands::list_photos,
            commands::remove_photo,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Asset Manager");
}

/// Lock the vault after an idle period.
///
/// A dedicated timer rather than a check on the next command: a vault that
/// only locks when someone tries to use it has not locked at all, because the
/// key stays in memory for as long as the app sits idle.
///
/// The frontend is told so it can leave the catalog screen; the lock itself
/// has already happened in the backend, so a frontend that ignores the event
/// still cannot read anything.
fn spawn_auto_lock<R: tauri::Runtime>(app: tauri::AppHandle<R>, idle_limit: Duration) {
    std::thread::spawn(move || {
        // Poll at a fraction of the limit so the worst-case overshoot is small
        // relative to the timeout itself.
        let interval = (idle_limit / 30).max(Duration::from_secs(1));
        loop {
            std::thread::sleep(interval);
            let session = app.state::<Session>();
            if session.lock_if_idle(idle_limit) {
                let _ = app.emit("vault-auto-locked", ());
            }
        }
    });
}
