//! Encrypted thumbnail generation.
//!
//! A plaintext thumbnail cache would defeat the entire scheme — a 256px thumb
//! of a slabbed coin is still a photo of your coin — so variants are encrypted
//! exactly like originals, under their own [`Purpose::Thumbnail`] subkey and
//! their own opaque object IDs.
//!
//! Variants get **separate random IDs** rather than `<original-id>.<size>`: a
//! size suffix would leak which variants exist and reintroduce an
//! extension-like marker on disk.

use std::path::Path;

use am_crypto::{ObjectContext, Purpose};
use image::GenericImageView;

use crate::objects::{load_object, ObjectError};
use crate::vault::{object_path, Vault, CACHE_DIR};

/// Sizes generated for the grid and the detail view.
pub const VARIANTS: &[(&str, u32)] = &[("thumb:256", 256), ("thumb:1024", 1024)];

/// Refuse to decode an image whose pixel count would blow up memory.
///
/// Streaming ciphertext bounds *file* size, not decoded size: a few hundred KB
/// of PNG can declare dimensions that decode to gigabytes. 50 megapixels is
/// far beyond any phone camera while stopping a decompression bomb.
pub const MAX_DECODED_PIXELS: u64 = 50_000_000;

#[derive(Debug, thiserror::Error)]
pub enum ThumbError {
    #[error("image is too large to decode safely ({width}x{height})")]
    TooManyPixels { width: u32, height: u32 },
    #[error("could not decode image")]
    Decode,
    #[error(transparent)]
    Object(#[from] ObjectError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

fn random_object_id() -> String {
    let mut raw = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
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

/// Decode, checking declared dimensions *before* allocating the pixel buffer.
fn decode_bounded(bytes: &[u8]) -> Result<image::DynamicImage, ThumbError> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ThumbError::Decode)?;

    // Dimensions come from the header, so this rejects a decompression bomb
    // without ever materializing its pixels.
    if let Ok((width, height)) = reader.into_dimensions() {
        if u64::from(width) * u64::from(height) > MAX_DECODED_PIXELS {
            return Err(ThumbError::TooManyPixels { width, height });
        }
    }

    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ThumbError::Decode)?
        .decode()
        .map_err(|_| ThumbError::Decode)
}

/// Generate and store the encrypted variants for an object.
///
/// Idempotent: variants that already exist are left alone, so this is safe to
/// call again after an interrupted run.
pub fn generate_variants(
    vault: &Vault,
    root: &Path,
    object_id: &str,
    now: &str,
) -> Result<Vec<String>, ThumbError> {
    let plaintext = load_object(vault, root, object_id)?;

    // PDFs and anything else non-decodable simply get no thumbnails.
    let Ok(source) = decode_bounded(&plaintext) else {
        return Ok(Vec::new());
    };
    let (width, height) = source.dimensions();

    let mut created = Vec::new();
    for (variant, max_edge) in VARIANTS {
        let existing: i64 = vault.conn().query_row(
            "SELECT count(*) FROM media_variants WHERE object_id = ?1 AND variant = ?2",
            rusqlite::params![object_id, variant],
            |r| r.get(0),
        )?;
        if existing > 0 {
            continue;
        }

        // Never upscale: a 128px original should not become a blurry 256px file.
        let resized = if width.max(height) <= *max_edge {
            source.clone()
        } else {
            source.thumbnail(*max_edge, *max_edge)
        };

        let mut encoded = Vec::new();
        resized
            .write_to(&mut std::io::Cursor::new(&mut encoded), image::ImageFormat::Jpeg)
            .map_err(|_| ThumbError::Decode)?;

        let variant_id = random_object_id();
        let id_bytes = parse_hex16(&variant_id).expect("generated id is valid hex");
        let subkey = vault.subkey(Purpose::Thumbnail);
        let vault_id = vault.vault_id();

        // Thumbnails authenticate under the Thumbnail purpose, so a thumbnail
        // cannot be served in place of an original, or vice versa.
        let ciphertext = am_crypto::seal(
            &subkey,
            ObjectContext {
                vault_id: &vault_id,
                object_id: &id_bytes,
                purpose: Purpose::Thumbnail.label(),
                format_version: crate::header::FORMAT_VERSION,
            },
            &encoded,
        )
        .map_err(|_| ThumbError::Decode)?;

        let path = variant_path(root, &variant_id)?;
        crate::objects::write_bytes_atomic(&path, &ciphertext)?;

        vault.conn().execute(
            "INSERT INTO media_variants (object_id, variant, variant_object_id, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![object_id, variant, &variant_id, now],
        )?;
        created.push((*variant).to_string());
    }

    // Record the original's real dimensions while we have them decoded.
    vault.conn().execute(
        "UPDATE objects SET width = ?1, height = ?2 WHERE object_id = ?3",
        rusqlite::params![width, height, object_id],
    )?;

    Ok(created)
}

