//! Vault lifecycle: create, unlock, lock, back up, restore.
//!
//! This is where the format meets the filesystem. Everything here is about
//! surviving a crash at the wrong moment: a vault that cannot be restored is
//! worse than one that was never created.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use am_crypto::{derive_subkey, kdf::KEY_LEN, KdfParams, Purpose};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::header::{Credential, NewVault, PairingError, UnlockError, VaultHeader};
use crate::migrate::{migrate, SCHEMA_VERSION};
use crate::{key_to_hex, open_encrypted};

pub const HEADER_FILE: &str = "vault.header";
/// A credential change in flight: the next header, written before the
/// database advances and renamed over [`HEADER_FILE`] after. Present only if
/// the process stopped between those steps; unlock settles it.
pub const HEADER_NEXT_FILE: &str = "vault.header.next";
pub const DB_FILE: &str = "catalog.db";
pub const OBJECTS_DIR: &str = "objects";
pub const CACHE_DIR: &str = "cache";
pub const LOCK_FILE: &str = "vault.lock";
pub const MANIFEST_FILE: &str = "manifest.json";

/// Manifest layout written by this build.
///
/// - **1** listed each object's *plaintext* SHA-256. Anyone holding the backup
///   could hash a photo of their own and learn whether the vault contained
///   it, so version 1 is read but never written.
/// - **2** lists only ciphertext digests, which say nothing about content.
pub const MANIFEST_VERSION: u32 = 2;

