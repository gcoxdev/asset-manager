//! Portable mode: the vault and preferences beside the app, not in the
//! user profile.
//!
//! Only the AppImage and the standalone Windows executable can be portable.
//! It is switched on by a marker file, `assetmanager-portable`, beside the
//! launcher — or explicitly with `--portable` or `ASSET_MANAGER_PORTABLE=1`.
//! Data then lives in a private `AssetManagerData` folder beside it, so the
//! launcher, its marker and that folder can be carried together (a USB stick
//! is fine: the vault inside is encrypted).
//!
//! The location is still decided here, never by the UI — the reason the
//! vault path is resolved in the backend at all (see paths.rs).
//!
//! When something is wrong — an `AssetManagerData` folder with no marker,
//! a folder that cannot be written — the app opens to an error saying so,
//! rather than quietly using the standard location and showing a different
//! (or an empty) catalog.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub const DATA_DIRECTORY: &str = "AssetManagerData";
pub const MARKER_FILE: &str = "assetmanager-portable";
pub const ENVIRONMENT_VARIABLE: &str = "ASSET_MANAGER_PORTABLE";
pub const ARGUMENT: &str = "--portable";

/// Where this run keeps its data.
#[derive(Debug, Clone, PartialEq)]
pub enum Storage {
    /// The platform's per-user application directory.
    Standard,
    /// `AssetManagerData` beside the launcher.
    Portable(PathBuf),
    /// Portable mode was asked for, or its data found, but cannot be used.
    /// Nothing may fall back to the standard location.
    Unavailable(String),
}

/// `ASSET_MANAGER_PORTABLE`, read strictly: a typo must not silently mean
/// "off".
pub fn parse_environment(value: Option<&OsStr>) -> Result<bool, String> {
    let Some(value) = value else { return Ok(false) };
    match value.to_string_lossy().trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "" | "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("{ENVIRONMENT_VARIABLE} must be 1/true/yes/on or 0/false/no/off")),
    }
}

/// The decision, given the launcher (if known) and whether portable mode was
/// asked for explicitly. `None` means the standard location.
pub fn decide(launcher: Option<&Path>, explicitly: bool) -> Result<Option<PathBuf>, String> {
    let beside = |name: &str| launcher.and_then(Path::parent).map(|dir| dir.join(name));
    let marker = beside(MARKER_FILE).is_some_and(|m| m.is_file());
    if !explicitly && !marker {
        if beside(DATA_DIRECTORY).is_some_and(|d| d.is_dir()) {
            return Err(format!(
                "{DATA_DIRECTORY} is beside this copy of Asset Manager, but portable mode is off — \
                 the {MARKER_FILE} marker file is missing. Put the marker back to open that catalog, \
                 or move {DATA_DIRECTORY} away to use the standard location."
            ));
        }
        return Ok(None);
    }
    let launcher = launcher.ok_or_else(|| {
        "Portable mode works only with the AppImage on Linux and the standalone executable on \
         Windows."
            .to_string()
    })?;
    root_beside(launcher).map(Some)
}

/// `AssetManagerData` beside the launcher: made if missing, private, and
/// proven writable.
fn root_beside(launcher: &Path) -> Result<PathBuf, String> {
    let launcher = fs::canonicalize(launcher).map_err(|e| {
        format!("Could not find this copy of Asset Manager ({}): {e}", launcher.display())
    })?;
    if !launcher.is_file() {
        return Err(format!("The portable launcher is not a file: {}", launcher.display()));
    }
    #[cfg(windows)]
    reject_program_files(&launcher)?;
    let parent =
        launcher.parent().ok_or_else(|| "The portable launcher has no folder".to_string())?;
    let root = parent.join(DATA_DIRECTORY);
    match fs::symlink_metadata(&root) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!("{} must be a folder, not a link", root.display()));
        }
        Ok(meta) if !meta.is_dir() => {
            return Err(format!("{} exists but is not a folder", root.display()));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Could not check {}: {e}", root.display())),
    }
    make_private_directory(&root).map_err(|e| {
        format!(
            "Portable mode cannot use {} ({e}). Move Asset Manager to a folder you can write to, \
             or remove the {MARKER_FILE} marker.",
            root.display()
        )
    })?;
    let probe = root.join(format!(".write-check-{}", uuid::Uuid::new_v4()));
    fs::write(&probe, b"Asset Manager portable storage check")
        .and_then(|()| fs::remove_file(&probe))
        .map_err(|e| format!("{} cannot be written to: {e}", root.display()))?;
    Ok(root)
}

