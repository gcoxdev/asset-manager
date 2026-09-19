//! Cryptography for Asset Manager vaults.
//!
//! Key hierarchy, following QiRing (AGPL-3.0, vendored — see VENDOR.md):
//!
//! ```text
//! passphrase ──Argon2id──> KEK ──wraps──┐
//!                                       ├──> data key ──HKDF──> per-purpose subkeys
//! recovery key ─Argon2id──> KEK ──wraps──┘                        (db / object / thumb)
//! ```
//!
//! Two independent unlock paths wrap the *same* data key, so recovery works
//! without a backdoor and a passphrase change need not re-encrypt content.
//!
//! What this crate does NOT do, deliberately:
//! - decide policy (lock timeouts, retry limits) — that belongs to the app
//! - touch the filesystem — callers own atomic writes and durability
//! - log anything — no secret can leak through a log this way

pub mod kdf;
pub mod stream;
pub mod wrap;

pub use kdf::{
    derive_kek, derive_subkey, generate_recovery_key, normalize_recovery_key, random_bytes,
    random_key, random_salt, recovery_fingerprint, KdfParams, Purpose, KEY_LEN, SALT_LEN,
};
pub use stream::{open, seal, ObjectContext, StreamError, CHUNK_SIZE, MAX_OBJECT_BYTES};
pub use wrap::{unwrap_data_key, wrap_data_key, WrapError, WrappedKey};