/// A manifest is a small JSON list. Anything larger is not one of ours, and is
/// refused before it is read into memory.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

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
            VaultHeader::create(passphrase, params, now)
                .map_err(|e| VaultError::Other(e.to_string()))?;

        let conn = open_db(root, &data_key)?;
        migrate(&conn).map_err(|e| VaultError::Other(e.to_string()))?;

        conn.execute(
            "INSERT INTO vault_meta (id, vault_id, key_epoch, schema_version, created_at)
             VALUES (1, ?1, ?2, ?3, ?4)",
            rusqlite::params![
                hex(&header.vault_id),
                epoch_to_sql(header.key_epoch)?,
                SCHEMA_VERSION,
                now
            ],
        )?;

        write_header_atomic(root, &header)?;

        Ok((
            Self { root: root.to_path_buf(), header, conn, data_key, _lock: lock },
            recovery_key,
        ))
    }

    /// Open with either credential.
    ///
    /// Also settles a credential change that was interrupted (see
    /// [`HEADER_NEXT_FILE`]): the data key is the same under the old and the
    /// new header, so whichever one the credential opens reaches the
    /// database, and the database's epoch says which header is current.
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

        let current = read_header(&header_path)?;
        let next_path = root.join(HEADER_NEXT_FILE);
        // Written atomically, so it is whole or absent; one that does not
        // parse is not ours and plays no part.
        let staged = if next_path.exists() { read_header(&next_path).ok() } else { None };

        let (data_key, opened, from_staged) = match unlock_header(&current, credential, secret)
        {
            Ok((key, header)) => (key, header, false),
            Err(original) => {
                match staged.as_ref().map(|s| unlock_header(s, credential, secret)) {
                    Some(Ok((key, header))) => (key, header, true),
                    _ => return Err(original.into()),
                }
            }
        };

        let conn = open_db(root, &data_key)?;
        migrate(&conn).map_err(|e| VaultError::Other(e.to_string()))?;
        let (db_id, db_epoch) = db_pairing(&conn)?;

        let header = if opened.check_pairing(&db_id, db_epoch).is_ok() {
            opened
        } else if from_staged && opened.vault_id == db_id && opened.key_epoch == db_epoch + 1 {
            // Stopped after the new header was staged but before the database
            // advanced. The credential just used belongs to the new header,
            // so finish the change rather than let that credential stop
            // working on the next unlock.
            conn.execute(
                "UPDATE vault_meta SET key_epoch = ?1 WHERE id = 1",
                rusqlite::params![epoch_to_sql(opened.key_epoch)?],
            )?;
            opened
        } else if let Some(paired) = [Some(&current), staged.as_ref()]
            .into_iter()
            .flatten()
            .find(|h| h.check_pairing(&db_id, db_epoch).is_ok())
        {
            // The other credential's header is the current one — for example
            // the old passphrase, typed after the database had already moved
            // to the new header. Same data key, so this is still the owner.
            paired.clone()
        } else {
            // The split-brain check: a header restored beside a different
            // (or older) database would otherwise fail as an apparent bad
            // passphrase.
            return Err(opened.check_pairing(&db_id, db_epoch).unwrap_err().into());
        };

        // Only after pairing has passed: write back a repaired slot or a
        // settled credential change, then drop the staged file either way.
        if header != current {
            write_header_atomic(root, &header)?;
        }
        if next_path.exists() {
            fs::remove_file(&next_path)?;
            sync_dir(root);
        }

        Ok(Self { root: root.to_path_buf(), header, conn, data_key, _lock: lock })
    }

    /// Open with an already-recovered data key, as when reopening after a
    /// backup. Runs the same migration and pairing checks as unlock.
    fn open_with_key(
        root: &Path,
        header: VaultHeader,
        data_key: Zeroizing<[u8; KEY_LEN]>,
        lock: ProcessLock,
    ) -> Result<Self, VaultError> {
        let conn = open_db(root, &data_key)?;
        migrate(&conn).map_err(|e| VaultError::Other(e.to_string()))?;
        let (db_id, db_epoch) = db_pairing(&conn)?;
        header.check_pairing(&db_id, db_epoch)?;
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

    /// Check a credential against this vault without reopening it.
    ///
    /// Used before sensitive changes, so an unlocked but unattended session
    /// cannot have its passphrase changed by whoever walks up to it.
    pub fn verify(&self, credential: Credential, secret: &str) -> bool {
        self.header.unlock(credential, secret).is_ok()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Back up, then reopen with the same key.
    ///
    /// [`Vault::backup_to`] must close the database to get a coherent copy.
    /// For an interactive backup that would lock the owner out mid-session,
    /// so this keeps the data key across the copy and reopens without asking
    /// for the passphrase again. The protocol is unchanged: the database is
    /// checkpointed and *closed* before a single byte is copied — and the
    /// process lock is held throughout, so no other instance can open the
    /// vault and write to it mid-copy.
    ///
    /// Returns the reopened vault (if reopening worked) alongside the backup
    /// result, so a failed copy does not also lose the session.
    pub fn backup_and_reopen(
        self,
        dest: &Path,
        now: &str,
    ) -> (Option<Self>, Result<BackupManifest, VaultError>) {
        let root = self.root.clone();
        let data_key = Zeroizing::new(*self.data_key);

        let (lock, result) = self.backup_keeping_lock(dest, now);

        let reopened = (|| {
            let header = read_header(&root.join(HEADER_FILE))?;
            Self::open_with_key(&root, header, data_key, lock)
        })();
        (reopened.ok(), result)
    }

    pub fn change_passphrase(
        &mut self,
        new_passphrase: &str,
        params: &KdfParams,
    ) -> Result<(), VaultError> {
        let mut next = self.header.clone();
        next.change_passphrase(&self.data_key, new_passphrase, params)
            .map_err(|e| VaultError::Other(e.to_string()))?;
        self.commit_header(next)
    }

    pub fn rotate_recovery_key(&mut self, params: &KdfParams) -> Result<String, VaultError> {
        let mut next = self.header.clone();
        let key = next
            .rotate_recovery_key(&self.data_key, params)
            .map_err(|e| VaultError::Other(e.to_string()))?;
        self.commit_header(next)?;
        Ok(key)
    }

    /// Move header and database to a new key epoch together.
    ///
    /// Two files cannot be replaced atomically, so the change is staged:
    ///
    /// 1. write the new header beside the current one ([`HEADER_NEXT_FILE`]);
    /// 2. advance the database's epoch — the commit point;
    /// 3. rename the staged header into place.
    ///
    /// Stopping after any step leaves a vault that [`Vault::unlock`] opens
    /// with the old credential or the new one and settles to a consistent
    /// pair. Writing the header first and the database second, as this used
    /// to, left a window in which neither credential paired.
    fn commit_header(&mut self, next: VaultHeader) -> Result<(), VaultError> {
        self.stage_header(&next)?;
        self.advance_db_epoch(next.key_epoch)?;
        // The database has moved on, so the in-memory header must too even
        // if publishing fails; the staged file lets the next unlock finish.
        self.header = next;
        publish_staged_header(&self.root)
    }

    fn stage_header(&self, next: &VaultHeader) -> Result<(), VaultError> {
        let json = next.to_json().map_err(|e| VaultError::Other(e.to_string()))?;
        write_atomic(&self.root.join(HEADER_NEXT_FILE), json.as_bytes())
    }

    fn advance_db_epoch(&self, epoch: u64) -> Result<(), VaultError> {
        self.conn.execute(
            "UPDATE vault_meta SET key_epoch = ?1 WHERE id = 1",
            rusqlite::params![epoch_to_sql(epoch)?],
        )?;
        Ok(())
    }

    /// Quiesce and copy. SQLite's online backup API does not work on an
    /// encrypted database, so this is the only correct protocol.
    ///
    /// Takes `self` by value: the database is closed, which is what
    /// guarantees no writer is active and the WAL is folded back in.
    pub fn backup_to(self, dest: &Path, now: &str) -> Result<BackupManifest, VaultError> {
        let (lock, result) = self.backup_keeping_lock(dest, now);
        drop(lock);
        result
    }

    /// The backup proper. Closes the database but hands the process lock
    /// back to the caller, so the vault stays claimed until the caller has
    /// finished with it — reopened it, or let it go.
    fn backup_keeping_lock(
        self,
        dest: &Path,
        now: &str,
    ) -> (ProcessLock, Result<BackupManifest, VaultError>) {
        let Vault { root, header, conn, data_key, _lock: lock } = self;
        drop(data_key); // zeroized; copying ciphertext needs no key
        let result = backup_closed(&root, &header, conn, dest, now);
        (lock, result)
    }
}

/// Recover the data key from one header, repairing a slot orphaned by an
/// older build's credential change (see `VaultHeader::unlock_orphaned_slot`).
/// Returns the header as it should be written back if repaired.
fn unlock_header(
    header: &VaultHeader,
    credential: Credential,
    secret: &str,
) -> Result<(Zeroizing<[u8; KEY_LEN]>, VaultHeader), UnlockError> {
    match header.unlock(credential, secret) {
        Ok(key) => Ok((key, header.clone())),
        // The original error is the one reported if repair finds nothing.
        Err(original) => match header.unlock_orphaned_slot(credential, secret) {
            Ok((key, epoch)) => {
                let mut repaired = header.clone();
                repaired.pin_slot_epoch_at(credential, epoch);
                Ok((key, repaired))
            }
            Err(_) => Err(original),
        },
    }
}

fn db_pairing(conn: &Connection) -> Result<([u8; 16], u64), VaultError> {
    let (db_vault_id, db_epoch): (String, i64) =
        conn.query_row("SELECT vault_id, key_epoch FROM vault_meta WHERE id = 1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let db_epoch = u64::try_from(db_epoch)
        .map_err(|_| VaultError::Other("the database's key epoch is negative".into()))?;
    Ok((parse_hex16(&db_vault_id)?, db_epoch))
}

/// SQLite integers are signed 64-bit. The key epoch is a small counter;
/// stored exactly as earlier builds stored it (rusqlite used to convert
/// u64 the same way, failing above i64::MAX).
fn epoch_to_sql(epoch: u64) -> Result<i64, VaultError> {
    i64::try_from(epoch).map_err(|_| VaultError::Other("key epoch out of range".into()))
}

fn read_header(path: &Path) -> Result<VaultHeader, VaultError> {
    // A header is a few hundred bytes. Refuse a huge file before reading it.
    if fs::metadata(path)?.len() > 1024 * 1024 {
        return Err(UnlockError::Malformed("header file is implausibly large".into()).into());
    }
    Ok(VaultHeader::from_json(&fs::read_to_string(path)?)?)
}

fn publish_staged_header(root: &Path) -> Result<(), VaultError> {
    fs::rename(root.join(HEADER_NEXT_FILE), root.join(HEADER_FILE))?;
    sync_dir(root);
    Ok(())
}

/// The copy itself, with the database already handed over to be closed.
fn backup_closed(
    root: &Path,
    header: &VaultHeader,
    conn: Connection,
    dest: &Path,
    now: &str,
) -> Result<BackupManifest, VaultError> {
    // Refuse destinations that would corrupt or recurse: inside the vault
    // (the copy would copy itself), or over an existing backup.
    if dest.starts_with(root) {
        return Err(VaultError::Other(
            "choose a backup location outside the vault folder".into(),
        ));
    }
    if dest.exists() && fs::read_dir(dest)?.next().is_some() {
        return Err(VaultError::Other(format!(
            "{} is not empty — choose an empty folder for the backup",
            dest.display()
        )));
    }

    // TRUNCATE folds committed transactions out of the WAL. Without it a file
    // copy can silently miss them.
    conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;

    // Only ciphertext facts. The plaintext hash used for deduplication stays
    // inside the encrypted database: in a manifest anyone can read, it would
    // let them confirm whether a photo of their own is in the vault.
    let objects: Vec<(String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT object_id, ciphertext_bytes FROM objects WHERE gc_state = 'live'
             ORDER BY object_id",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    drop(conn); // closed before a single byte is copied

    // Everything is written beside the destination and renamed into place
    // last, so a backup folder with that name is always a complete one.
    let partial = partial_path(dest);
    if partial.exists() {
        fs::remove_dir_all(&partial)?;
    }
    let result = (|| {
        fs::create_dir_all(partial.join(OBJECTS_DIR))?;
        copy_file_synced(&root.join(HEADER_FILE), &partial.join(HEADER_FILE))?;
        copy_file_synced(&root.join(DB_FILE), &partial.join(DB_FILE))?;

        let mut listed = Vec::with_capacity(objects.len());
        for (object_id, bytes) in objects {
            let src = object_path(&root.join(OBJECTS_DIR), &object_id);
            let dst = object_path(&partial.join(OBJECTS_DIR), &object_id);
            let (copied, digest) = copy_file_hashed(&src, &dst)?;
            if copied as i64 != bytes {
                return Err(VaultError::Other(format!(
                    "object {object_id} is {copied} bytes on disk, but the vault expects {bytes} — \
                     the vault itself may be damaged"
                )));
            }
            listed.push(ManifestObject { object_id, ciphertext_sha256: Some(digest), bytes });
        }
        // cache/ is regenerable and deliberately excluded.

        let manifest = BackupManifest {
            format: crate::header::FORMAT_TAG.to_string(),
            manifest_version: MANIFEST_VERSION,
            format_version: header.format_version,
            schema_version: SCHEMA_VERSION,
            vault_id: hex(&header.vault_id),
            key_epoch: header.key_epoch,
            created_at: now.to_string(),
            objects: listed,
        };
        let json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| VaultError::Other(e.to_string()))?;
        write_atomic(&partial.join(MANIFEST_FILE), json.as_bytes())?;
        sync_tree(&partial);

        if dest.exists() {
            fs::remove_dir(dest)?; // checked empty above
        }
        fs::rename(&partial, dest)?;
        if let Some(parent) = dest.parent() {
            sync_dir(parent);
        }
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&partial);
    }
    result
}

fn partial_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".partial");
    dest.with_file_name(name)
}