fn make_private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// An installed copy's folder may be shared, read-only, or replaced by the
/// next upgrade — no place for a vault.
#[cfg(windows)]
fn reject_program_files(launcher: &Path) -> Result<(), String> {
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(dir) = std::env::var_os(variable).map(PathBuf::from) {
            if launcher.starts_with(&dir) {
                return Err(format!(
                    "Portable mode does not work for a copy installed under {}. Use the standalone \
                     Windows build in a folder of your own.",
                    dir.display()
                ));
            }
        }
    }
    Ok(())
}

/// Where this run keeps its data, from how it was launched.
pub fn detect() -> Storage {
    let explicitly = std::env::args_os().any(|a| a == ARGUMENT);
    let explicitly = match parse_environment(std::env::var_os(ENVIRONMENT_VARIABLE).as_deref())
    {
        Ok(env) => explicitly || env,
        Err(message) => return Storage::Unavailable(message),
    };

    // The AppImage runtime names the AppImage file; the binary itself runs
    // from a temporary mount, so its own path says nothing about where the
    // user keeps it.
    #[cfg(target_os = "linux")]
    let launcher = std::env::var_os("APPIMAGE").map(PathBuf::from);
    #[cfg(windows)]
    let launcher = std::env::current_exe().ok();
    #[cfg(not(any(target_os = "linux", windows)))]
    let launcher: Option<PathBuf> = None;

    match decide(launcher.as_deref(), explicitly) {
        Ok(Some(root)) => Storage::Portable(root),
        Ok(None) => Storage::Standard,
        Err(message) => Storage::Unavailable(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launcher_in(dir: &Path) -> PathBuf {
        let launcher = dir.join("AssetManager.AppImage");
        fs::write(&launcher, b"launcher").unwrap();
        launcher
    }

    #[test]
    fn without_a_marker_the_standard_location_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = launcher_in(dir.path());
        assert_eq!(decide(Some(&launcher), false), Ok(None));
        assert_eq!(decide(None, false), Ok(None));
        assert!(!dir.path().join(DATA_DIRECTORY).exists(), "nothing is created");
    }

    #[test]
    fn a_marker_puts_private_writable_data_beside_the_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = launcher_in(dir.path());
        fs::write(dir.path().join(MARKER_FILE), b"").unwrap();
        let root = decide(Some(&launcher), false).unwrap().unwrap();
        assert_eq!(root, fs::canonicalize(dir.path()).unwrap().join(DATA_DIRECTORY));
        assert!(root.is_dir());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0, "the write check cleans up");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn asking_explicitly_works_without_a_marker() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = launcher_in(dir.path());
        assert!(decide(Some(&launcher), true).unwrap().is_some());
    }

    #[test]
    fn data_without_its_marker_stops_rather_than_opening_another_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = launcher_in(dir.path());
        fs::create_dir(dir.path().join(DATA_DIRECTORY)).unwrap();
        let err = decide(Some(&launcher), false).unwrap_err();
        assert!(err.contains(MARKER_FILE), "{err}");
    }

    #[test]
    fn portable_needs_a_launcher_to_sit_beside() {
        assert!(decide(None, true).unwrap_err().contains("AppImage"));
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_data_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let launcher = launcher_in(dir.path());
        fs::write(dir.path().join(MARKER_FILE), b"").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), dir.path().join(DATA_DIRECTORY)).unwrap();
        assert!(decide(Some(&launcher), false).unwrap_err().contains("not a link"));
    }

    #[test]
    fn the_environment_switch_is_read_strictly() {
        assert_eq!(parse_environment(None), Ok(false));
        for on in ["1", "true", "YES", " on "] {
            assert_eq!(parse_environment(Some(OsStr::new(on))), Ok(true), "{on}");
        }
        for off in ["", "0", "false", "No", "off"] {
            assert_eq!(parse_environment(Some(OsStr::new(off))), Ok(false), "{off}");
        }
        assert!(parse_environment(Some(OsStr::new("portable"))).is_err());
    }
}
