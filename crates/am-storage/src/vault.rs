//! Vault lifecycle: create, unlock, lock, back up, restore.
//!
//! This is where the format meets the filesystem. Everything here is about
//! surviving a crash at the wrong moment: a vault that cannot be restored is
//! worse than one that was never created.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use am_crypto::{derive_subkey, kdf::KEY_LEN, KdfParams, Purpose};
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::header::{Credential, NewVault, PairingError, UnlockError, VaultHeader};
use crate::migrate::{migrate, SCHEMA_VERSION};
use crate::{key_to_hex, open_encrypted};

pub const HEADER_FILE: &str = "vault.header";
pub const DB_FILE: &str = "catalog.db";
pub const OBJECTS_DIR: &str = "objects";
pub const CACHE_DIR: &str = "cache";
pub const LOCK_FILE: &str = "vault.lock";

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("no vault found at {0}")]
    NotFound(PathBuf),
    #[error("a vault already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error(transparent)]
    Unlock(#[from] UnlockError),
    #[error(transparent)]
    Pairing(#[from] PairingError),
    #[error("another Asset Manager instance has this vault open")]
    Locked,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Other(String),
}

/// An open, unlocked vault.
///
/// Dropping it closes the database and releases the process lock. The data key
/// is zeroized on drop by `Zeroizing`.
pub struct Vault {
    root: PathBuf,
    header: VaultHeader,
    conn: Connection,
    data_key: Zeroizing<[u8; KEY_LEN]>,
    _lock: ProcessLock,
}

/// Hand-written rather than derived: a derived `Debug` would print the data
/// key, and debug output ends up in logs and panic messages.
impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("root", &self.root)
            .field("vault_id", &hex(&self.header.vault_id))
            .field("key_epoch", &self.header.key_epoch)
            .field("data_key", &"<redacted>")
            .finish()
    }
}

impl Vault {
    pub fn create(
        root: &Path,
        passphrase: &str,
        params: &KdfParams,
        now: &str,
    ) -> Result<(Self, String), VaultError> {
        if root.join(HEADER_FILE).exists() {
            return Err(VaultError::AlreadyExists(root.to_path_buf()));
        }
        fs::create_dir_all(root)?;
        fs::create_dir_all(root.join(OBJECTS_DIR))?;
        fs::create_dir_all(root.join(CACHE_DIR))?;

        let lock = ProcessLock::acquire(&root.join(LOCK_FILE))?;

        let NewVault { header, data_key, recovery_key } =
            VaultHeader::create(passphrase, params, now).map_err(|e| VaultError::Other(e.to_string()))?;

        let conn = open_db(root, &data_key)?;
        migrate(&conn).map_err(|e| VaultError::Other(e.to_string()))?;

        conn.execute(
            "INSERT INTO vault_meta (id, vault_id, key_epoch, schema_version, created_at)
             VALUES (1, ?1, ?2, ?3, ?4)",
            rusqlite::params![hex(&header.vault_id), header.key_epoch, SCHEMA_VERSION, now],
        )?;

        write_header_atomic(root, &header)?;

        Ok((Self { root: root.to_path_buf(), header, conn, data_key, _lock: lock }, recovery_key))
    }