/// What a successful restore brought back.
#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub manifest: BackupManifest,
    /// Photos and documents restored, from the backup's own database.
    pub objects: usize,
    /// Assets in the backup's catalog (not counting its trash).
    pub assets: i64,
    /// Objects decrypted and authenticated, in a full verification. A
    /// restore checks sizes and digests only, so this is zero there.
    pub decrypted: usize,
}

/// Restore into a staging directory, prove the result opens and is complete,
/// then swap — keeping the previous vault recoverable beside it.
///
/// Verification needs a credential: the staged copy is unlocked exactly as
/// the restored vault will be, its database integrity-checked, and every
/// photo the database refers to copied and checked against its size and the
/// manifest's ciphertext digest. A backup that fails any of that never
/// displaces the vault already in place.
pub fn restore_from(
    backup: &Path,
    dest: &Path,
    credential: Credential,
    secret: &str,
) -> Result<RestoreReport, VaultError> {
    stage_restore(backup, dest, credential, secret)?.commit()
}

/// The first half of [`restore_from`]: copy and verify, but leave the vault
/// at `dest` untouched. A caller with that vault open can keep using it until
/// the backup has proven good, then close it and [`StagedRestore::commit`].
pub fn stage_restore(
    backup: &Path,
    dest: &Path,
    credential: Credential,
    secret: &str,
) -> Result<StagedRestore, VaultError> {
    let manifest = read_manifest(backup)?;
    for name in [HEADER_FILE, DB_FILE] {
        require_regular_file(&backup.join(name)).map_err(|_| {
            VaultError::Other(format!("backup is missing {name} and cannot be restored"))
        })?;
    }

    let staging = dest.with_extension("restore-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    // From here the staging folder is removed again unless committed.
    let mut staged = StagedRestore {
        staging,
        dest: dest.to_path_buf(),
        report: RestoreReport { manifest, objects: 0, assets: 0, decrypted: 0 },
        committed: false,
    };
    let filled = fill_staging(
        backup,
        &staged.staging,
        &staged.report.manifest,
        credential,
        secret,
        false,
    )?;
    staged.report.objects = filled.objects;
    staged.report.assets = filled.assets;
    Ok(staged)
}

/// Prove a backup restores, without restoring it: everything a restore
/// does, into a scratch folder beside `scratch`, plus decrypting every
/// photo and document — then the copy is removed. The vault in use is not
/// touched. This is a restore rehearsal, and the only check that says a
/// backup will actually open on a new machine with the credential given.
pub fn verify_backup(
    backup: &Path,
    scratch: &Path,
    credential: Credential,
    secret: &str,
) -> Result<RestoreReport, VaultError> {
    let manifest = read_manifest(backup)?;
    for name in [HEADER_FILE, DB_FILE] {
        require_regular_file(&backup.join(name)).map_err(|_| {
            VaultError::Other(format!("backup is missing {name} and cannot be restored"))
        })?;
    }
    let staging = scratch.with_extension("verify-staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    let result = fill_staging(backup, &staging, &manifest, credential, secret, true);
    let _ = fs::remove_dir_all(&staging);
    let filled = result?;
    Ok(RestoreReport {
        manifest,
        objects: filled.objects,
        assets: filled.assets,
        decrypted: filled.decrypted,
    })
}

struct Filled {
    objects: usize,
    assets: i64,
    decrypted: usize,
}

/// A verified copy of a backup, waiting beside the vault it will replace.
/// Dropped without [`StagedRestore::commit`], it removes itself.
#[derive(Debug)]
pub struct StagedRestore {
    staging: PathBuf,
    dest: PathBuf,
    report: RestoreReport,
    committed: bool,
}

impl StagedRestore {
    pub fn report(&self) -> &RestoreReport {
        &self.report
    }

    /// Swap the staged copy into place, setting the previous vault aside as
    /// `<dest>.pre-restore`.
    pub fn commit(mut self) -> Result<RestoreReport, VaultError> {
        let dest = self.dest.clone();

        // Do not pull a vault out from under a running instance.
        if dest.join(HEADER_FILE).exists() {
            drop(ProcessLock::acquire(&dest.join(LOCK_FILE))?);
        }

        let previous = dest.with_extension("pre-restore");
        if previous.exists() {
            fs::remove_dir_all(&previous)?;
        }
        if dest.exists() {
            fs::rename(&dest, &previous)?;
        }
        if let Err(e) = fs::rename(&self.staging, &dest) {
            // Put the previous vault back rather than leave nothing in place.
            if previous.exists() {
                let _ = fs::rename(&previous, &dest);
            }
            return Err(e.into());
        }
        self.committed = true;
        if let Some(parent) = dest.parent() {
            sync_dir(parent);
        }
        Ok(self.report.clone())
    }
}

impl Drop for StagedRestore {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.staging);
        }
    }
}

