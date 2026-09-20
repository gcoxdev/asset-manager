//! Asset Manager desktop shell.

pub mod balance_provider;
pub mod collectible_commands;
pub mod commands;
pub mod crypto_commands;
pub mod crypto_provider;
pub mod metals_commands;
pub mod metals_provider;
pub mod paths;
pub mod protocol;
pub mod report_commands;
pub mod session;
pub mod valuation_commands;

use std::time::Duration;

use tauri::{Emitter, Manager};

use session::{Session, AUTO_LOCK_IDLE};

/// Work around a WebKitGTK/Mesa explicit-sync bug on Wayland.
///
/// WebKitGTK's DMA-BUF renderer negotiates `wp_linux_drm_syncobj` (explicit
/// GPU sync) and then commits a buffer without setting an acquire point.
/// Strict compositors — KWin among them — correctly reject that as a protocol
/// violation, and the app dies before its window appears:
///
/// ```text
/// wl_display#1.error(wp_linux_drm_syncobj_surface_v1#52, 4,
///                    "explicit sync is used, but no acquire point is set")
/// Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.
/// ```
///
/// Disabling the DMA-BUF renderer avoids that path. The cost is a slower
/// composite path for the WebView; the benefit is that the app starts at all.
///
/// Set from inside the process so a user running the binary directly gets a
/// working app without needing a launcher script.
///
/// An already-set value is left alone. Note that WebKit treats the variable's
/// *presence* as the switch rather than parsing its contents — setting it to
/// `0` or to an empty string does not re-enable the renderer. To test whether
/// upstream has fixed this, the variable has to be removed from the
/// environment entirely and this function short-circuited; there is no
/// value that turns it back on.
#[cfg(target_os = "linux")]
fn apply_wayland_workarounds() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: called before any window or thread that reads the
        // environment exists, so there is no concurrent getenv to race.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_wayland_workarounds() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    apply_wayland_workarounds();

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
            commands::export_csv,
            commands::import_csv,
            commands::read_text_file,
            commands::write_text_file,
            valuation_commands::set_prices,
            valuation_commands::portfolio_total,
            valuation_commands::valuation_history,
            valuation_commands::change_quantity,
            valuation_commands::quantity_on,
            valuation_commands::portfolio_series,
            metals_commands::bullion_presets,
            metals_commands::spot_prices,
            metals_commands::set_spot_price,
            metals_commands::metals_quota,
            metals_commands::value_metal_holding,
            metals_commands::metals_provider_status,
            metals_commands::set_metals_api_key,
            metals_commands::refresh_spot_prices,
            crypto_commands::common_coins,
            crypto_commands::crypto_provider_status,
            crypto_commands::set_crypto_api_key,
            crypto_commands::value_crypto_holding,
            crypto_commands::refresh_crypto_prices,
            collectible_commands::collectible_types,
            collectible_commands::graders,
            collectible_commands::validate_collectible,
            collectible_commands::create_collectible,
            report_commands::insurance_report,
            collectible_commands::scan_slab_label,
            crypto_commands::balance_lookup_status,
            crypto_commands::set_balance_lookup,
            crypto_commands::lookup_balance,
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
