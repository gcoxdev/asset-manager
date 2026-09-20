//! Chunked streaming AEAD for photo objects.
//!
//! Photos run 2-8 MB, so they are encrypted in chunks rather than held whole
//! in memory. The format below is Option B from the plan: explicit chunked
//! XChaCha20-Poly1305, which fits the vault key hierarchy directly.
//!
//! # Format
//!
//! ```text
//! [24-byte base nonce] [chunk 0] [chunk 1] ... [chunk n]
//! ```
//!
//! Each chunk is `CHUNK_SIZE` plaintext bytes (the last may be shorter),
//! encrypted to `CHUNK_SIZE + TAG_LEN` bytes. The per-chunk nonce is the base
//! nonce with the chunk index XORed into its last 8 bytes.
//!
//! # Nonce derivation (explicit, because getting this wrong is fatal)
//!
//! The 24-byte base nonce is **freshly random per seal**, from the OS RNG, and
//! stored in the clear at the head of the object. Per-chunk nonces XOR the
//! little-endian chunk index into its last 8 bytes.
//!
//! Crucially, the nonce is **not derived from the object ID**. That would be a
//! trap: object IDs are random 128-bit values, so deriving a nonce from one
//! gives no birthday-bound protection against two objects sharing a
//! key+nonce pair — and XChaCha20-Poly1305 fails catastrophically under nonce
//! reuse (keystream reuse, and forgeable tags). A fresh random 192-bit nonce
//! per object makes collision probability negligible (~1e-46 at millions of
//! objects) and is re-randomized on every re-encryption.
//!
//! # Why the AAD carries an end-of-stream flag
//!
//! A chunk index alone does not make truncation detectable: dropping whole
//! chunks from the end yields a shorter but individually-valid sequence. Each
//! chunk therefore authenticates `context || index || is_final`, so the final
//! chunk is cryptographically marked as final. Removing it leaves the new last
//! chunk authenticating `is_final = false`, which the reader rejects.
//!
//! The chunk index stays in the AAD **in addition to** the final flag, so
//! reordering is caught as well as truncation. AAD is per-chunk, not per-file.

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use zeroize::Zeroizing;

use crate::kdf::KEY_LEN;

pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

/// 64 KiB plaintext per chunk, matching the age STREAM convention.
pub const CHUNK_SIZE: usize = 64 * 1024;

/// Refuse absurd inputs rather than allocating unboundedly. 512 MiB is far
/// above any real photo while still bounding a hostile or corrupt length.
pub const MAX_OBJECT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("encryption failure")]
    Encrypt,
    /// Deliberately opaque: callers must not learn which chunk failed or why,
    /// only that the object did not authenticate.
    #[error("decryption failure")]
    Decrypt,
    #[error("ciphertext is malformed")]
    Malformed,
    #[error("object exceeds the maximum supported size")]
    TooLarge,
}

/// Binds ciphertext to where it lives, so an object cannot be swapped for
/// another, reused under a different purpose, or replayed into another vault.
///
/// Note this deliberately does NOT include an asset id: one object may be
/// attached to several assets, and moving an attachment must not invalidate
/// the bytes. The asset association lives in the encrypted database.
#[derive(Debug, Clone, Copy)]
pub struct ObjectContext<'a> {
    pub vault_id: &'a [u8; 16],
    pub object_id: &'a [u8; 16],
    /// `crate::kdf::Purpose::label()`, e.g. `am/v1/object`.
    pub purpose: &'a str,
    pub format_version: u16,
}

impl ObjectContext<'_> {
    /// Length-prefix every field so distinct contexts cannot collide by
    /// concatenation (e.g. purposes "ab"+"c" vs "a"+"bc").
    fn encode(&self, chunk_index: u64, is_final: bool) -> Vec<u8> {
        let purpose = self.purpose.as_bytes();
        let mut out = Vec::with_capacity(16 + 16 + 2 + 8 + purpose.len() + 8 + 1);
        out.extend_from_slice(self.vault_id);
        out.extend_from_slice(self.object_id);
        out.extend_from_slice(&self.format_version.to_le_bytes());
        out.extend_from_slice(&(purpose.len() as u64).to_le_bytes());
        out.extend_from_slice(purpose);
        out.extend_from_slice(&chunk_index.to_le_bytes());
        out.push(u8::from(is_final));
        out
    }
}

fn chunk_nonce(base: &[u8; NONCE_LEN], index: u64) -> XNonce {
    let mut nonce = *base;
    let idx = index.to_le_bytes();
    for (slot, byte) in nonce[NONCE_LEN - 8..].iter_mut().zip(idx) {
        *slot ^= byte;
    }
    *XNonce::from_slice(&nonce)
}