/// Load a decrypted variant, falling back to `None` if it has not been
/// generated.
pub fn load_variant(
    vault: &Vault,
    root: &Path,
    object_id: &str,
    variant: &str,
) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, ThumbError> {
    let variant_id: Option<String> = vault
        .conn()
        .query_row(
            "SELECT variant_object_id FROM media_variants WHERE object_id = ?1 AND variant = ?2",
            rusqlite::params![object_id, variant],
            |r| r.get(0),
        )
        .ok();

    let Some(variant_id) = variant_id else { return Ok(None) };

    let path = variant_path(root, &variant_id)?;
    let Ok(ciphertext) = std::fs::read(&path) else { return Ok(None) };

    let id_bytes = parse_hex16(&variant_id).ok_or(ThumbError::Decode)?;
    let subkey = vault.subkey(Purpose::Thumbnail);
    let vault_id = vault.vault_id();

    let plaintext = am_crypto::open(
        &subkey,
        ObjectContext {
            vault_id: &vault_id,
            object_id: &id_bytes,
            purpose: Purpose::Thumbnail.label(),
            format_version: crate::header::FORMAT_VERSION,
        },
        &ciphertext,
    )
    .map_err(|_| ThumbError::Decode)?;

    Ok(Some(plaintext))
}

/// Remove a variant's files and rows. Called when its original is swept.
pub fn purge_variants(vault: &Vault, root: &Path, object_id: &str) -> Result<(), ThumbError> {
    let mut stmt = vault
        .conn()
        .prepare("SELECT variant_object_id FROM media_variants WHERE object_id = ?1")?;
    let ids: Vec<String> =
        stmt.query_map([object_id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    for id in ids {
        let path = variant_path(root, &id)?;
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    vault.conn().execute("DELETE FROM media_variants WHERE object_id = ?1", [object_id])?;
    Ok(())
}

pub fn variant_path(root: &Path, variant_id: &str) -> std::io::Result<std::path::PathBuf> {
    object_path(&root.join(CACHE_DIR).join("thumbs"), variant_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::import_object;
    use crate::vault::Vault;
    use am_crypto::KdfParams;

    const NOW: &str = "2026-09-19T00:00:00Z";
    const PASS: &str = "correct horse battery staple";

    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    /// A real, decodable PNG of the given size.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn generates_encrypted_variants() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        let stored = import_object(&vault, &root, &png(800, 600), NOW).unwrap();
        let created = generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();
        assert_eq!(created.len(), 2, "both variants should be generated");

        let thumb = load_variant(&vault, &root, &stored.object_id, "thumb:256")
            .unwrap()
            .expect("variant should exist");
        let decoded = image::load_from_memory(&thumb).unwrap();
        assert!(decoded.width() <= 256 && decoded.height() <= 256);
    }

    #[test]
    fn variant_files_are_ciphertext_with_opaque_names() {
        // The thumbnail cache is the most commonly missed leak.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        let stored = import_object(&vault, &root, &png(400, 400), NOW).unwrap();
        generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();

        let variant_id: String = vault
            .conn()
            .query_row(
                "SELECT variant_object_id FROM media_variants WHERE object_id = ?1 LIMIT 1",
                [&stored.object_id],
                |r| r.get(0),
            )
            .unwrap();

        let path = variant_path(&root, &variant_id).unwrap();
        let bytes = std::fs::read(&path).unwrap();

        assert!(!bytes.starts_with(b"\xFF\xD8\xFF"), "thumbnail stored as plaintext JPEG");
        assert!(!bytes.starts_with(b"\x89PNG"), "thumbnail stored as plaintext PNG");
        assert!(path.extension().is_none(), "variant filename leaks a type");
        assert!(
            !path.to_string_lossy().contains(&stored.object_id),
            "variant filename must not reveal which original it derives from"
        );
    }

    #[test]
    fn thumbnails_do_not_decrypt_under_the_object_key() {
        // Purpose separation: a thumbnail must not be servable as an original.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        let stored = import_object(&vault, &root, &png(400, 400), NOW).unwrap();
        generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();

        let variant_id: String = vault
            .conn()
            .query_row(
                "SELECT variant_object_id FROM media_variants WHERE object_id = ?1 LIMIT 1",
                [&stored.object_id],
                |r| r.get(0),
            )
            .unwrap();

        let ciphertext = std::fs::read(variant_path(&root, &variant_id).unwrap()).unwrap();
        let id_bytes = parse_hex16(&variant_id).unwrap();
        let vault_id = vault.vault_id();

        let with_object_key = am_crypto::open(
            &vault.subkey(Purpose::Object),
            ObjectContext {
                vault_id: &vault_id,
                object_id: &id_bytes,
                purpose: Purpose::Object.label(),
                format_version: crate::header::FORMAT_VERSION,
            },
            &ciphertext,
        );
        assert!(with_object_key.is_err(), "thumbnail decrypted under the object purpose");
    }

    #[test]
    fn generation_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        let stored = import_object(&vault, &root, &png(500, 500), NOW).unwrap();
        assert_eq!(generate_variants(&vault, &root, &stored.object_id, NOW).unwrap().len(), 2);
        assert_eq!(
            generate_variants(&vault, &root, &stored.object_id, NOW).unwrap().len(),
            0,
            "a second run must not duplicate variants"
        );

        let count: i64 = vault
            .conn()
            .query_row("SELECT count(*) FROM media_variants", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        let stored = import_object(&vault, &root, &png(64, 48), NOW).unwrap();
        generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();

        let thumb =
            load_variant(&vault, &root, &stored.object_id, "thumb:256").unwrap().unwrap();
        let decoded = image::load_from_memory(&thumb).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (64, 48), "must not upscale");
    }

    #[test]
    fn records_original_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        let stored = import_object(&vault, &root, &png(321, 123), NOW).unwrap();
        generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();

        let (w, h): (u32, u32) = vault
            .conn()
            .query_row(
                "SELECT width, height FROM objects WHERE object_id = ?1",
                [&stored.object_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((w, h), (321, 123));
    }

    #[test]
    fn non_images_get_no_variants_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        let pdf = b"%PDF-1.4\n% a document, not an image\n".to_vec();
        let stored = import_object(&vault, &root, &pdf, NOW).unwrap();

        let created = generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();
        assert!(created.is_empty(), "a PDF should yield no thumbnails, not an error");
    }

    #[test]
    fn purge_removes_variant_files_and_rows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let (vault, _r) = Vault::create(&root, PASS, &fast(), NOW).unwrap();

        let stored = import_object(&vault, &root, &png(300, 300), NOW).unwrap();
        generate_variants(&vault, &root, &stored.object_id, NOW).unwrap();

        let ids: Vec<String> = {
            let mut stmt = vault
                .conn()
                .prepare("SELECT variant_object_id FROM media_variants WHERE object_id = ?1")
                .unwrap();
            let v = stmt
                .query_map([&stored.object_id], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            v
        };
        assert_eq!(ids.len(), 2);

        purge_variants(&vault, &root, &stored.object_id).unwrap();

        for id in &ids {
            assert!(!variant_path(&root, id).unwrap().exists(), "variant file should be gone");
        }
        let count: i64 = vault
            .conn()
            .query_row("SELECT count(*) FROM media_variants", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn decompression_bombs_are_refused() {
        // A PNG declaring enormous dimensions must be rejected on its header,
        // before any pixel buffer is allocated.
        let width = 30_000u32;
        let height = 30_000u32; // 900M pixels, well over the limit
        assert!(u64::from(width) * u64::from(height) > MAX_DECODED_PIXELS);

        // Hand-built PNG header with the huge dimensions.
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
        bytes.extend_from_slice(&0u32.to_be_bytes()); // CRC placeholder

        match decode_bounded(&bytes) {
            Err(ThumbError::TooManyPixels { .. }) => {}
            // A malformed CRC may make this undecodable first; either refusal
            // is acceptable, silently decoding it is not.
            Err(ThumbError::Decode) => {}
            other => panic!("decompression bomb was not refused: {other:?}"),
        }
    }
}