    pub fn unlock(
        root: &Path,
        credential: Credential,
        secret: &str,
    ) -> Result<Self, VaultError> {
        let header_path = root.join(HEADER_FILE);
        if !header_path.exists() {
            return Err(VaultError::NotFound(root.to_path_buf()));
        }
        let lock = ProcessLock::acquire(&root.join(LOCK_FILE))?;

        let header = VaultHeader::from_json(&fs::read_to_string(&header_path)?)?;
        let data_key = header.unlock(credential, secret)?;

        let conn = open_db(root, &data_key)?;
        migrate(&conn).map_err(|e| VaultError::Other(e.to_string()))?;

        // The split-brain check: a header restored beside a different (or
        // older) database would otherwise fail as an apparent bad passphrase.
        let (db_vault_id, db_epoch): (String, u64) = conn.query_row(
            "SELECT vault_id, key_epoch FROM vault_meta WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        header.check_pairing(&parse_hex16(&db_vault_id)?, db_epoch)?;

        Ok(Self { root: root.to_path_buf(), header, conn, data_key, _lock: lock })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn header(&self) -> &VaultHeader {
        &self.header
    }

    /// Subkey for a purpose. Objects and thumbnails never share a key.
    pub fn subkey(&self, purpose: Purpose) -> Zeroizing<[u8; KEY_LEN]> {
        derive_subkey(&self.data_key, purpose)
    }

    pub fn vault_id(&self) -> [u8; 16] {
        self.header.vault_id
    }

    pub fn change_passphrase(
        &mut self,
        new_passphrase: &str,
        params: &KdfParams,
    ) -> Result<(), VaultError> {
        self.header
            .change_passphrase(&self.data_key, new_passphrase, params)
            .map_err(|e| VaultError::Other(e.to_string()))?;

        // Database and header must advance together, or the next unlock fails
        // the pairing check.
        self.conn.execute(
            "UPDATE vault_meta SET key_epoch = ?1 WHERE id = 1",
            rusqlite::params![self.header.key_epoch],
        )?;
        write_header_atomic(&self.root, &self.header)?;
        Ok(())
    }

    pub fn rotate_recovery_key(&mut self, params: &KdfParams) -> Result<String, VaultError> {
        let key = self
            .header
            .rotate_recovery_key(&self.data_key, params)
            .map_err(|e| VaultError::Other(e.to_string()))?;

        self.conn.execute(
            "UPDATE vault_meta SET key_epoch = ?1 WHERE id = 1",
            rusqlite::params![self.header.key_epoch],
        )?;
        write_header_atomic(&self.root, &self.header)?;
        Ok(key)
    }

    /// Quiesce and copy. SQLite's online backup API does not work on an
    /// encrypted database, so this is the only correct protocol.
    ///
    /// Takes `self` by value: the vault is closed, which is what guarantees no
    /// writer is active and the WAL is folded back in.
    pub fn backup_to(self, dest: &Path, now: &str) -> Result<BackupManifest, VaultError> {
        // TRUNCATE folds committed transactions out of the WAL. Without it a
        // file copy can silently miss them.
        self.conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;

        let objects: Vec<(String, String, i64)> = {
            let mut stmt = self.conn.prepare(
                "SELECT object_id, plaintext_sha256, ciphertext_bytes
                 FROM objects WHERE gc_state = 'live'",
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };

        let manifest = BackupManifest {
            format: crate::header::FORMAT_TAG.to_string(),
            format_version: self.header.format_version,
            schema_version: SCHEMA_VERSION,
            vault_id: hex(&self.header.vault_id),
            key_epoch: self.header.key_epoch,
            created_at: now.to_string(),
            objects: objects
                .into_iter()
                .map(|(id, hash, bytes)| ManifestObject { object_id: id, sha256: hash, bytes })
                .collect(),
        };

        let root = self.root.clone();
        drop(self); // close the database and release the lock before copying

        fs::create_dir_all(dest)?;
        fs::copy(root.join(HEADER_FILE), dest.join(HEADER_FILE))?;
        fs::copy(root.join(DB_FILE), dest.join(DB_FILE))?;
        copy_dir(&root.join(OBJECTS_DIR), &dest.join(OBJECTS_DIR))?;

        // cache/ is regenerable and deliberately excluded.

        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| VaultError::Other(e.to_string()))?;
        write_atomic(&dest.join("manifest.json"), manifest_json.as_bytes())?;

        Ok(manifest)
    }
}

/// Restore into a staging directory, verify, then swap — keeping the previous
/// vault recoverable until the swap succeeds.
pub fn restore_from(backup: &Path, dest: &Path) -> Result<BackupManifest, VaultError> {
    let manifest_path = backup.join("manifest.json");
    if !manifest_path.exists() {
        return Err(VaultError::Other("backup has no manifest.json".into()));
    }
    let manifest: BackupManifest = serde_json::from_str(&fs::read_to_string(&manifest_path)?)
        .map_err(|e| VaultError::Other(format!("unreadable manifest: {e}")))?;

    if manifest.format != crate::header::FORMAT_TAG {
        return Err(VaultError::Other(format!("unknown backup format: {}", manifest.format)));
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(VaultError::Other(format!(
            "backup uses schema version {}, newer than this build supports ({SCHEMA_VERSION})",
            manifest.schema_version
        )));
    }
    if !backup.join(HEADER_FILE).exists() {
        return Err(VaultError::Other(
            "backup is missing vault.header and cannot be decrypted".into(),
        ));
    }

    // Verify every object is present and the right size before touching the
    // destination.
    for obj in &manifest.objects {
        let path = object_path(&backup.join(OBJECTS_DIR), &obj.object_id);
        let meta = fs::metadata(&path).map_err(|_| {
            VaultError::Other(format!("backup is missing object {}", obj.object_id))
        })?;
        if meta.len() as i64 != obj.bytes {
            return Err(VaultError::Other(format!(
                "object {} is {} bytes, manifest says {}",
                obj.object_id,
                meta.len(),
                obj.bytes
            )));
        }
    }

    let staging = dest.with_extension("restore-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    fs::copy(backup.join(HEADER_FILE), staging.join(HEADER_FILE))?;
    fs::copy(backup.join(DB_FILE), staging.join(DB_FILE))?;
    copy_dir(&backup.join(OBJECTS_DIR), &staging.join(OBJECTS_DIR))?;
    fs::create_dir_all(staging.join(CACHE_DIR))?;

    // Swap, keeping the old vault until the new one is in place.
    let previous = dest.with_extension("pre-restore");
    if previous.exists() {
        fs::remove_dir_all(&previous)?;
    }
    if dest.exists() {
        fs::rename(dest, &previous)?;
    }
    fs::rename(&staging, dest)?;

    Ok(manifest)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupManifest {
    pub format: String,
    pub format_version: u16,
    pub schema_version: i64,
    pub vault_id: String,
    pub key_epoch: u64,
    pub created_at: String,
    /// Present from v1 even though v1 copies everything: objects are immutable
    /// and content-addressed, so incremental backup is cheap later — but only
    /// if the manifest already lists them.
    pub objects: Vec<ManifestObject>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManifestObject {
    pub object_id: String,
    pub sha256: String,
    pub bytes: i64,
}

/// Two-level fan-out: filesystems degrade with tens of thousands of entries in
/// one directory.
pub fn object_path(objects_dir: &Path, object_id: &str) -> PathBuf {
    objects_dir.join(&object_id[0..2]).join(&object_id[2..4]).join(object_id)
}

fn open_db(root: &Path, data_key: &[u8; KEY_LEN]) -> Result<Connection, VaultError> {
    let db_subkey = derive_subkey(data_key, Purpose::Database);
    let path = root.join(DB_FILE);
    open_encrypted(path.to_str().ok_or_else(|| VaultError::Other("non-UTF-8 path".into()))?,
                   &key_to_hex(&db_subkey))
        .map_err(|e| VaultError::Other(e.to_string()))
}

fn write_header_atomic(root: &Path, header: &VaultHeader) -> Result<(), VaultError> {
    let json = header.to_json().map_err(|e| VaultError::Other(e.to_string()))?;
    write_atomic(&root.join(HEADER_FILE), json.as_bytes())
}

/// Write, fsync, rename. A crash leaves either the old file or the new one,
/// never a truncated one — which for the header would mean an unopenable vault.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;

    // Also sync the directory, or the rename itself may not survive a crash.
    if let Some(dir) = path.parent() {
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), VaultError> {
    fs::create_dir_all(dst)?;
    if !src.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex16(s: &str) -> Result<[u8; 16], VaultError> {
    if s.len() != 32 {
        return Err(VaultError::Other("malformed vault id in database".into()));
    }
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|_| VaultError::Other("malformed vault id in database".into()))?;
    }
    Ok(out)
}

/// Advisory single-instance lock.
///
/// "Single machine" does not mean "one process": launching the app twice would
/// otherwise give two writers to one SQLite file.
struct ProcessLock {
    path: PathBuf,
}

impl ProcessLock {
    fn acquire(path: &Path) -> Result<Self, VaultError> {
        match fs::OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut f) => {
                let _ = write!(f, "{}", std::process::id());
                Ok(Self { path: path.to_path_buf() })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // A stale lock from a crash should not brick the vault, so
                // check whether the recorded process still exists.
                if let Ok(contents) = fs::read_to_string(path) {
                    if let Ok(pid) = contents.trim().parse::<u32>() {
                        if !process_is_running(pid) {
                            fs::remove_file(path)?;
                            return Self::acquire(path);
                        }
                    }
                }
                Err(VaultError::Locked)
            }
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for ProcessLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(target_os = "linux")]
fn process_is_running(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(target_os = "linux"))]
fn process_is_running(_pid: u32) -> bool {
    // Conservative: assume it is alive rather than stealing a live lock.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    const NOW: &str = "2026-09-19T00:00:00Z";
    const PASS: &str = "correct horse battery staple";

    fn create(root: &Path) -> String {
        let (vault, recovery) = Vault::create(root, PASS, &fast(), NOW).unwrap();
        drop(vault);
        recovery
    }

    #[test]
    fn create_then_unlock_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let recovery = create(&root);

        assert!(root.join(HEADER_FILE).exists());
        assert!(root.join(DB_FILE).exists());
        assert!(root.join(OBJECTS_DIR).is_dir());

        let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
        drop(v);

        let v = Vault::unlock(&root, Credential::RecoveryKey, &recovery).unwrap();
        drop(v);
    }

    #[test]
    fn wrong_passphrase_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        create(&root);

        assert!(Vault::unlock(&root, Credential::Passphrase, "nope").is_err());
    }

    #[test]
    fn will_not_overwrite_an_existing_vault() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        create(&root);

        let again = Vault::create(&root, PASS, &fast(), NOW);
        assert!(matches!(again, Err(VaultError::AlreadyExists(_))));
    }