/// Encrypt `plaintext` into the chunked format described above.
pub fn seal(
    key: &[u8; KEY_LEN],
    ctx: ObjectContext<'_>,
    plaintext: &[u8],
) -> Result<Vec<u8>, StreamError> {
    if plaintext.len() as u64 > MAX_OBJECT_BYTES {
        return Err(StreamError::TooLarge);
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| StreamError::Encrypt)?;

    let mut base = [0u8; NONCE_LEN];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut base);

    let mut out = Vec::with_capacity(NONCE_LEN + plaintext.len() + TAG_LEN);
    out.extend_from_slice(&base);

    // An empty object still writes one final (empty) chunk, so that "no
    // chunks at all" is always malformed rather than a valid empty object.
    let chunks: Vec<&[u8]> =
        if plaintext.is_empty() { vec![&[]] } else { plaintext.chunks(CHUNK_SIZE).collect() };
    let last = chunks.len() - 1;

    for (index, chunk) in chunks.into_iter().enumerate() {
        let is_final = index == last;
        let aad = ctx.encode(index as u64, is_final);
        let sealed = cipher
            .encrypt(&chunk_nonce(&base, index as u64), Payload { msg: chunk, aad: &aad })
            .map_err(|_| StreamError::Encrypt)?;
        out.extend_from_slice(&sealed);
    }

    Ok(out)
}