/// Copy a backup into `staging` and verify it there. Returns the number of
/// objects restored.
fn fill_staging(
    backup: &Path,
    staging: &Path,
    manifest: &BackupManifest,
    credential: Credential,
    secret: &str,
    decrypt_all: bool,
) -> Result<Filled, VaultError> {
    fs::create_dir_all(staging.join(OBJECTS_DIR))?;
    fs::create_dir_all(staging.join(CACHE_DIR))?;
    copy_file_synced(&backup.join(HEADER_FILE), &staging.join(HEADER_FILE))?;
    copy_file_synced(&backup.join(DB_FILE), &staging.join(DB_FILE))?;

    // The same unlock the restored vault will get: credential, header
    // authentication, header↔database pairing, migrations.
    let vault = Vault::unlock(staging, credential, secret)?;
    if hex(&vault.header.vault_id) != manifest.vault_id {
        return Err(VaultError::Other(
            "the manifest belongs to a different vault than the files beside it".into(),
        ));
    }
    let check: String = vault.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(VaultError::Other(format!("the backup's database is damaged: {check}")));
    }

    // The encrypted database, not the readable manifest, says which objects
    // must exist: a manifest can be edited, the database cannot without the
    // key. The manifest supplies only the digest to check each copy against.
    let objects: Vec<(String, i64)> = {
        let mut stmt = vault.conn.prepare(
            "SELECT object_id, ciphertext_bytes FROM objects WHERE gc_state = 'live'",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let digests: HashMap<&str, Option<&str>> = manifest
        .objects
        .iter()
        .map(|o| (o.object_id.as_str(), o.ciphertext_sha256.as_deref()))
        .collect();

    for (object_id, bytes) in &objects {
        if !is_object_id(object_id) {
            return Err(VaultError::Other(format!(
                "the backup's database lists a malformed object id {object_id:?}"
            )));
        }
        let src = object_path(&backup.join(OBJECTS_DIR), object_id);
        let meta = require_regular_file(&src)
            .map_err(|_| VaultError::Other(format!("backup is missing object {object_id}")))?;
        if meta.len() as i64 != *bytes {
            return Err(VaultError::Other(format!(
                "object {object_id} is {} bytes, the vault expects {bytes} — the backup copy \
                 is damaged",
                meta.len()
            )));
        }
        let expected = match digests.get(object_id.as_str()) {
            Some(digest) => *digest,
            // Version 1 manifests carry no ciphertext digest; size is all
            // there is to check. From version 2 every object is listed.
            None if manifest.manifest_version >= 2 => {
                return Err(VaultError::Other(format!(
                    "the manifest does not list object {object_id} — it is incomplete or has \
                     been edited"
                )))
            }
            None => None,
        };
        let dst = object_path(&staging.join(OBJECTS_DIR), object_id);
        let (_, digest) = copy_file_hashed(&src, &dst)?;
        if let Some(expected) = expected {
            if !digest.eq_ignore_ascii_case(expected) {
                return Err(VaultError::Other(format!(
                    "object {object_id} does not match the backup's checksum — the copy is \
                     damaged"
                )));
            }
        }
    }
    sync_tree(staging);

    // A size and a digest show the copy is the file that was backed up;
    // only decrypting it shows that file is intact and belongs to this key.
    let mut decrypted = 0;
    if decrypt_all {
        for (object_id, _) in &objects {
            crate::objects::load_object(&vault, staging, object_id).map_err(|e| {
                VaultError::Other(format!("object {object_id} does not decrypt: {e}"))
            })?;
            decrypted += 1;
        }
    }
    let assets: i64 = vault.conn.query_row(
        "SELECT count(*) FROM assets WHERE deleted_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(Filled { objects: objects.len(), assets, decrypted })
}

/// Read and validate a manifest before anything is copied: a backup folder
/// is untrusted input, however it was made.
pub fn read_manifest(backup: &Path) -> Result<BackupManifest, VaultError> {
    let path = backup.join(MANIFEST_FILE);
    let meta = require_regular_file(&path)
        .map_err(|_| VaultError::Other("backup has no manifest.json".into()))?;
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err(VaultError::Other(
            "manifest.json is too large to be a backup manifest".into(),
        ));
    }
    let manifest: BackupManifest = serde_json::from_str(&fs::read_to_string(&path)?)
        .map_err(|e| VaultError::Other(format!("unreadable manifest: {e}")))?;

    if manifest.format != crate::header::FORMAT_TAG {
        return Err(VaultError::Other(format!("unknown backup format: {}", manifest.format)));
    }
    if manifest.manifest_version == 0 || manifest.manifest_version > MANIFEST_VERSION {
        return Err(VaultError::Other(format!(
            "backup manifest version {} is not supported by this build",
            manifest.manifest_version
        )));
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(VaultError::Other(format!(
            "backup uses schema version {}, newer than this build supports ({SCHEMA_VERSION})",
            manifest.schema_version
        )));
    }
    if manifest.vault_id.len() != 32
        || !manifest.vault_id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(VaultError::Other("the manifest's vault id is malformed".into()));
    }
    for obj in &manifest.objects {
        if !is_object_id(&obj.object_id) || obj.bytes < 0 {
            return Err(VaultError::Other(format!(
                "the manifest lists a malformed object {:?}",
                obj.object_id
            )));
        }
        if let Some(digest) = &obj.ciphertext_sha256 {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(VaultError::Other(format!(
                    "the manifest's checksum for {} is malformed",
                    obj.object_id
                )));
            }
        }
    }
    Ok(manifest)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupManifest {
    pub format: String,
    /// Layout of this file; see [`MANIFEST_VERSION`]. Absent in version 1.
    #[serde(default = "manifest_v1")]
    pub manifest_version: u32,
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

fn manifest_v1() -> u32 {
    1
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManifestObject {
    pub object_id: String,
    /// SHA-256 of the encrypted file, for detecting a damaged copy. Never the
    /// plaintext hash. Absent in version 1 manifests, whose plaintext `sha256`
    /// field is ignored on read and never written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ciphertext_sha256: Option<String>,
    pub bytes: i64,
}

/// An object ID as the vault generates them: 32 lowercase hex characters.
pub fn is_object_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Two-level fan-out: filesystems degrade with tens of thousands of entries in
/// one directory.
///
/// Never panics on a short or non-ASCII ID; callers that take IDs from
/// untrusted input check them with [`is_object_id`] first.
pub fn object_path(objects_dir: &Path, object_id: &str) -> PathBuf {
    match (object_id.get(0..2), object_id.get(2..4)) {
        (Some(a), Some(b)) => objects_dir.join(a).join(b).join(object_id),
        _ => objects_dir.join(object_id),
    }
}

fn open_db(root: &Path, data_key: &[u8; KEY_LEN]) -> Result<Connection, VaultError> {
    let db_subkey = derive_subkey(data_key, Purpose::Database);
    let path = root.join(DB_FILE);
    open_encrypted(
        path.to_str().ok_or_else(|| VaultError::Other("non-UTF-8 path".into()))?,
        &key_to_hex(&db_subkey),
    )
    .map_err(|e| VaultError::Other(e.to_string()))
}

fn write_header_atomic(root: &Path, header: &VaultHeader) -> Result<(), VaultError> {
    let json = header.to_json().map_err(|e| VaultError::Other(e.to_string()))?;
    write_atomic(&root.join(HEADER_FILE), json.as_bytes())
}

/// Write, fsync, rename. A crash leaves either the old file or the new one,
/// never a truncated one — which for the header would mean an unopenable vault.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), VaultError> {
    let mut tmp_name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;

    // Also sync the directory, or the rename itself may not survive a crash.
    if let Some(dir) = path.parent() {
        sync_dir(dir);
    }
    Ok(())
}

fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

/// Sync every directory under `root`, so renames and new entries in a fresh
/// copy are durable before it is published.
fn sync_tree(root: &Path) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                sync_tree(&entry.path());
            }
        }
    }
    sync_dir(root);
}

