//! The "file manager test" from the plan, at integration level.
//!
//! Requirement under test: *an attacker holding a byte-for-byte copy of the
//! vault directory, without the passphrase or recovery key, learns nothing
//! beyond object count and approximate sizes.*
//!
//! These are regression checks for specified invariants. They do not prove
//! that no plaintext can ever escape — no scan can.
//!
//! Note what is deliberately NOT asserted: the absence of JPEG magic bytes.
//! Uniformly random ciphertext contains `FF D8 FF` roughly 64 times per GiB by
//! chance, so that check would eventually fail on correctly encrypted vaults.
//! High-entropy sentinels have no such false-positive problem.

use std::fs;
use std::path::Path;

use am_crypto::KdfParams;
use am_storage::header::Credential;
use am_storage::vault::{Vault, HEADER_FILE};

/// Long, high-entropy, and unlikely to arise by chance.
const NAME_SENTINEL: &str = "SENTINEL7QK4XW2M9PLZVB3NTYR8JHCF6DGS";
const LOCATION_SENTINEL: &str = "SENTINEL5WD8NXQ2VKMJ4RPT7BZHYC3LFGA";
const NOTES_SENTINEL: &str = "SENTINEL2MJ9XPTB6VKQ4NRZ8WYHDLC5FGS";

const PASS: &str = "correct horse battery staple";
const NOW: &str = "2026-09-19T00:00:00Z";

fn fast() -> KdfParams {
    KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
}

/// Every regular file under `dir`, recursively.
fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn catalog_contents_never_appear_in_plaintext_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");

    {
        let (vault, _recovery) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        vault
            .conn()
            .execute(
                "INSERT INTO assets
                   (asset_id, type_id, name, notes, storage_location, created_at, updated_at)
                 VALUES ('a1', 'gold_bullion', ?1, ?2, ?3, ?4, ?4)",
                rusqlite::params![NAME_SENTINEL, NOTES_SENTINEL, LOCATION_SENTINEL, NOW],
            )
            .unwrap();

        // Force a search so FTS5 writes its index, and a checkpoint so the WAL
        // is folded in. Both are places plaintext could leak.
        let hits: i64 = vault
            .conn()
            .query_row(
                "SELECT count(*) FROM assets_fts WHERE assets_fts MATCH ?1",
                [NAME_SENTINEL],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "the fixture must actually be searchable");

        vault.conn().pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
    }

    let mut files = Vec::new();
    walk(&root, &mut files);
    assert!(!files.is_empty(), "the vault directory must contain files");

    for path in &files {
        let bytes = fs::read(path).unwrap();
        for (label, sentinel) in [
            ("asset name", NAME_SENTINEL),
            ("notes", NOTES_SENTINEL),
            ("storage location", LOCATION_SENTINEL),
        ] {
            assert!(
                !contains(&bytes, sentinel.as_bytes()),
                "{label} found in plaintext in {}",
                path.display()
            );
        }
    }
}

#[test]
fn the_header_is_readable_but_reveals_no_secrets() {
    // The header must be parseable without a credential — that is its whole
    // purpose — while leaking neither the passphrase nor the recovery key.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");

    let recovery = {
        let (vault, recovery) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        drop(vault);
        recovery
    };

    let header = fs::read_to_string(root.join(HEADER_FILE)).unwrap();

    assert!(header.contains("asset-manager-vault-v1"), "format tag should be readable");
    assert!(!header.contains(PASS), "passphrase leaked into the header");
    assert!(!header.contains(&recovery), "recovery key leaked into the header");

    let normalized = am_crypto::normalize_recovery_key(&recovery);
    assert!(!header.contains(&normalized), "normalized recovery key leaked into the header");
}

#[test]
fn a_copied_vault_is_useless_without_a_credential() {
    // The stated threat model: someone walks off with the whole directory.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let stolen = dir.path().join("stolen");

    {
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        vault
            .conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                 VALUES ('a1','generic',?1,?2,?2)",
                rusqlite::params![NAME_SENTINEL, NOW],
            )
            .unwrap();
        vault.conn().pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
    }

    // Byte-for-byte copy, as an attacker would take it.
    fs::create_dir_all(&stolen).unwrap();
    let mut files = Vec::new();
    walk(&root, &mut files);
    for path in &files {
        let rel = path.strip_prefix(&root).unwrap();
        let target = stolen.join(rel);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(path, &target).unwrap();
    }

    for wrong in ["", "password", "correct horse battery stapl", PASS.to_uppercase().as_str()] {
        assert!(
            Vault::unlock(&stolen, Credential::Passphrase, wrong).is_err(),
            "unlocked a stolen vault with {wrong:?}"
        );
    }
    assert!(
        Vault::unlock(&stolen, Credential::RecoveryKey, "AAAA-BBBB-CCCC-DDDD-EEEE").is_err()
    );

    // The correct passphrase still opens it, so the test is not passing
    // because the copy is simply broken.
    let opened = Vault::unlock(&stolen, Credential::Passphrase, PASS).unwrap();
    let name: String = opened
        .conn()
        .query_row("SELECT name FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, NAME_SENTINEL);
}