/// Decrypt a chunked object, rejecting truncation, reordering and tampering.
pub fn open(
    key: &[u8; KEY_LEN],
    ctx: ObjectContext<'_>,
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, StreamError> {
    if ciphertext.len() as u64 > MAX_OBJECT_BYTES + (NONCE_LEN + TAG_LEN) as u64 {
        return Err(StreamError::TooLarge);
    }
    if ciphertext.len() < NONCE_LEN + TAG_LEN {
        return Err(StreamError::Malformed);
    }

    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| StreamError::Decrypt)?;
    let mut base = [0u8; NONCE_LEN];
    base.copy_from_slice(&ciphertext[..NONCE_LEN]);
    let body = &ciphertext[NONCE_LEN..];

    let sealed_chunk = CHUNK_SIZE + TAG_LEN;
    let mut out = Zeroizing::new(Vec::with_capacity(body.len()));

    let pieces: Vec<&[u8]> = body.chunks(sealed_chunk).collect();
    let last = pieces.len().checked_sub(1).ok_or(StreamError::Malformed)?;

    for (index, piece) in pieces.into_iter().enumerate() {
        if piece.len() < TAG_LEN {
            return Err(StreamError::Malformed);
        }

        // `is_final` comes from position in the stream, NOT from whether the
        // piece is short. Inferring it from length silently breaks objects
        // whose size is an exact multiple of CHUNK_SIZE (their true final
        // chunk is full-length), and — far worse — makes whole-chunk
        // truncation undetectable, since the new trailing chunk would also be
        // read as "final". Both were caught by the tests below.
        let is_final = index == last;

        let aad = ctx.encode(index as u64, is_final);
        let plain = cipher
            .decrypt(&chunk_nonce(&base, index as u64), Payload { msg: piece, aad: &aad })
            .map_err(|_| StreamError::Decrypt)?;
        out.extend_from_slice(&plain);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VAULT: [u8; 16] = [1u8; 16];
    const OBJECT: [u8; 16] = [2u8; 16];

    fn ctx() -> ObjectContext<'static> {
        ObjectContext {
            vault_id: &VAULT,
            object_id: &OBJECT,
            purpose: "am/v1/object",
            format_version: 1,
        }
    }

    fn key() -> [u8; KEY_LEN] {
        [7u8; KEY_LEN]
    }

    #[test]
    fn roundtrips_across_chunk_boundaries() {
        // Empty, sub-chunk, exactly one chunk, one byte over, and several
        // chunks -- the off-by-one cases are where framing bugs hide.
        for len in [0, 1, 100, CHUNK_SIZE - 1, CHUNK_SIZE, CHUNK_SIZE + 1, CHUNK_SIZE * 3 + 17]
        {
            let plaintext: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let sealed = seal(&key(), ctx(), &plaintext).unwrap();
            let opened = open(&key(), ctx(), &sealed).unwrap();
            assert_eq!(opened.as_slice(), plaintext.as_slice(), "failed at len {len}");
        }
    }

    #[test]
    fn ciphertext_does_not_contain_plaintext() {
        // A photo's magic bytes must not survive into the stored object.
        let mut plaintext = vec![0xFF, 0xD8, 0xFF, 0xE0];
        plaintext.extend(std::iter::repeat_n(0xAB, 4096));
        let sealed = seal(&key(), ctx(), &plaintext).unwrap();
        assert!(
            !sealed.windows(4).any(|w| w == [0xFF, 0xD8, 0xFF, 0xE0]),
            "JPEG signature leaked into ciphertext"
        );
    }

    #[test]
    fn rejects_truncated_final_chunk() {
        // The attack the end-of-stream flag exists to stop: drop whole chunks
        // from the end and the remainder must not verify as a complete object.
        let plaintext = vec![9u8; CHUNK_SIZE * 3];
        let sealed = seal(&key(), ctx(), &plaintext).unwrap();

        let one_chunk_removed = &sealed[..sealed.len() - (CHUNK_SIZE + TAG_LEN)];
        assert!(
            open(&key(), ctx(), one_chunk_removed).is_err(),
            "truncation must be detected, not silently returned as a shorter object"
        );
    }

    #[test]
    fn rejects_byte_level_truncation() {
        let plaintext = vec![3u8; 5000];
        let sealed = seal(&key(), ctx(), &plaintext).unwrap();
        assert!(open(&key(), ctx(), &sealed[..sealed.len() - 1]).is_err());
    }

    #[test]
    fn rejects_tampering() {
        let plaintext = vec![4u8; 4096];
        let sealed = seal(&key(), ctx(), &plaintext).unwrap();

        let mut flipped = sealed.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 0x01;
        assert!(open(&key(), ctx(), &flipped).is_err(), "tag tampering must be rejected");

        let mut nonce_flipped = sealed.clone();
        nonce_flipped[0] ^= 0x01;
        assert!(
            open(&key(), ctx(), &nonce_flipped).is_err(),
            "nonce tampering must be rejected"
        );
    }

    #[test]
    fn rejects_reordered_chunks() {
        let plaintext: Vec<u8> = (0..(CHUNK_SIZE * 2)).map(|i| (i % 251) as u8).collect();
        let sealed = seal(&key(), ctx(), &plaintext).unwrap();

        let sealed_chunk = CHUNK_SIZE + TAG_LEN;
        let mut swapped = Vec::with_capacity(sealed.len());
        swapped.extend_from_slice(&sealed[..NONCE_LEN]);
        swapped
            .extend_from_slice(&sealed[NONCE_LEN + sealed_chunk..NONCE_LEN + 2 * sealed_chunk]);
        swapped.extend_from_slice(&sealed[NONCE_LEN..NONCE_LEN + sealed_chunk]);

        assert!(open(&key(), ctx(), &swapped).is_err(), "reordering must be rejected");
    }

    #[test]
    fn rejects_wrong_key() {
        let sealed = seal(&key(), ctx(), b"secret photo bytes").unwrap();
        assert!(open(&[8u8; KEY_LEN], ctx(), &sealed).is_err());
    }

    #[test]
    fn rejects_context_substitution() {
        let sealed = seal(&key(), ctx(), b"secret photo bytes").unwrap();

        // Same key, different object id: an attacker moving bytes between
        // objects inside one vault.
        let other_object = [3u8; 16];
        let moved = ObjectContext { object_id: &other_object, ..ctx() };
        assert!(open(&key(), moved, &sealed).is_err(), "object id must be bound");

        // Same key, different vault: bytes replayed into a restored vault.
        let other_vault = [4u8; 16];
        let replayed = ObjectContext { vault_id: &other_vault, ..ctx() };
        assert!(open(&key(), replayed, &sealed).is_err(), "vault id must be bound");

        // Same key, thumbnail purpose: a thumbnail served as an original.
        let repurposed = ObjectContext { purpose: "am/v1/thumb", ..ctx() };
        assert!(open(&key(), repurposed, &sealed).is_err(), "purpose must be bound");

        let rolled = ObjectContext { format_version: 2, ..ctx() };
        assert!(open(&key(), rolled, &sealed).is_err(), "format version must be bound");
    }

    #[test]
    fn rejects_empty_and_stub_ciphertext() {
        assert!(open(&key(), ctx(), &[]).is_err());
        assert!(open(&key(), ctx(), &[0u8; NONCE_LEN]).is_err());
        assert!(open(&key(), ctx(), &[0u8; NONCE_LEN + TAG_LEN - 1]).is_err());
    }

    #[test]
    fn distinct_seals_use_distinct_nonces() {
        let a = seal(&key(), ctx(), b"same plaintext").unwrap();
        let b = seal(&key(), ctx(), b"same plaintext").unwrap();
        assert_ne!(a, b, "nonce reuse would leak that two objects are identical");
    }
}
