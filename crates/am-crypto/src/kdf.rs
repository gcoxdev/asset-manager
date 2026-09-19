//! Passphrase-derived key-encryption keys and the vault key hierarchy.
//!
//! Adapted from `qiring-crypto` (AGPL-3.0), with two additions this project
//! needs and QiRing does not: domain-separated subkeys, and a recovery key
//! encoded for reliable transcription off a printed sheet.

use argon2::{Argon2, Params};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const SALT_LEN: usize = 16;
pub const KEY_LEN: usize = 32;

// Bounds carried over from QiRing: a tampered header must not be able to force
// absurd work (a denial of service) or trivially weak work (a broken vault).
pub const MIN_MEMORY_COST_KIB: u32 = 8 * 1024;
pub const MAX_MEMORY_COST_KIB: u32 = 256 * 1024;
pub const MIN_ITERATIONS: u32 = 1;
pub const MAX_ITERATIONS: u32 = 10;
pub const MIN_PARALLELISM: u32 = 1;
pub const MAX_PARALLELISM: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub memory_cost_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self { memory_cost_kib: 64 * 1024, iterations: 3, parallelism: 1 }
    }
}

impl KdfParams {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !(MIN_MEMORY_COST_KIB..=MAX_MEMORY_COST_KIB).contains(&self.memory_cost_kib)
            || !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&self.iterations)
            || !(MIN_PARALLELISM..=MAX_PARALLELISM).contains(&self.parallelism)
        {
            anyhow::bail!("KDF parameters are outside supported resource bounds");
        }
        Ok(())
    }
}

pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    OsRng.fill_bytes(&mut out);
    out
}

pub fn random_salt() -> [u8; SALT_LEN] {
    let mut out = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut out);
    out
}

pub fn random_key() -> Zeroizing<[u8; KEY_LEN]> {
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    OsRng.fill_bytes(out.as_mut());
    out
}

/// Derive a key-encryption key from a passphrase (or a recovery key).
pub fn derive_kek(
    secret: &str,
    salt: &[u8],
    params: &KdfParams,
) -> anyhow::Result<Zeroizing<[u8; KEY_LEN]>> {
    if salt.len() != SALT_LEN {
        anyhow::bail!("salt must be {SALT_LEN} bytes");
    }
    params.validate()?;

    let p = Params::new(params.memory_cost_kib, params.iterations, params.parallelism, Some(KEY_LEN))
        .map_err(|e| anyhow::anyhow!("invalid argon2 parameters: {e}"))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, p);

    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(secret.as_bytes(), salt, out.as_mut())
        .map_err(|e| anyhow::anyhow!("argon2 derivation failed: {e}"))?;
    Ok(out)
}

/// Purposes a subkey can be derived for. Distinct purposes must never share a
/// key: reusing one key across the database, objects and thumbnails would let
/// a weakness in one context affect the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Database,
    Object,
    Thumbnail,
}

impl Purpose {
    /// Domain-separation label. These strings are part of the on-disk format:
    /// changing one makes existing vaults undecryptable for that purpose.
    pub const fn label(self) -> &'static str {
        match self {
            Purpose::Database => "am/v1/db",
            Purpose::Object => "am/v1/object",
            Purpose::Thumbnail => "am/v1/thumb",
        }
    }
}

/// Derive a purpose-specific subkey from the vault data key.
///
/// HKDF-SHA256 expand (RFC 5869). The data key is already uniformly random,
/// so no extract step is needed — `from_prk` takes it directly.
///
/// The `info` string is **length-prefixed**, not bare: concatenating
/// unprefixed fields lets distinct inputs collide (`"ab"+"c"` == `"a"+"bc"`).
/// Cheap here, and the habit matters once info strings carry more than one
/// field.
///
/// Deliberately NOT `derive_kek` with the label as a password: that would
/// cost 64 MiB of Argon2 per subkey for no security gain, since the data key
/// is already high-entropy.
pub fn derive_subkey(data_key: &[u8; KEY_LEN], purpose: Purpose) -> Zeroizing<[u8; KEY_LEN]> {
    let label = purpose.label().as_bytes();
    let mut info = Vec::with_capacity(8 + label.len());
    info.extend_from_slice(&(label.len() as u64).to_le_bytes());
    info.extend_from_slice(label);

    let hk = hkdf::Hkdf::<sha2::Sha256>::from_prk(data_key)
        .expect("32-byte PRK is a valid length for HKDF-SHA256");
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    hk.expand(&info, out.as_mut()).expect("32 bytes is within HKDF-SHA256 output limits");
    out
}

