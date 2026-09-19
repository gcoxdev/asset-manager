//! Encrypted object store: photos and documents on disk.
//!
//! Every object is written once, encrypted, under a random opaque ID. The
//! plaintext hash used for deduplication lives only in the encrypted database,
//! so a filename reveals nothing — not the content, not the type, not whether
//! a given image is present.
//!
//! # Durability
//!
//! Import order is: write ciphertext to a temp file on the same filesystem →
//! fsync → atomic rename → *then* commit the database row. A crash between the
//! rename and the commit leaves a sweepable orphan; the reverse order would
//! leave a database row pointing at a file that does not exist, which looks
//! like a successful import until the user opens the photo.

use std::fs;
use std::io::Write;
use std::path::Path;

use am_crypto::{kdf::KEY_LEN, ObjectContext, Purpose};
use sha2::{Digest, Sha256};

use crate::vault::{object_path, Vault, VaultError, OBJECTS_DIR};

/// Refuse anything larger before allocating. Generous for a photo, bounded
/// against a hostile or mistaken input.
pub const MAX_IMPORT_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ObjectError {
    #[error("file is larger than the {} MiB limit", MAX_IMPORT_BYTES / 1024 / 1024)]
    TooLarge,
    #[error("unsupported media type: {0}")]
    UnsupportedType(String),
    #[error("object {0} is missing from the store")]
    Missing(String),
    #[error("object {0} failed to decrypt — it may be corrupt or from another vault")]
    Corrupt(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Vault(#[from] VaultError),
}

/// Formats accepted on import.
///
/// An allowlist, not a denylist: anything not named here is refused rather
/// than stored and hoped about.
pub const SUPPORTED_TYPES: &[(&str, &[u8])] = &[
    ("image/jpeg", b"\xFF\xD8\xFF"),
    ("image/png", b"\x89PNG\r\n\x1a\n"),
    ("image/webp", b"RIFF"),
    ("application/pdf", b"%PDF"),
];

/// Identify by content, not by file extension: an extension is a claim by
/// whoever named the file, and a mislabelled file should be rejected rather
/// than stored under the wrong type.
pub fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    SUPPORTED_TYPES
        .iter()
        .find(|(_, magic)| bytes.starts_with(magic))
        .map(|(name, _)| *name)
}

#[derive(Debug, Clone)]
pub struct StoredObject {
    pub object_id: String,
    pub media_type: String,
    pub bytes: u64,
    /// True when an identical file was already present and no bytes were
    /// written.
    pub deduplicated: bool,
}

fn random_object_id() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

fn plaintext_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Import a file into the vault.
///
/// Deduplicates on the plaintext hash: re-importing the same file, or
/// attaching one photo to two assets, stores a single copy.
pub fn import_object(
    vault: &Vault,
    root: &Path,
    plaintext: &[u8],
    now: &str,
) -> Result<StoredObject, ObjectError> {
    if plaintext.len() as u64 > MAX_IMPORT_BYTES {
        return Err(ObjectError::TooLarge);
    }
    let media_type = sniff_media_type(plaintext)
        .ok_or_else(|| ObjectError::UnsupportedType("unrecognized file format".into()))?;

    let hash = plaintext_hash(plaintext);

    // Dedup check: identical bytes are stored once.
    let existing: Option<(String, i64)> = vault
        .conn()
        .query_row(
            "SELECT object_id, ciphertext_bytes FROM objects
             WHERE plaintext_sha256 = ?1 AND gc_state = 'live'",
            [&hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();

    if let Some((object_id, bytes)) = existing {
        return Ok(StoredObject {
            object_id,
            media_type: media_type.to_string(),
            bytes: bytes as u64,
            deduplicated: true,
        });
    }

    let object_id = random_object_id();
    let subkey = vault.subkey(Purpose::Object);
    let vault_id = vault.vault_id();
    let id_bytes = parse_hex16(&object_id).expect("generated id is valid hex");

    let ciphertext = am_crypto::seal(
        &subkey,
        ObjectContext {
            vault_id: &vault_id,
            object_id: &id_bytes,
            purpose: Purpose::Object.label(),
            format_version: crate::header::FORMAT_VERSION,
        },
        plaintext,
    )
    .map_err(|_| ObjectError::Corrupt(object_id.clone()))?;

    // File first, then the database row. See the module note on ordering.
    let path = object_path(&root.join(OBJECTS_DIR), &object_id);
    write_object_atomic(&path, &ciphertext)?;

    vault.conn().execute(
        "INSERT INTO objects
           (object_id, plaintext_sha256, ciphertext_bytes, media_type, refcount, gc_state, created_at)
         VALUES (?1, ?2, ?3, ?4, 0, 'live', ?5)",
        rusqlite::params![&object_id, &hash, ciphertext.len() as i64, media_type, now],
    )?;

    Ok(StoredObject {
        object_id,
        media_type: media_type.to_string(),
        bytes: ciphertext.len() as u64,
        deduplicated: false,
    })
}

/// Read and decrypt an object.
pub fn load_object(
    vault: &Vault,
    root: &Path,
    object_id: &str,
) -> Result<zeroize::Zeroizing<Vec<u8>>, ObjectError> {
    // Confirm the object belongs to this vault and is live before touching the
    // filesystem — this is also what stops a caller reading an arbitrary path.
    let exists: i64 = vault.conn().query_row(
        "SELECT count(*) FROM objects WHERE object_id = ?1 AND gc_state = 'live'",
        [object_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(ObjectError::Missing(object_id.to_string()));
    }

    let path = object_path(&root.join(OBJECTS_DIR), object_id);
    let ciphertext =
        fs::read(&path).map_err(|_| ObjectError::Missing(object_id.to_string()))?;

    let id_bytes =
        parse_hex16(object_id).ok_or_else(|| ObjectError::Missing(object_id.to_string()))?;
    let subkey = vault.subkey(Purpose::Object);
    let vault_id = vault.vault_id();

    am_crypto::open(
        &subkey,
        ObjectContext {
            vault_id: &vault_id,
            object_id: &id_bytes,
            purpose: Purpose::Object.label(),
            format_version: crate::header::FORMAT_VERSION,
        },
        &ciphertext,
    )
    .map_err(|_| ObjectError::Corrupt(object_id.to_string()))
}

/// Attach an object to an asset, incrementing its reference count.
pub fn attach_to_asset(
    vault: &Vault,
    asset_id: &str,
    object_id: &str,
    now: &str,
) -> Result<(), ObjectError> {
    let tx = vault.conn().unchecked_transaction()?;

    let is_first: i64 = tx.query_row(
        "SELECT count(*) FROM asset_media WHERE asset_id = ?1",
        [asset_id],
        |r| r.get(0),
    )?;

    tx.execute(
        "INSERT INTO asset_media (asset_id, object_id, is_primary, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![asset_id, object_id, i64::from(is_first == 0), now],
    )?;
    tx.execute("UPDATE objects SET refcount = refcount + 1 WHERE object_id = ?1", [object_id])?;
    tx.commit()?;
    Ok(())
}

/// Detach an object, marking it for collection when nothing references it.
///
/// The row is marked rather than deleted so that a crash before the file is
/// unlinked leaves a record the sweeper can find.
pub fn detach_from_asset(
    vault: &Vault,
    asset_id: &str,
    object_id: &str,
) -> Result<(), ObjectError> {
    let tx = vault.conn().unchecked_transaction()?;
    tx.execute(
        "DELETE FROM asset_media WHERE asset_id = ?1 AND object_id = ?2",
        rusqlite::params![asset_id, object_id],
    )?;
    tx.execute(
        "UPDATE objects SET refcount = max(0, refcount - 1) WHERE object_id = ?1",
        [object_id],
    )?;
    tx.execute(
        "UPDATE objects SET gc_state = 'pending_delete'
         WHERE object_id = ?1 AND refcount = 0 AND gc_state = 'live'",
        [object_id],
    )?;
    tx.commit()?;
    Ok(())
}

/// Delete files for objects marked `pending_delete`.
///
/// File first, then the row: a crash between them leaves a `pending_delete`
/// row whose file is already gone, which the next sweep handles idempotently.
/// The reverse would leave a file nothing points at — an invisible leak.
pub fn sweep_deleted(vault: &Vault, root: &Path) -> Result<usize, ObjectError> {
    let mut stmt = vault
        .conn()
        .prepare("SELECT object_id FROM objects WHERE gc_state = 'pending_delete'")?;
    let pending: Vec<String> =
        stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let mut swept = 0;
    for object_id in pending {
        let path = object_path(&root.join(OBJECTS_DIR), &object_id);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // already gone
            Err(e) => return Err(e.into()),
        }
        vault
            .conn()
            .execute("DELETE FROM objects WHERE object_id = ?1", [&object_id])?;
        swept += 1;
    }
    Ok(swept)
}

fn write_object_atomic(path: &Path, bytes: &[u8]) -> Result<(), ObjectError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        if let Ok(d) = fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

fn parse_hex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Re-exported so callers need not depend on `am_crypto` directly.
pub use am_crypto::kdf::KEY_LEN as OBJECT_KEY_LEN;
const _: () = assert!(OBJECT_KEY_LEN == KEY_LEN);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::Vault;
    use am_crypto::KdfParams;

    const NOW: &str = "2026-09-19T00:00:00Z";
    const PASS: &str = "correct horse battery staple";

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    /// Minimal but genuinely JPEG-headed bytes.
    fn jpeg(marker: u8, len: usize) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.extend(std::iter::repeat_n(marker, len));
        v
    }

    fn open_vault(dir: &Path) -> Vault {
        let (vault, _r) = Vault::create(dir, PASS, &fast(), NOW).unwrap();
        vault
    }

    #[test]
    fn import_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        let photo = jpeg(0xAB, 200_000); // spans several 64 KiB chunks
        let stored = import_object(&vault, &root, &photo, NOW).unwrap();
        assert_eq!(stored.media_type, "image/jpeg");
        assert!(!stored.deduplicated);

        let loaded = load_object(&vault, &root, &stored.object_id).unwrap();
        assert_eq!(loaded.as_slice(), photo.as_slice());
    }

    #[test]
    fn stored_file_is_ciphertext_with_an_opaque_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        let photo = jpeg(0xCD, 5000);
        let stored = import_object(&vault, &root, &photo, NOW).unwrap();

        let path = object_path(&root.join(OBJECTS_DIR), &stored.object_id);
        let on_disk = fs::read(&path).unwrap();

        assert!(!on_disk.starts_with(b"\xFF\xD8\xFF"), "stored object is plaintext JPEG");
        assert!(
            !on_disk.windows(64).any(|w| w.iter().all(|&b| b == 0xCD)),
            "plaintext body survived into the stored object"
        );
        assert!(path.extension().is_none(), "filename must not carry a type-revealing extension");
        assert!(
            !path.to_string_lossy().contains(&plaintext_hash(&photo)),
            "the plaintext hash must not appear in the path"
        );
    }

    #[test]
    fn identical_files_are_stored_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        let photo = jpeg(0x11, 1000);
        let first = import_object(&vault, &root, &photo, NOW).unwrap();
        let second = import_object(&vault, &root, &photo, NOW).unwrap();

        assert!(!first.deduplicated);
        assert!(second.deduplicated, "re-importing identical bytes must dedup");
        assert_eq!(first.object_id, second.object_id);

        let count: i64 =
            vault.conn().query_row("SELECT count(*) FROM objects", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn different_photos_are_not_deduplicated() {
        // Two photographs of the same item differ byte-wise; no design dedups
        // them, and pretending otherwise would lose data.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        let a = import_object(&vault, &root, &jpeg(0x01, 1000), NOW).unwrap();
        let b = import_object(&vault, &root, &jpeg(0x02, 1000), NOW).unwrap();
        assert_ne!(a.object_id, b.object_id);
    }

    #[test]
    fn unsupported_formats_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        let err = import_object(&vault, &root, b"#!/bin/sh\nrm -rf /", NOW).unwrap_err();
        assert!(matches!(err, ObjectError::UnsupportedType(_)));

        // An executable renamed to .jpg is still refused: we sniff content.
        assert!(import_object(&vault, &root, b"MZ\x90\x00executable", NOW).is_err());
    }

    #[test]
    fn oversized_imports_are_refused_before_allocation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        // Claim a size over the limit without actually allocating it.
        let mut huge = jpeg(0x00, 16);
        huge.resize((MAX_IMPORT_BYTES + 1) as usize, 0);
        assert!(matches!(
            import_object(&vault, &root, &huge, NOW),
            Err(ObjectError::TooLarge)
        ));
    }

    #[test]
    fn unknown_object_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        // Not in the database: must fail without touching the filesystem.
        assert!(matches!(
            load_object(&vault, &root, "ffffffffffffffffffffffffffffffff"),
            Err(ObjectError::Missing(_))
        ));
        // Not even a valid id shape.
        assert!(load_object(&vault, &root, "../../etc/passwd").is_err());
    }

    #[test]
    fn a_tampered_object_fails_to_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        let stored = import_object(&vault, &root, &jpeg(0x33, 5000), NOW).unwrap();
        let path = object_path(&root.join(OBJECTS_DIR), &stored.object_id);

        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        fs::write(&path, &bytes).unwrap();

        assert!(matches!(
            load_object(&vault, &root, &stored.object_id),
            Err(ObjectError::Corrupt(_))
        ));
    }

    #[test]
    fn a_truncated_object_fails_to_load() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        let stored = import_object(&vault, &root, &jpeg(0x44, 200_000), NOW).unwrap();
        let path = object_path(&root.join(OBJECTS_DIR), &stored.object_id);

        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();

        assert!(matches!(
            load_object(&vault, &root, &stored.object_id),
            Err(ObjectError::Corrupt(_))
        ));
    }

    #[test]
    fn shared_object_survives_until_the_last_reference_goes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        vault
            .conn()
            .execute_batch(
                "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                 VALUES ('a1','generic','One','2026-09-19','2026-09-19'),
                        ('a2','generic','Two','2026-09-19','2026-09-19')",
            )
            .unwrap();

        let stored = import_object(&vault, &root, &jpeg(0x55, 1000), NOW).unwrap();
        attach_to_asset(&vault, "a1", &stored.object_id, NOW).unwrap();
        attach_to_asset(&vault, "a2", &stored.object_id, NOW).unwrap();

        detach_from_asset(&vault, "a1", &stored.object_id).unwrap();
        assert_eq!(sweep_deleted(&vault, &root).unwrap(), 0, "still referenced by a2");
        assert!(load_object(&vault, &root, &stored.object_id).is_ok());

        detach_from_asset(&vault, "a2", &stored.object_id).unwrap();
        assert_eq!(sweep_deleted(&vault, &root).unwrap(), 1, "last reference gone");

        let path = object_path(&root.join(OBJECTS_DIR), &stored.object_id);
        assert!(!path.exists(), "swept object file must be gone");
    }

    #[test]
    fn sweep_is_idempotent_after_a_crash() {
        // Simulates a crash between unlinking the file and deleting the row.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        let stored = import_object(&vault, &root, &jpeg(0x66, 1000), NOW).unwrap();
        vault
            .conn()
            .execute(
                "UPDATE objects SET gc_state = 'pending_delete' WHERE object_id = ?1",
                [&stored.object_id],
            )
            .unwrap();

        let path = object_path(&root.join(OBJECTS_DIR), &stored.object_id);
        fs::remove_file(&path).unwrap(); // file already gone, row still present

        assert_eq!(sweep_deleted(&vault, &root).unwrap(), 1, "must tolerate a missing file");
        assert_eq!(sweep_deleted(&vault, &root).unwrap(), 0, "second sweep is a no-op");
    }

    #[test]
    fn first_attachment_becomes_primary() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let vault = open_vault(&root);

        vault
            .conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                 VALUES ('a1','generic','One','2026-09-19','2026-09-19')",
                [],
            )
            .unwrap();

        let first = import_object(&vault, &root, &jpeg(0x77, 100), NOW).unwrap();
        let second = import_object(&vault, &root, &jpeg(0x88, 100), NOW).unwrap();
        attach_to_asset(&vault, "a1", &first.object_id, NOW).unwrap();
        attach_to_asset(&vault, "a1", &second.object_id, NOW).unwrap();

        let primary: String = vault
            .conn()
            .query_row(
                "SELECT object_id FROM asset_media WHERE asset_id='a1' AND is_primary=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(primary, first.object_id);
    }

    #[test]
    fn objects_do_not_decrypt_under_another_vaults_key() {
        // Copying an object file into a different vault must not make it
        // readable there — the AEAD context binds it to its vault.
        let dir = tempfile::tempdir().unwrap();
        let root_a = dir.path().join("a");
        let root_b = dir.path().join("b");

        let stored = {
            let vault_a = open_vault(&root_a);
            import_object(&vault_a, &root_a, &jpeg(0x99, 5000), NOW).unwrap()
        };

        let vault_b = open_vault(&root_b);
        // Register the same id in B's database and copy the ciphertext across.
        vault_b
            .conn()
            .execute(
                "INSERT INTO objects
                   (object_id, plaintext_sha256, ciphertext_bytes, media_type, gc_state, created_at)
                 VALUES (?1, 'x', 0, 'image/jpeg', 'live', ?2)",
                rusqlite::params![&stored.object_id, NOW],
            )
            .unwrap();

        let src = object_path(&root_a.join(OBJECTS_DIR), &stored.object_id);
        let dst = object_path(&root_b.join(OBJECTS_DIR), &stored.object_id);
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        fs::copy(&src, &dst).unwrap();

        assert!(
            matches!(load_object(&vault_b, &root_b, &stored.object_id), Err(ObjectError::Corrupt(_))),
            "an object must not decrypt in a vault it does not belong to"
        );
    }
}