    #[test]
    fn second_instance_is_locked_out() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        create(&root);

        let first = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
        assert!(
            matches!(Vault::unlock(&root, Credential::Passphrase, PASS), Err(VaultError::Locked)),
            "a second instance must not open the same vault"
        );

        drop(first);
        // ...and the lock is released on close.
        assert!(Vault::unlock(&root, Credential::Passphrase, PASS).is_ok());
    }

    #[test]
    fn passphrase_change_advances_epoch_in_both_places() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        create(&root);

        {
            let mut v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.change_passphrase("a different passphrase", &fast()).unwrap();
        }

        // If the database epoch had not been updated alongside the header,
        // this unlock would fail the pairing check.
        let v = Vault::unlock(&root, Credential::Passphrase, "a different passphrase").unwrap();
        assert_eq!(v.header().key_epoch, 2);
        drop(v);

        assert!(Vault::unlock(&root, Credential::Passphrase, PASS).is_err());
    }

    #[test]
    fn recovery_rotation_keeps_the_vault_openable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let old_recovery = create(&root);

        let new_recovery = {
            let mut v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.rotate_recovery_key(&fast()).unwrap()
        };

        assert!(Vault::unlock(&root, Credential::RecoveryKey, &old_recovery).is_err());
        assert!(Vault::unlock(&root, Credential::RecoveryKey, &new_recovery).is_ok());
    }

    #[test]
    fn header_from_a_different_vault_cannot_decrypt_the_database() {
        // Two vaults have different data keys, so a foreign header fails at
        // the SQLCipher layer before the pairing check is ever consulted.
        // Defense in depth rather than a gap: the bytes stay unreadable.
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        create(&a);
        create(&b);

        fs::copy(b.join(HEADER_FILE), a.join(HEADER_FILE)).unwrap();

        let err = Vault::unlock(&a, Credential::Passphrase, PASS).unwrap_err();
        assert!(err.to_string().contains("not a database"), "got: {err}");
    }

    #[test]
    fn stale_header_from_the_same_vault_is_caught_by_the_pairing_check() {
        // The case the pairing check actually exists for, and the one that
        // happens in practice: back up, change the passphrase, then restore
        // only vault.header from the older backup. The data key is unchanged,
        // so the database decrypts fine — and without the epoch check the
        // vault would open in an inconsistent state.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        create(&root);

        let stale_header = dir.path().join("stale.header");
        fs::copy(root.join(HEADER_FILE), &stale_header).unwrap();

        {
            let mut v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.change_passphrase("a different passphrase", &fast()).unwrap();
        }

        fs::copy(&stale_header, root.join(HEADER_FILE)).unwrap();

        let err = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap_err();
        assert!(matches!(err, VaultError::Pairing(_)), "expected a pairing error, got: {err}");
        assert!(
            err.to_string().contains("partial restore"),
            "the error must point at the real cause, not a bad passphrase"
        );
    }

    #[test]
    fn backup_and_restore_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        create(&root);

        {
            let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.conn()
                .execute(
                    "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                     VALUES ('a1','gold_bullion','1 oz Maple','2026-09-19','2026-09-19')",
                    [],
                )
                .unwrap();
            v.backup_to(&backup, NOW).unwrap();
        }

        assert!(backup.join(HEADER_FILE).exists(), "backup must include the header");
        assert!(backup.join("manifest.json").exists());

        // Destroy the original and restore over it.
        fs::remove_dir_all(&root).unwrap();
        restore_from(&backup, &root).unwrap();

        let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
        let name: String = v
            .conn()
            .query_row("SELECT name FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "1 oz Maple");
    }

    #[test]
    fn restore_works_with_recovery_key_on_a_clean_machine() {
        // The scenario that matters: the original machine is gone, and all the
        // user has is the backup and the printed recovery sheet.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        let recovery = create(&root);

        {
            let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.conn()
                .execute(
                    "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                     VALUES ('a1','generic','Irreplaceable','2026-09-19','2026-09-19')",
                    [],
                )
                .unwrap();
            v.backup_to(&backup, NOW).unwrap();
        }
        fs::remove_dir_all(&root).unwrap();

        let elsewhere = dir.path().join("new-machine");
        restore_from(&backup, &elsewhere).unwrap();

        let v = Vault::unlock(&elsewhere, Credential::RecoveryKey, &recovery).unwrap();
        let name: String = v
            .conn()
            .query_row("SELECT name FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Irreplaceable");
    }

    #[test]
    fn restore_keeps_the_previous_vault_recoverable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        create(&root);
        {
            let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.backup_to(&backup, NOW).unwrap();
        }

        restore_from(&backup, &root).unwrap();
        assert!(
            root.with_extension("pre-restore").exists(),
            "the pre-restore copy must survive so a bad restore is undoable"
        );
    }

    #[test]
    fn restore_refuses_a_backup_without_a_header() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        create(&root);
        {
            let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.backup_to(&backup, NOW).unwrap();
        }

        fs::remove_file(backup.join(HEADER_FILE)).unwrap();

        let err = restore_from(&backup, &dir.path().join("out")).unwrap_err();
        assert!(err.to_string().contains("missing vault.header"));
    }

    #[test]
    fn restore_detects_a_truncated_object() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        create(&root);

        {
            let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.conn()
                .execute(
                    "INSERT INTO objects (object_id, plaintext_sha256, ciphertext_bytes, media_type, created_at)
                     VALUES ('ab12cd34ef','hash1',100,'image/jpeg','2026-09-19')",
                    [],
                )
                .unwrap();

            let obj_dir = root.join(OBJECTS_DIR).join("ab").join("12");
            fs::create_dir_all(&obj_dir).unwrap();
            fs::write(obj_dir.join("ab12cd34ef"), vec![0u8; 100]).unwrap();

            v.backup_to(&backup, NOW).unwrap();
        }

        // Corrupt the backup: shorten an object.
        let copied = backup.join(OBJECTS_DIR).join("ab").join("12").join("ab12cd34ef");
        fs::write(&copied, vec![0u8; 50]).unwrap();

        let err = restore_from(&backup, &dir.path().join("out")).unwrap_err();
        assert!(err.to_string().contains("manifest says"), "got: {err}");
    }

    #[test]
    fn manifest_lists_live_objects() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        create(&root);

        let manifest = {
            let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.conn()
                .execute_batch(
                    "INSERT INTO objects (object_id, plaintext_sha256, ciphertext_bytes, media_type, gc_state, created_at)
                     VALUES ('aa11bb22','h1',10,'image/jpeg','live','2026-09-19'),
                            ('cc33dd44','h2',20,'image/jpeg','pending_delete','2026-09-19')",
                )
                .unwrap();
            let obj_dir = root.join(OBJECTS_DIR).join("aa").join("11");
            fs::create_dir_all(&obj_dir).unwrap();
            fs::write(obj_dir.join("aa11bb22"), vec![0u8; 10]).unwrap();

            v.backup_to(&backup, NOW).unwrap()
        };

        assert_eq!(manifest.objects.len(), 1, "only live objects belong in the manifest");
        assert_eq!(manifest.objects[0].object_id, "aa11bb22");
    }

    #[test]
    fn object_paths_fan_out() {
        let p = object_path(Path::new("/vault/objects"), "3f7a9c02deadbeef");
        assert_eq!(p, Path::new("/vault/objects/3f/7a/3f7a9c02deadbeef"));
    }
}