/// Generate a recovery key: 160 bits, base32 (Crockford-style alphabet via
/// data-encoding's RFC4648 without padding), grouped for transcription.
///
/// Base32 rather than QiRing's base64: a recovery key is read back off a
/// printed sheet, where case-insensitivity and the absence of visually similar
/// glyph pairs matter more than density.
pub fn generate_recovery_key() -> String {
    let entropy = random_bytes(20);
    let encoded = data_encoding::BASE32_NOPAD.encode(&entropy);
    encoded
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).expect("base32 is ascii"))
        .collect::<Vec<_>>()
        .join("-")
}

/// Normalize a user-typed recovery key: strip grouping and whitespace,
/// uppercase. Accepts the key with or without dashes.
pub fn normalize_recovery_key(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Short, non-secret identifier for a recovery key, so a stored backup can be
/// told apart from another vault's without revealing the key itself.
pub fn recovery_fingerprint(recovery_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"am/v1/recovery-fingerprint");
    hasher.update(normalize_recovery_key(recovery_key).as_bytes());
    let digest = hasher.finalize();
    data_encoding::BASE32_NOPAD.encode(&digest[..5])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kdf_params_reject_out_of_bounds() {
        assert!(KdfParams::default().validate().is_ok());
        assert!(KdfParams { memory_cost_kib: 1, iterations: 3, parallelism: 1 }.validate().is_err());
        assert!(KdfParams { memory_cost_kib: 64 * 1024, iterations: 99, parallelism: 1 }
            .validate()
            .is_err());
        assert!(KdfParams { memory_cost_kib: 1024 * 1024, iterations: 3, parallelism: 1 }
            .validate()
            .is_err());
    }

    #[test]
    fn derive_kek_is_deterministic_and_salt_dependent() {
        let params = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
        let salt_a = [7u8; SALT_LEN];
        let salt_b = [9u8; SALT_LEN];

        let a1 = derive_kek("correct horse battery staple", &salt_a, &params).unwrap();
        let a2 = derive_kek("correct horse battery staple", &salt_a, &params).unwrap();
        let b = derive_kek("correct horse battery staple", &salt_b, &params).unwrap();

        assert_eq!(a1.as_ref(), a2.as_ref(), "same input must derive the same key");
        assert_ne!(a1.as_ref(), b.as_ref(), "different salt must derive a different key");
    }

    #[test]
    fn subkeys_differ_per_purpose() {
        let data_key = [42u8; KEY_LEN];
        let db = derive_subkey(&data_key, Purpose::Database);
        let obj = derive_subkey(&data_key, Purpose::Object);
        let thumb = derive_subkey(&data_key, Purpose::Thumbnail);

        assert_ne!(db.as_ref(), obj.as_ref());
        assert_ne!(obj.as_ref(), thumb.as_ref());
        assert_ne!(db.as_ref(), thumb.as_ref());
        assert_ne!(db.as_ref(), &data_key, "subkey must not equal the data key");
    }

    #[test]
    fn subkey_derivation_is_stable() {
        // Guards the on-disk format: if this changes, existing vaults break.
        let data_key = [1u8; KEY_LEN];
        let a = derive_subkey(&data_key, Purpose::Object);
        let b = derive_subkey(&data_key, Purpose::Object);
        assert_eq!(a.as_ref(), b.as_ref());
    }

    #[test]
    fn recovery_key_roundtrips_through_transcription() {
        let key = generate_recovery_key();
        assert!(key.contains('-'), "grouped for readability: {key}");

        // Typed back in lowercase, without dashes, with stray spaces.
        let typed = key.to_lowercase().replace('-', " ");
        assert_eq!(normalize_recovery_key(&key), normalize_recovery_key(&typed));
    }

    #[test]
    fn recovery_keys_are_distinct_and_fingerprints_track_them() {
        let a = generate_recovery_key();
        let b = generate_recovery_key();
        assert_ne!(a, b);
        assert_ne!(recovery_fingerprint(&a), recovery_fingerprint(&b));
        // Fingerprint is stable across transcription variants.
        assert_eq!(recovery_fingerprint(&a), recovery_fingerprint(&a.to_lowercase()));
        // ...and does not leak the key.
        assert!(!a.contains(&recovery_fingerprint(&a)));
    }
}