/// A regular file, not a symlink or a directory: a backup folder must not be
/// able to point a restore at some other file on the machine.
fn require_regular_file(path: &Path) -> std::io::Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    Ok(meta)
}

fn copy_file_synced(src: &Path, dst: &Path) -> Result<u64, VaultError> {
    copy_file_hashed(src, dst).map(|(bytes, _)| bytes)
}

/// Copy a regular file, fsync the copy, and return its size and SHA-256.
fn copy_file_hashed(src: &Path, dst: &Path) -> Result<(u64, String), VaultError> {
    require_regular_file(src)?;
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut input = fs::File::open(src)?;
    let mut output = fs::File::create(dst)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        output.write_all(&buf[..n])?;
        total += n as u64;
    }
    output.sync_all()?;
    Ok((total, hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex16(s: &str) -> Result<[u8; 16], VaultError> {
    let malformed = || VaultError::Other("malformed vault id in database".into());
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(malformed());
    }
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| malformed())?;
    }
    Ok(out)
}

/// Advisory single-instance lock.
///
/// "Single machine" does not mean "one process": launching the app twice would
/// otherwise give two writers to one SQLite file.
///
/// An OS file lock rather than a PID file: the operating system releases it
/// when the process exits for any reason, so a crash cannot leave the vault
/// claimed — on every platform, and without guessing whether a recorded PID
/// was reused. The file itself is left in place on release; deleting it would
/// let a process that opened the old file and one that created a new file
/// both hold "the" lock.
struct ProcessLock {
    _file: fs::File,
}

