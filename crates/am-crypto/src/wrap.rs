//! Wrapping the vault data key under a key-encryption key.
//!
//! Adapted from `qiring-crypto` (AGPL-3.0). A single small AEAD blob, unlike
//! the chunked format in [`crate::stream`] — a 32-byte key needs no streaming.
//!
//! The AAD is where header authentication happens: callers pass the encoded
//! header metadata, so tampering with KDF parameters or the vault ID makes
//! the unwrap fail. See `am-storage`'s header module for what goes in it.

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::kdf::KEY_LEN;
use crate::stream::NONCE_LEN;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedKey {
    #[serde(with = "hex_bytes")]
    pub nonce: Vec<u8>,
    #[serde(with = "hex_bytes")]
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum WrapError {
    #[error("key wrapping failed")]
    Wrap,
    /// Opaque on purpose: a caller must not be able to distinguish "wrong
    /// passphrase" from "tampered header" by the error alone.
    #[error("key unwrapping failed")]
    Unwrap,
}

/// Encrypt the data key under a KEK, binding it to `aad`.
pub fn wrap_data_key(
    kek: &[u8; KEY_LEN],
    data_key: &[u8; KEY_LEN],
    aad: &[u8],
) -> Result<WrappedKey, WrapError> {
    let cipher = XChaCha20Poly1305::new_from_slice(kek).map_err(|_| WrapError::Wrap)?;

    let mut nonce = [0u8; NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut nonce);

    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: data_key.as_slice(), aad })
        .map_err(|_| WrapError::Wrap)?;

    Ok(WrappedKey { nonce: nonce.to_vec(), ciphertext })
}

/// Recover the data key. Fails if the KEK is wrong *or* `aad` differs from
/// what was used to wrap — which is how header tampering is detected.
pub fn unwrap_data_key(
    kek: &[u8; KEY_LEN],
    wrapped: &WrappedKey,
    aad: &[u8],
) -> Result<Zeroizing<[u8; KEY_LEN]>, WrapError> {
    if wrapped.nonce.len() != NONCE_LEN {
        return Err(WrapError::Unwrap);
    }
    let cipher = XChaCha20Poly1305::new_from_slice(kek).map_err(|_| WrapError::Unwrap)?;

    let plain = cipher
        .decrypt(
            XNonce::from_slice(&wrapped.nonce),
            Payload { msg: wrapped.ciphertext.as_slice(), aad },
        )
        .map_err(|_| WrapError::Unwrap)?;

    if plain.len() != KEY_LEN {
        return Err(WrapError::Unwrap);
    }
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    out.copy_from_slice(&plain);
    Ok(out)
}

/// Hex in the serialized header: it survives JSON round-trips without base64
/// padding surprises, and a human can eyeball a corrupted field.
mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        if s.len() % 2 != 0 {
            return Err(serde::de::Error::custom("odd-length hex string"));
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(serde::de::Error::custom))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEK: [u8; KEY_LEN] = [3u8; KEY_LEN];
    const DATA_KEY: [u8; KEY_LEN] = [9u8; KEY_LEN];

    #[test]
    fn roundtrips() {
        let wrapped = wrap_data_key(&KEK, &DATA_KEY, b"header-aad").unwrap();
        let out = unwrap_data_key(&KEK, &wrapped, b"header-aad").unwrap();
        assert_eq!(out.as_ref(), &DATA_KEY);
    }

    #[test]
    fn rejects_wrong_kek() {
        let wrapped = wrap_data_key(&KEK, &DATA_KEY, b"aad").unwrap();
        assert!(unwrap_data_key(&[4u8; KEY_LEN], &wrapped, b"aad").is_err());
    }

    #[test]
    fn rejects_modified_aad() {
        // The header-tampering case: same passphrase, altered metadata.
        let wrapped = wrap_data_key(&KEK, &DATA_KEY, b"memory=64MiB").unwrap();
        assert!(
            unwrap_data_key(&KEK, &wrapped, b"memory=8MiB").is_err(),
            "changing authenticated metadata must break the unwrap"
        );
    }

    #[test]
    fn rejects_tampered_ciphertext() {
        let mut wrapped = wrap_data_key(&KEK, &DATA_KEY, b"aad").unwrap();
        wrapped.ciphertext[0] ^= 0x01;
        assert!(unwrap_data_key(&KEK, &wrapped, b"aad").is_err());
    }

    #[test]
    fn rejects_malformed_nonce() {
        let mut wrapped = wrap_data_key(&KEK, &DATA_KEY, b"aad").unwrap();
        wrapped.nonce.truncate(4);
        assert!(unwrap_data_key(&KEK, &wrapped, b"aad").is_err());
    }

    #[test]
    fn two_wraps_of_one_key_differ() {
        // Distinct nonces, so the two unlock paths don't reveal they wrap the
        // same data key.
        let a = wrap_data_key(&KEK, &DATA_KEY, b"aad").unwrap();
        let b = wrap_data_key(&KEK, &DATA_KEY, b"aad").unwrap();
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn survives_json_roundtrip() {
        let wrapped = wrap_data_key(&KEK, &DATA_KEY, b"aad").unwrap();
        let json = serde_json::to_string(&wrapped).unwrap();
        let parsed: WrappedKey = serde_json::from_str(&json).unwrap();
        assert_eq!(wrapped, parsed);
        assert_eq!(unwrap_data_key(&KEK, &parsed, b"aad").unwrap().as_ref(), &DATA_KEY);
    }
}
