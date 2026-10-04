//! Asset Manager desktop shell.

pub mod asset_commands;
pub mod backup_commands;
pub mod balance_provider;
pub mod care_commands;
pub mod collectible_commands;
pub mod commands;
pub mod crypto_commands;
pub mod crypto_provider;
pub mod custody_commands;
pub mod ipc;
pub mod metals_commands;
pub mod metals_provider;
pub mod organize_commands;
pub mod paths;
pub mod protocol;
pub mod report_commands;
pub mod session;
pub mod spreadsheet_commands;
pub mod types_commands;
pub mod valuation_commands;

use std::time::Duration;

use tauri::{Emitter, Manager};

use session::Session;

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

    configure(tauri::Builder::default())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            spawn_auto_lock(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Asset Manager");
}

/// Everything the app is except its window chrome: session state, the
/// `asset://` protocol and the command table.
///
/// Split out of [`run`] so the integration tests drive exactly the handlers
/// the app ships, over real IPC serialization, on Tauri's mock runtime.
pub fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .register_uri_scheme_protocol("asset", protocol::handle)
        .manage(Session::new())
        .invoke_handler(tauri::generate_handler![
            // Vault, credentials, backup, settings
            commands::vault_status,
            commands::create_vault,
            commands::unlock_vault,
            commands::lock_vault,
            commands::session_state,
            commands::keep_alive,
            commands::change_passphrase,
            commands::rotate_recovery_key,
            backup_commands::backup_vault,
            backup_commands::backup_centre,
            backup_commands::update_backup_preferences,
            backup_commands::verify_backup,
            commands::restore_vault,
            commands::confirm_recovery_saved,
            commands::get_settings,
            commands::update_settings,
            commands::vault_info,
            // Assets and photos
            asset_commands::asset_types,
            asset_commands::list_assets,
            asset_commands::search_assets,
            asset_commands::get_asset,
            asset_commands::validate_asset,
            asset_commands::create_asset,
            asset_commands::update_asset,
            asset_commands::delete_asset,
            asset_commands::restore_asset,
            asset_commands::list_trash,
            asset_commands::purge_trash,
            asset_commands::void_valuation,
            asset_commands::asset_revisions,
            asset_commands::restore_revision,
            // Care and service
            care_commands::list_care,
            care_commands::add_care,
            care_commands::delete_care,
            care_commands::care_due,
            // Custody
            custody_commands::list_custody,
            custody_commands::record_custody,
            custody_commands::delete_custody,
            custody_commands::away_list,
            // Item types
            types_commands::list_custom_types,
            types_commands::save_custom_type,
            types_commands::delete_custom_type,
            // Organizing
            organize_commands::list_tags,
            organize_commands::set_asset_tags,
            organize_commands::rename_tag,
            organize_commands::list_locations,
            organize_commands::rename_location,
            organize_commands::bulk_edit,
            organize_commands::bulk_trash,
            organize_commands::duplicate_asset,
            organize_commands::list_saved_views,
            organize_commands::save_view,
            organize_commands::delete_saved_view,
            spreadsheet_commands::inspect_spreadsheet,
            spreadsheet_commands::import_spreadsheet,
            asset_commands::set_pricing,
            asset_commands::import_photo,
            asset_commands::list_photos,
            asset_commands::remove_photo,
            asset_commands::set_primary_photo,
            asset_commands::export_csv,
            asset_commands::import_csv,
            asset_commands::read_text_file,
            asset_commands::write_text_file,
            asset_commands::export_attachment,
            asset_commands::describe_attachment,
            // Valuation
            valuation_commands::set_prices,
            valuation_commands::portfolio_total,
            valuation_commands::dashboard,
            valuation_commands::change_quantity,
            valuation_commands::portfolio_series,
            // Metals
            metals_commands::bullion_presets,
            metals_commands::spot_prices,
            metals_commands::set_spot_price,
            metals_commands::metals_quota,
            metals_commands::value_metal_holding,
            metals_commands::metals_provider_status,
            metals_commands::set_metals_api_key,
            metals_commands::refresh_spot_prices,
            // Crypto
            crypto_commands::common_coins,
            crypto_commands::crypto_provider_status,
            crypto_commands::set_crypto_api_key,
            crypto_commands::value_crypto_holding,
            crypto_commands::refresh_crypto_prices,
            crypto_commands::set_coin_price,
            crypto_commands::crypto_prices,
            crypto_commands::balance_lookup_status,
            crypto_commands::set_balance_lookup,
            crypto_commands::lookup_balance,
            // Collectibles and reports
            collectible_commands::collectible_types,
            collectible_commands::graders,
            collectible_commands::validate_collectible,
            collectible_commands::scan_slab_label,
            report_commands::insurance_report,
            report_commands::export_claim_files,
        ])
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
fn spawn_auto_lock<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    std::thread::spawn(move || {
        // Poll at a fraction of the limit so the worst-case overshoot is small
        // relative to the timeout itself.
        //
        // The limit is re-read every tick, since the owner can change it.
        loop {
            let session = app.state::<Session>();
            let limit = session.idle_limit();
            let interval = (limit / 30).clamp(Duration::from_secs(1), Duration::from_secs(10));
            std::thread::sleep(interval);
            if session.lock_if_idle(limit) {
                let _ = app.emit("vault-auto-locked", ());
            }
        }
    });
}