#[test]
fn no_vault_file_decodes_as_an_image() {
    // Checks the property directly rather than sniffing magic bytes: nothing
    // in the vault should be a readable image.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    {
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        drop(vault);
    }

    let mut files = Vec::new();
    walk(&root, &mut files);

    for path in &files {
        let bytes = fs::read(path).unwrap();
        // A real image starts with one of these. The header is JSON and the
        // database is SQLCipher ciphertext; neither should.
        for magic in [b"\xFF\xD8\xFF".as_slice(), b"\x89PNG".as_slice(), b"GIF8".as_slice()] {
            assert!(
                !bytes.starts_with(magic),
                "{} begins with image magic bytes",
                path.display()
            );
        }
    }
}

/// A distinctive pixel pattern acts as the sentinel for image content.
fn sentinel_png() -> Vec<u8> {
    let img = image::RgbImage::from_fn(240, 180, |x, y| {
        // A deterministic, high-contrast pattern: if any of this survives to
        // disk in plaintext, the raw bytes would be recognizable.
        image::Rgb([((x * 7) % 256) as u8, ((y * 11) % 256) as u8, 0xC3])
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

#[test]
fn imported_photos_and_their_thumbnails_are_never_plaintext_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");

    let photo = sentinel_png();

    {
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
        vault
            .conn()
            .execute(
                "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
                 VALUES ('a1','generic','Coin','2026-09-19','2026-09-19')",
                [],
            )
            .unwrap();

        let stored = am_storage::objects::import_object(&vault, &root, &photo, NOW).unwrap();
        am_storage::objects::attach_to_asset(&vault, "a1", &stored.object_id, NOW).unwrap();
        am_storage::thumbs::generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();

        vault.conn().pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
    }

    let mut files = Vec::new();
    walk(&root, &mut files);

    // The PNG header and a long run from the middle of the encoded file: both
    // would appear if anything wrote the image out unencrypted.
    let png_header = &photo[..16];
    let png_middle = &photo[photo.len() / 2..photo.len() / 2 + 64];

    for path in &files {
        let bytes = fs::read(path).unwrap();
        assert!(
            !contains(&bytes, png_header),
            "plaintext image header found in {}",
            path.display()
        );
        assert!(
            !contains(&bytes, png_middle),
            "plaintext image body found in {}",
            path.display()
        );
        assert!(!bytes.starts_with(b"\x89PNG"), "{} is a readable PNG", path.display());
        assert!(
            !bytes.starts_with(b"\xFF\xD8\xFF"),
            "{} is a readable JPEG (thumbnails are JPEG-encoded before encryption)",
            path.display()
        );
    }

    // Sanity: the test is meaningful only if files were actually written.
    let object_files = files
        .iter()
        .filter(|p| {
            p.to_string_lossy().contains("objects") || p.to_string_lossy().contains("thumbs")
        })
        .count();
    assert!(object_files >= 3, "expected an original plus two variants, found {object_files}");
}

#[test]
fn a_deleted_photo_leaves_nothing_decryptable_behind() {
    // Deleting a photo must remove its thumbnails too — otherwise a deleted
    // photo is still a readable photo.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("vault");
    let photo = sentinel_png();

    let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();
    vault
        .conn()
        .execute(
            "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
             VALUES ('a1','generic','Coin','2026-09-19','2026-09-19')",
            [],
        )
        .unwrap();

    let stored = am_storage::objects::import_object(&vault, &root, &photo, NOW).unwrap();
    am_storage::objects::attach_to_asset(&vault, "a1", &stored.object_id, NOW).unwrap();
    am_storage::thumbs::generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();

    let mut before = Vec::new();
    walk(&root.join("objects"), &mut before);
    walk(&root.join("cache"), &mut before);
    assert!(before.len() >= 3);

    am_storage::objects::detach_from_asset(&vault, "a1", &stored.object_id).unwrap();
    am_storage::objects::sweep_deleted(&vault, &root).unwrap();

    let mut after = Vec::new();
    walk(&root.join("objects"), &mut after);
    walk(&root.join("cache"), &mut after);
    assert!(
        after.is_empty(),
        "files survived deletion: {:?}",
        after.iter().map(|p| p.display().to_string()).collect::<Vec<_>>()
    );
}