impl ProcessLock {
    fn acquire(path: &Path) -> Result<Self, VaultError> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(VaultError::Locked),
            Err(fs::TryLockError::Error(e)) => return Err(e.into()),
        }
        // For a person wondering which process holds it; not consulted.
        let _ = file.set_len(0);
        let _ = write!(file, "{}", std::process::id());
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    const NOW: &str = "2026-09-19T00:00:00Z";
    const PASS: &str = "correct horse battery staple";
    const NEW_PASS: &str = "a different passphrase";
    /// A well-formed object ID for rows planted directly in the database.
    const FAKE_ID: &str = "ab12cd34ef56ab12cd34ef56ab12cd34";

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
            matches!(
                Vault::unlock(&root, Credential::Passphrase, PASS),
                Err(VaultError::Locked)
            ),
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
        restore_from(&backup, &root, Credential::Passphrase, PASS).unwrap();

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
        restore_from(&backup, &elsewhere, Credential::RecoveryKey, &recovery).unwrap();

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

        restore_from(&backup, &root, Credential::Passphrase, PASS).unwrap();
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

        let err = restore_from(&backup, &dir.path().join("out"), Credential::Passphrase, PASS)
            .unwrap_err();
        assert!(err.to_string().contains("missing vault.header"), "got: {err}");
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
                     VALUES (?1,'hash1',100,'image/jpeg','2026-09-19')",
                    [FAKE_ID],
                )
                .unwrap();

            let path = object_path(&root.join(OBJECTS_DIR), FAKE_ID);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, vec![0u8; 100]).unwrap();

            v.backup_to(&backup, NOW).unwrap();
        }

        // Corrupt the backup: shorten an object.
        let copied = object_path(&backup.join(OBJECTS_DIR), FAKE_ID);
        fs::write(&copied, vec![0u8; 50]).unwrap();

        let out = dir.path().join("out");
        let err = restore_from(&backup, &out, Credential::Passphrase, PASS).unwrap_err();
        assert!(err.to_string().contains("the vault expects 100"), "got: {err}");
        assert!(!out.exists(), "a failed restore leaves nothing in place");
        assert!(!out.with_extension("restore-staging").exists(), "and cleans up its staging");
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

    #[test]
    fn backup_and_reopen_keeps_the_session_and_produces_a_restorable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (v, recovery) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        v.conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                 VALUES ('a1','generic','Kept',?1,?1)",
                [NOW],
            )
            .unwrap();

        let dest = dir.path().join("backup");
        let (reopened, result) = v.backup_and_reopen(&dest, NOW);
        let manifest = result.unwrap();
        let v = reopened.expect("the vault must reopen without asking for the passphrase");
        assert_eq!(manifest.vault_id, hex(&v.vault_id()));

        // Still usable, still the same data.
        let name: String =
            v.conn().query_row("SELECT name FROM assets", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "Kept");
        drop(v);

        // And the copy restores onto a clean machine with the recovery key.
        let elsewhere = dir.path().join("new-machine").join("vault");
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        restore_from(&dest, &elsewhere, Credential::RecoveryKey, &recovery).unwrap();
        let restored = Vault::unlock(&elsewhere, Credential::RecoveryKey, &recovery).unwrap();
        let name: String =
            restored.conn().query_row("SELECT name FROM assets", [], |r| r.get(0)).unwrap();
        assert_eq!(name, "Kept");
    }

    #[test]
    fn a_failed_backup_does_not_lose_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (v, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        // Inside the vault: the copy would copy itself.
        let (reopened, result) = v.backup_and_reopen(&root.join("nested"), NOW);
        assert!(result.is_err());
        let v = reopened.expect("still open after a refused backup");

        // A folder that already has something in it.
        let occupied = dir.path().join("occupied");
        std::fs::create_dir_all(&occupied).unwrap();
        std::fs::write(occupied.join("keep.txt"), "mine").unwrap();
        let (reopened, result) = v.backup_and_reopen(&occupied, NOW);
        assert!(result.unwrap_err().to_string().contains("not empty"));
        assert!(reopened.is_some());
        assert_eq!(std::fs::read_to_string(occupied.join("keep.txt")).unwrap(), "mine");
    }

    #[test]
    fn verify_checks_the_credential_without_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let (v, recovery) =
            Vault::create(&dir.path().join("vault"), PASS, &fast(), NOW).unwrap();
        assert!(v.verify(Credential::Passphrase, PASS));
        assert!(!v.verify(Credential::Passphrase, "not the passphrase"));
        assert!(v.verify(Credential::RecoveryKey, &recovery));
    }

    #[test]
    fn a_recovery_key_orphaned_by_an_older_build_is_repaired_on_unlock() {
        // Reproduce the old bug: change the passphrase, then strip the slot
        // epoch the fixed code records, as a header written by an older
        // build would lack it.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (mut v, recovery) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        v.change_passphrase("a different passphrase", &fast()).unwrap();
        drop(v);

        let path = root.join(HEADER_FILE);
        let mut header = VaultHeader::from_json(&fs::read_to_string(&path).unwrap()).unwrap();
        header.recovery_slot.wrapped_at_epoch = None;
        fs::write(&path, header.to_json().unwrap()).unwrap();
        assert!(
            header.unlock(Credential::RecoveryKey, &recovery).is_err(),
            "the orphaned state the old build left behind"
        );

        let v = Vault::unlock(&root, Credential::RecoveryKey, &recovery).unwrap();
        drop(v);

        let repaired = VaultHeader::from_json(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(repaired.recovery_slot.wrapped_at_epoch, Some(1), "written back");
        assert!(repaired.unlock(Credential::RecoveryKey, &recovery).is_ok());
        assert!(repaired.unlock(Credential::Passphrase, "a different passphrase").is_ok());

        // And a wrong key is still simply wrong.
        assert!(Vault::unlock(
            &root,
            Credential::RecoveryKey,
            "AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-GGGG-HHHH"
        )
        .is_err());
    }

    // ------------------------------------------------------------ backup privacy

    fn jpeg(marker: u8) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
        bytes.extend(std::iter::repeat_n(marker, 4096));
        bytes
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files_under(&path, out);
            } else {
                out.push(path);
            }
        }
    }

    /// A vault holding one real, encrypted photo, backed up to `backup`.
    /// Returns the photo's plaintext and object ID.
    fn backed_up_photo(root: &Path, backup: &Path) -> (Vec<u8>, String) {
        let photo = jpeg(0x5A);
        let (v, _recovery) = Vault::create(root, PASS, &fast(), NOW).unwrap();
        let stored = crate::objects::import_object(&v, root, &photo, NOW).unwrap();
        v.backup_to(backup, NOW).unwrap();
        (photo, stored.object_id)
    }

    #[test]
    fn a_backup_never_reveals_which_photos_the_vault_holds() {
        // The membership check this prevents: hash a photo you already have,
        // look for that hash in someone's backup.
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        let (photo, object_id) = backed_up_photo(&dir.path().join("vault"), &backup);
        let fingerprint = sha256_hex(&photo);
        let raw_fingerprint = Sha256::digest(&photo);

        let mut files = Vec::new();
        files_under(&backup, &mut files);
        assert!(files.len() >= 4, "header, database, manifest and the photo");
        for file in files {
            let bytes = fs::read(&file).unwrap();
            let text = String::from_utf8_lossy(&bytes).to_lowercase();
            assert!(
                !text.contains(&fingerprint),
                "{} holds the plaintext hash",
                file.display()
            );
            assert!(
                !bytes.windows(raw_fingerprint.len()).any(|w| w == raw_fingerprint.as_slice()),
                "{} holds the raw plaintext hash",
                file.display()
            );
        }

        let manifest = read_manifest(&backup).unwrap();
        assert_eq!(manifest.manifest_version, MANIFEST_VERSION);
        let listed = &manifest.objects[0];
        assert_eq!(listed.object_id, object_id);
        let ciphertext = fs::read(object_path(&backup.join(OBJECTS_DIR), &object_id)).unwrap();
        assert_eq!(listed.ciphertext_sha256.as_deref(), Some(sha256_hex(&ciphertext).as_str()));
    }

    #[test]
    fn a_restored_photo_still_decrypts() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        let (photo, object_id) = backed_up_photo(&dir.path().join("vault"), &backup);

        let elsewhere = dir.path().join("elsewhere");
        let report = restore_from(&backup, &elsewhere, Credential::Passphrase, PASS).unwrap();
        assert_eq!(report.objects, 1);
        let v = Vault::unlock(&elsewhere, Credential::Passphrase, PASS).unwrap();
        let loaded = crate::objects::load_object(&v, &elsewhere, &object_id).unwrap();
        assert_eq!(loaded.as_slice(), photo.as_slice());
    }

    #[test]
    fn verification_decrypts_everything_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        let root = dir.path().join("vault");
        backed_up_photo(&root, &backup);

        let report =
            verify_backup(&backup, &dir.path().join("scratch"), Credential::Passphrase, PASS)
                .unwrap();
        assert_eq!((report.objects, report.decrypted), (1, 1));
        assert!(
            !dir.path().join("scratch.verify-staging").exists(),
            "the rehearsal is cleaned up"
        );
        assert!(root.join(HEADER_FILE).exists(), "the vault in use is untouched");
        assert!(verify_backup(
            &backup,
            &dir.path().join("scratch"),
            Credential::Passphrase,
            "wrong"
        )
        .is_err());
    }

    #[test]
    fn only_verification_catches_an_object_altered_along_with_its_checksum() {
        // Someone (or something) rewrote both the file and the manifest: the
        // copy checks pass, but the content is not what was encrypted.
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        let (_photo, object_id) = backed_up_photo(&dir.path().join("vault"), &backup);
        let copy = object_path(&backup.join(OBJECTS_DIR), &object_id);
        let mut bytes = fs::read(&copy).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        fs::write(&copy, &bytes).unwrap();
        let mut manifest = read_manifest(&backup).unwrap();
        manifest.objects[0].ciphertext_sha256 = Some(sha256_hex(&bytes));
        fs::write(backup.join(MANIFEST_FILE), serde_json::to_string(&manifest).unwrap())
            .unwrap();

        let err =
            verify_backup(&backup, &dir.path().join("scratch"), Credential::Passphrase, PASS)
                .unwrap_err();
        assert!(err.to_string().contains("does not decrypt"), "got: {err}");
    }

    #[test]
    fn restore_detects_a_corrupted_object_of_the_right_size() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        let (_photo, object_id) = backed_up_photo(&dir.path().join("vault"), &backup);

        let copy = object_path(&backup.join(OBJECTS_DIR), &object_id);
        let mut bytes = fs::read(&copy).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0x01;
        fs::write(&copy, bytes).unwrap();

        let out = dir.path().join("out");
        let err = restore_from(&backup, &out, Credential::Passphrase, PASS).unwrap_err();
        assert!(err.to_string().contains("checksum"), "got: {err}");
        assert!(!out.exists());
    }

    #[test]
    fn restore_detects_an_edited_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        backed_up_photo(&dir.path().join("vault"), &backup);
        let path = backup.join(MANIFEST_FILE);
        let original = read_manifest(&backup).unwrap();

        // An object dropped from the list: the encrypted database still
        // says it must exist.
        let mut edited = original.clone();
        edited.objects.clear();
        fs::write(&path, serde_json::to_string(&edited).unwrap()).unwrap();
        let err = restore_from(&backup, &dir.path().join("a"), Credential::Passphrase, PASS)
            .unwrap_err();
        assert!(err.to_string().contains("does not list"), "got: {err}");

        // A different vault's ID.
        let mut edited = original.clone();
        edited.vault_id = "00".repeat(16);
        fs::write(&path, serde_json::to_string(&edited).unwrap()).unwrap();
        let err = restore_from(&backup, &dir.path().join("b"), Credential::Passphrase, PASS)
            .unwrap_err();
        assert!(err.to_string().contains("different vault"), "got: {err}");
    }

    #[test]
    fn a_version_1_backup_still_restores() {
        // Made by an earlier build: no manifest_version, plaintext `sha256`.
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        let (photo, object_id) = backed_up_photo(&dir.path().join("vault"), &backup);
        let bytes =
            fs::metadata(object_path(&backup.join(OBJECTS_DIR), &object_id)).unwrap().len();
        let current = read_manifest(&backup).unwrap();
        let v1 = serde_json::json!({
            "format": current.format,
            "format_version": current.format_version,
            "schema_version": current.schema_version,
            "vault_id": current.vault_id,
            "key_epoch": current.key_epoch,
            "created_at": current.created_at,
            "objects": [{ "object_id": object_id, "sha256": sha256_hex(&photo), "bytes": bytes }],
        });
        fs::write(backup.join(MANIFEST_FILE), v1.to_string()).unwrap();

        let out = dir.path().join("out");
        let report = restore_from(&backup, &out, Credential::Passphrase, PASS).unwrap();
        assert_eq!(report.manifest.manifest_version, 1);
        assert_eq!(report.objects, 1);
    }

    #[test]
    fn a_wrong_credential_never_displaces_the_current_vault() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        create(&root);
        Vault::unlock(&root, Credential::Passphrase, PASS)
            .unwrap()
            .backup_to(&backup, NOW)
            .unwrap();
        {
            let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.conn()
                .execute(
                    "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                     VALUES ('a1','generic','Added after the backup',?1,?1)",
                    [NOW],
                )
                .unwrap();
        }

        let err = restore_from(&backup, &root, Credential::Passphrase, "not it").unwrap_err();
        assert!(matches!(err, VaultError::Unlock(_)), "got: {err}");
        assert!(!root.with_extension("pre-restore").exists(), "nothing was swapped");
        assert!(!root.with_extension("restore-staging").exists());
        let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
        let count: i64 =
            v.conn().query_row("SELECT count(*) FROM assets", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "the current vault is untouched");
    }

    #[test]
    fn restore_will_not_replace_a_vault_another_instance_has_open() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        create(&root);
        Vault::unlock(&root, Credential::Passphrase, PASS)
            .unwrap()
            .backup_to(&backup, NOW)
            .unwrap();

        let open = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
        let err = restore_from(&backup, &root, Credential::Passphrase, PASS).unwrap_err();
        assert!(matches!(err, VaultError::Locked), "got: {err}");
        drop(open);
        restore_from(&backup, &root, Credential::Passphrase, PASS).unwrap();
    }

    #[test]
    fn malformed_manifests_are_refused_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        backed_up_photo(&dir.path().join("vault"), &backup);
        let good = read_manifest(&backup).unwrap();
        let path = backup.join(MANIFEST_FILE);

        for bad_id in [
            "é",
            "ab",
            "../../../../etc/passwd",
            "ABCDEF0123456789ABCDEF0123456789",
            "ééééééééééééééééé",
        ] {
            let mut m = good.clone();
            m.objects[0].object_id = bad_id.to_string();
            fs::write(&path, serde_json::to_string(&m).unwrap()).unwrap();
            let err = read_manifest(&backup).unwrap_err();
            assert!(err.to_string().contains("malformed"), "{bad_id:?}: {err}");
        }

        let mut m = good.clone();
        m.objects[0].ciphertext_sha256 = Some("zz".into());
        fs::write(&path, serde_json::to_string(&m).unwrap()).unwrap();
        assert!(read_manifest(&backup).is_err());

        let mut m = good.clone();
        m.manifest_version = 99;
        fs::write(&path, serde_json::to_string(&m).unwrap()).unwrap();
        assert!(read_manifest(&backup).unwrap_err().to_string().contains("not supported"));

        fs::write(&path, "{ not json").unwrap();
        assert!(read_manifest(&backup).unwrap_err().to_string().contains("unreadable"));
    }

    #[cfg(unix)]
    #[test]
    fn restore_refuses_symlinks_in_place_of_backup_files() {
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("backup");
        backed_up_photo(&dir.path().join("vault"), &backup);
        let elsewhere = dir.path().join("planted.db");
        fs::rename(backup.join(DB_FILE), &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, backup.join(DB_FILE)).unwrap();

        let err = restore_from(&backup, &dir.path().join("out"), Credential::Passphrase, PASS)
            .unwrap_err();
        assert!(err.to_string().contains("missing catalog.db"), "got: {err}");
    }

    #[test]
    fn a_failed_backup_leaves_no_folder_that_looks_complete() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let backup = dir.path().join("backup");
        let (v, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        // The database lists a photo whose file has gone missing.
        v.conn()
            .execute(
                "INSERT INTO objects (object_id, plaintext_sha256, ciphertext_bytes, media_type, created_at)
                 VALUES (?1,'h',10,'image/jpeg',?2)",
                [FAKE_ID, NOW],
            )
            .unwrap();

        assert!(v.backup_to(&backup, NOW).is_err());
        assert!(!backup.exists(), "no half-written backup under the real name");
        assert!(!partial_path(&backup).exists(), "and the partial copy is cleaned up");
    }

    #[test]
    fn the_vault_stays_claimed_for_the_whole_backup() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        create(&root);
        let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();

        let (lock, result) = v.backup_keeping_lock(&dir.path().join("backup"), NOW);
        result.unwrap();
        assert!(
            matches!(
                Vault::unlock(&root, Credential::Passphrase, PASS),
                Err(VaultError::Locked)
            ),
            "another instance must not get in between the copy and the reopen"
        );
        drop(lock);
        assert!(Vault::unlock(&root, Credential::Passphrase, PASS).is_ok());
    }

    // ------------------------------------------------------------ process lock

    #[test]
    fn a_lock_file_left_by_a_crash_does_not_lock_the_owner_out() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        create(&root);
        // What a killed process leaves behind: the file, with no lock held.
        fs::write(root.join(LOCK_FILE), "4194303").unwrap();
        assert!(Vault::unlock(&root, Credential::Passphrase, PASS).is_ok());
    }

    // ------------------------------------------------------------ credential changes

    /// Stop a passphrase change after `steps` of its three steps, as a crash
    /// or a full disk would.
    fn interrupt_passphrase_change(root: &Path, steps: usize) {
        let v = Vault::unlock(root, Credential::Passphrase, PASS).unwrap();
        let mut next = v.header.clone();
        next.change_passphrase(&v.data_key, NEW_PASS, &fast()).unwrap();
        if steps >= 1 {
            v.stage_header(&next).unwrap();
        }
        if steps >= 2 {
            v.advance_db_epoch(next.key_epoch).unwrap();
        }
    }

    #[test]
    fn a_change_interrupted_before_the_database_moved_opens_with_either_passphrase() {
        for (typed, keeps_working, stops_working) in
            [(PASS, PASS, NEW_PASS), (NEW_PASS, NEW_PASS, PASS)]
        {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("vault");
            create(&root);
            interrupt_passphrase_change(&root, 1);

            drop(Vault::unlock(&root, Credential::Passphrase, typed).unwrap());
            assert!(!root.join(HEADER_NEXT_FILE).exists(), "settled");
            assert!(Vault::unlock(&root, Credential::Passphrase, keeps_working).is_ok());
            assert!(Vault::unlock(&root, Credential::Passphrase, stops_working).is_err());
        }
    }

    #[test]
    fn a_change_interrupted_after_the_database_moved_opens_with_either_passphrase() {
        for typed in [PASS, NEW_PASS] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("vault");
            let recovery = create(&root);
            interrupt_passphrase_change(&root, 2);

            // The old header no longer pairs with the database. Before the
            // staged protocol this state could not be opened at all.
            let v = Vault::unlock(&root, Credential::Passphrase, typed).unwrap();
            assert_eq!(v.header().key_epoch, 2);
            drop(v);
            assert!(!root.join(HEADER_NEXT_FILE).exists());
            assert!(Vault::unlock(&root, Credential::Passphrase, NEW_PASS).is_ok());
            assert!(Vault::unlock(&root, Credential::RecoveryKey, &recovery).is_ok());
        }
    }

    #[test]
    fn an_unstaged_header_beside_a_mismatched_database_is_still_a_pairing_error() {
        // The staged path must not loosen the split-brain check.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        create(&root);
        {
            let v = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap();
            v.advance_db_epoch(7).unwrap();
        }
        let err = Vault::unlock(&root, Credential::Passphrase, PASS).unwrap_err();
        assert!(matches!(err, VaultError::Pairing(_)), "got: {err}");
    }
}
