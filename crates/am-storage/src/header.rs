//! The vault header: `vault.header`, unencrypted but authenticated.
//!
//! # Why it exists
//!
//! Unlocking needs metadata *before* anything can be decrypted — the format
//! version, the KDF salts and parameters, and the wrapped data key. Storing
//! those only inside the encrypted database would be circular: the vault
//! could never be opened at all.
//!
//! # How "authenticated" works
//!
//! There is **no separate MAC**, and there cannot be — the key to verify one
//! with would itself derive from salts stored in this very file. Instead,
//! the header fields are folded into the **AAD of the wrapped data key**.
//! Tampering is detected because the unwrap fails.
//!
//! Two consequences, accepted deliberately:
//!
//! - Tamper detection requires the correct credential. A hostile header edit
//!   is indistinguishable from a wrong passphrase until the right one is
//!   entered. So both report the same thing — see [`UnlockError`].
//! - **Every field must be listed in [`AuthenticatedHeader`] or it is simply
//!   unprotected.** Adding a field to `VaultHeader` without adding it there
//!   leaves it silently editable.

use am_crypto::{
    derive_kek, kdf::KEY_LEN, normalize_recovery_key, unwrap_data_key, wrap_data_key,
    KdfParams, WrappedKey,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Bumped only for a breaking change to the on-disk format. Note this is
/// bound into every object's AEAD context, so a bump means re-encrypting
/// every object — see the plan's note on resumable re-encryption.
pub const FORMAT_VERSION: u16 = 1;

/// How many earlier epochs [`VaultHeader::unlock_orphaned_slot`] will try.
/// Every credential change advances the epoch by one, so this covers far more
/// changes than any vault made by an older build will have seen, while
/// keeping the work a tampered header can demand to milliseconds.
pub const MAX_ORPHAN_EPOCH_RETRIES: u64 = 4096;

/// Identifies the file and prevents cross-format confusion.
pub const FORMAT_TAG: &str = "asset-manager-vault-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KdfSlot {
    #[serde(with = "hex16")]
    pub salt: [u8; 16],
    pub params: KdfParams,
    pub wrapped: WrappedKey,
    /// The `key_epoch` this slot was wrapped under, when it differs from the
    /// header's current epoch.
    ///
    /// Each slot's AAD binds an epoch. Changing one credential advances the
    /// header epoch but can only re-wrap *that* credential's slot — the other
    /// slot's KEK comes from a secret the app does not hold. Without this
    /// field the untouched slot's AAD would silently change and that
    /// credential would stop working: changing the passphrase used to break
    /// the recovery key, and rotating the recovery key broke the passphrase.
    ///
    /// Absent means "the header's epoch", which is exactly how every slot was
    /// authenticated before this field existed, so older headers read and
    /// unlock unchanged. The value is itself inside the AAD, so editing it
    /// breaks the unwrap like any other tampering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapped_at_epoch: Option<u64>,
}

/// One unlock path. Both wrap the *same* data key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credential {
    Passphrase,
    RecoveryKey,
}

impl Credential {
    const fn aad_purpose(self) -> &'static str {
        match self {
            Credential::Passphrase => "am/v1/wrap/passphrase",
            Credential::RecoveryKey => "am/v1/wrap/recovery",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultHeader {
    pub format: String,
    pub format_version: u16,
    /// Pairs this header with its database. See [`crate::header::PairingError`].
    #[serde(with = "hex16")]
    pub vault_id: [u8; 16],
    /// Incremented on every credential change or rewrap, so a stale header
    /// cannot be paired with a newer database.
    pub key_epoch: u64,
    pub created_at: String,
    pub passphrase_slot: KdfSlot,
    pub recovery_slot: KdfSlot,
    /// Non-secret identifier for the recovery key, so a user can tell which
    /// printed sheet belongs to this vault without revealing the key.
    pub recovery_fingerprint: String,
}

/// Exactly the fields bound into the wrapped-key AAD.
///
/// Field order is part of the format: this serializes to canonical JSON, so
/// reordering the struct changes the AAD and breaks every existing vault.
#[derive(Serialize)]
struct AuthenticatedHeader<'a> {
    format: &'a str,
    format_version: u16,
    purpose: &'a str,
    #[serde(with = "hex16")]
    vault_id: [u8; 16],
    key_epoch: u64,
    created_at: &'a str,
    /// Only the slot being unwrapped. Including both would mean rotating one
    /// credential invalidates the other.
    #[serde(with = "hex16")]
    slot_salt: [u8; 16],
    slot_params: &'a KdfParams,
}

#[derive(Debug, thiserror::Error)]
pub enum UnlockError {
    /// Deliberately one error for both wrong-credential and tampered-header.
    /// Distinguishing them would tell an attacker editing the header whether
    /// they had guessed the passphrase.
    #[error("vault cannot be unlocked: wrong credential, or the header has been modified")]
    CannotUnlock,
    #[error("unsupported vault format: {0}")]
    UnsupportedFormat(String),
    #[error("header is malformed: {0}")]
    Malformed(String),
}

/// Header and database disagree about which vault they belong to.
#[derive(Debug, thiserror::Error)]
#[error(
    "vault header and database do not match (header {header_vault}/epoch {header_epoch}, \
     database {db_vault}/epoch {db_epoch}) — this usually means a partial restore mixed \
     files from two different backups"
)]
pub struct PairingError {
    pub header_vault: String,
    pub db_vault: String,
    pub header_epoch: u64,
    pub db_epoch: u64,
}

pub struct NewVault {
    pub header: VaultHeader,
    pub data_key: Zeroizing<[u8; KEY_LEN]>,
    /// Shown once, then never recoverable. The caller must run the recovery
    /// ceremony before discarding it.
    pub recovery_key: String,
}

impl VaultHeader {
    /// Create a vault: generate a data key, wrap it under both credentials.
    pub fn create(passphrase: &str, params: &KdfParams, now: &str) -> anyhow::Result<NewVault> {
        params.validate()?;

        let data_key = am_crypto::random_key();
        let recovery_key = am_crypto::generate_recovery_key();

        let mut vault_id = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut vault_id);

        let passphrase_salt = am_crypto::random_salt();
        let recovery_salt = am_crypto::random_salt();

        // Built in two passes: the AAD covers the slot's own salt and params,
        // so those must be decided before wrapping.
        let mut header = VaultHeader {
            format: FORMAT_TAG.to_string(),
            format_version: FORMAT_VERSION,
            vault_id,
            key_epoch: 1,
            created_at: now.to_string(),
            passphrase_slot: KdfSlot {
                salt: passphrase_salt,
                params: params.clone(),
                wrapped: WrappedKey { nonce: Vec::new(), ciphertext: Vec::new() },
                wrapped_at_epoch: None,
            },
            recovery_slot: KdfSlot {
                salt: recovery_salt,
                params: params.clone(),
                wrapped: WrappedKey { nonce: Vec::new(), ciphertext: Vec::new() },
                wrapped_at_epoch: None,
            },
            recovery_fingerprint: am_crypto::recovery_fingerprint(&recovery_key),
        };

        let pass_kek = derive_kek(passphrase, &passphrase_salt, params)?;
        header.passphrase_slot.wrapped =
            wrap_data_key(&pass_kek, &data_key, &header.aad(Credential::Passphrase)?)
                .map_err(|_| anyhow::anyhow!("failed to wrap data key under passphrase"))?;

        let rec_kek =
            derive_kek(&normalize_recovery_key(&recovery_key), &recovery_salt, params)?;
        header.recovery_slot.wrapped =
            wrap_data_key(&rec_kek, &data_key, &header.aad(Credential::RecoveryKey)?)
                .map_err(|_| anyhow::anyhow!("failed to wrap data key under recovery key"))?;

        Ok(NewVault { header, data_key, recovery_key })
    }

    /// Recover the data key using either credential.
    pub fn unlock(
        &self,
        credential: Credential,
        secret: &str,
    ) -> Result<Zeroizing<[u8; KEY_LEN]>, UnlockError> {
        self.check_supported()?;

        let slot = match credential {
            Credential::Passphrase => &self.passphrase_slot,
            Credential::RecoveryKey => &self.recovery_slot,
        };
        // Bounds-check before spending memory on Argon2: a hostile header
        // must not be able to demand gigabytes.
        slot.params.validate().map_err(|e| UnlockError::Malformed(e.to_string()))?;

        let normalized;
        let secret = match credential {
            Credential::Passphrase => secret,
            Credential::RecoveryKey => {
                normalized = normalize_recovery_key(secret);
                &normalized
            }
        };

        let aad = self.aad(credential).map_err(|e| UnlockError::Malformed(e.to_string()))?;
        let kek = derive_kek(secret, &slot.salt, &slot.params)
            .map_err(|_| UnlockError::CannotUnlock)?;

        unwrap_data_key(&kek, &slot.wrapped, &aad).map_err(|_| UnlockError::CannotUnlock)
    }

    /// Re-wrap the data key under a new passphrase.
    ///
    /// Note this does **not** revoke the old passphrase against an older
    /// backup: that copy still holds a wrapper the old passphrase opens, and
    /// the data key is unchanged, so it decrypts newer content too. Changing
    /// the passphrase is not key rotation.
    pub fn change_passphrase(
        &mut self,
        data_key: &[u8; KEY_LEN],
        new_passphrase: &str,
        params: &KdfParams,
    ) -> anyhow::Result<()> {
        params.validate()?;

        let salt = am_crypto::random_salt();
        // The recovery slot cannot be re-wrapped here — its KEK comes from
        // the recovery key, which the app never keeps — so it goes on
        // authenticating under the epoch it was wrapped at.
        self.pin_slot_epoch(Credential::RecoveryKey);
        self.key_epoch += 1;
        self.passphrase_slot.salt = salt;
        self.passphrase_slot.params = params.clone();
        self.passphrase_slot.wrapped_at_epoch = None;

        let pass_kek = derive_kek(new_passphrase, &salt, params)?;
        self.passphrase_slot.wrapped =
            wrap_data_key(&pass_kek, data_key, &self.aad(Credential::Passphrase)?)
                .map_err(|_| anyhow::anyhow!("failed to wrap data key"))?;

        Ok(())
    }

    /// Issue a new recovery key, invalidating the previous one for this vault.
    pub fn rotate_recovery_key(
        &mut self,
        data_key: &[u8; KEY_LEN],
        params: &KdfParams,
    ) -> anyhow::Result<String> {
        params.validate()?;

        let recovery_key = am_crypto::generate_recovery_key();
        let salt = am_crypto::random_salt();

        // Likewise the passphrase slot, whose KEK needs the passphrase.
        self.pin_slot_epoch(Credential::Passphrase);
        self.key_epoch += 1;
        self.recovery_slot.salt = salt;
        self.recovery_slot.params = params.clone();
        self.recovery_slot.wrapped_at_epoch = None;
        self.recovery_fingerprint = am_crypto::recovery_fingerprint(&recovery_key);

        let rec_kek = derive_kek(&normalize_recovery_key(&recovery_key), &salt, params)?;
        self.recovery_slot.wrapped =
            wrap_data_key(&rec_kek, data_key, &self.aad(Credential::RecoveryKey)?)
                .map_err(|_| anyhow::anyhow!("failed to wrap data key"))?;

        Ok(recovery_key)
    }

    /// Verify this header belongs with this database.
    ///
    /// Guards the split-brain case: restoring a backup's `vault.header` beside
    /// a current `catalog.db` yields an intact but undecryptable vault. Without
    /// this check the failure looks like a wrong passphrase, which sends the
    /// user hunting for the wrong problem.
    pub fn check_pairing(
        &self,
        db_vault_id: &[u8; 16],
        db_epoch: u64,
    ) -> Result<(), PairingError> {
        if &self.vault_id == db_vault_id && self.key_epoch == db_epoch {
            return Ok(());
        }
        Err(PairingError {
            header_vault: hex(&self.vault_id),
            db_vault: hex(db_vault_id),
            header_epoch: self.key_epoch,
            db_epoch,
        })
    }

    fn check_supported(&self) -> Result<(), UnlockError> {
        if self.format != FORMAT_TAG {
            return Err(UnlockError::UnsupportedFormat(self.format.clone()));
        }
        if self.format_version > FORMAT_VERSION {
            return Err(UnlockError::UnsupportedFormat(format!(
                "version {} is newer than this build supports ({FORMAT_VERSION})",
                self.format_version
            )));
        }
        Ok(())
    }

    /// Find a slot orphaned by an older build, and the epoch it was wrapped at.
    ///
    /// Before slots recorded their own epoch, changing one credential left
    /// the other authenticating under an epoch the header no longer carried,
    /// so it stopped working. The wrapped key is intact; only the AAD moved.
    /// This derives the KEK once and retries the unwrap at each earlier
    /// epoch. It never succeeds for a wrong credential, and the caller must
    /// still pass the header↔database pairing check, so a header whose epoch
    /// was edited gains nothing from it.
    pub fn unlock_orphaned_slot(
        &self,
        credential: Credential,
        secret: &str,
    ) -> Result<(Zeroizing<[u8; KEY_LEN]>, u64), UnlockError> {
        let slot = match credential {
            Credential::Passphrase => &self.passphrase_slot,
            Credential::RecoveryKey => &self.recovery_slot,
        };
        if slot.wrapped_at_epoch.is_some() || self.key_epoch <= 1 {
            return Err(UnlockError::CannotUnlock);
        }
        self.check_supported()?;
        slot.params.validate().map_err(|e| UnlockError::Malformed(e.to_string()))?;

        let normalized;
        let secret = match credential {
            Credential::Passphrase => secret,
            Credential::RecoveryKey => {
                normalized = normalize_recovery_key(secret);
                &normalized
            }
        };
        let kek = derive_kek(secret, &slot.salt, &slot.params)
            .map_err(|_| UnlockError::CannotUnlock)?;

        // The epoch comes from the unauthenticated header, so the number of
        // attempts is bounded here rather than by it: a header edited to
        // claim epoch 2^64 must not buy an effectively endless loop.
        let oldest = self.key_epoch.saturating_sub(MAX_ORPHAN_EPOCH_RETRIES).max(1);
        for epoch in (oldest..self.key_epoch).rev() {
            let mut candidate = self.clone();
            candidate.pin_slot_epoch_at(credential, epoch);
            let aad =
                candidate.aad(credential).map_err(|e| UnlockError::Malformed(e.to_string()))?;
            if let Ok(key) = unwrap_data_key(&kek, &slot.wrapped, &aad) {
                return Ok((key, epoch));
            }
        }
        Err(UnlockError::CannotUnlock)
    }

    /// Pin a slot to a known epoch — used when repairing an orphaned slot.
    pub fn pin_slot_epoch_at(&mut self, credential: Credential, epoch: u64) {
        let slot = match credential {
            Credential::Passphrase => &mut self.passphrase_slot,
            Credential::RecoveryKey => &mut self.recovery_slot,
        };
        slot.wrapped_at_epoch = Some(epoch);
    }

    /// Record the epoch a slot is authenticated under before the header's
    /// epoch moves on without it.
    fn pin_slot_epoch(&mut self, credential: Credential) {
        let current = self.key_epoch;
        let slot = match credential {
            Credential::Passphrase => &mut self.passphrase_slot,
            Credential::RecoveryKey => &mut self.recovery_slot,
        };
        slot.wrapped_at_epoch.get_or_insert(current);
    }

    fn aad(&self, credential: Credential) -> anyhow::Result<Vec<u8>> {
        let slot = match credential {
            Credential::Passphrase => &self.passphrase_slot,
            Credential::RecoveryKey => &self.recovery_slot,
        };
        let authenticated = AuthenticatedHeader {
            format: &self.format,
            format_version: self.format_version,
            purpose: credential.aad_purpose(),
            vault_id: self.vault_id,
            key_epoch: slot.wrapped_at_epoch.unwrap_or(self.key_epoch),
            created_at: &self.created_at,
            slot_salt: slot.salt,
            slot_params: &slot.params,
        };
        Ok(serde_json::to_vec(&authenticated)?)
    }

    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> Result<Self, UnlockError> {
        let header: VaultHeader =
            serde_json::from_str(s).map_err(|e| UnlockError::Malformed(e.to_string()))?;
        header.check_supported()?;
        Ok(header)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

mod hex16 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 16], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 16], D::Error> {
        let s = String::deserialize(d)?;
        // Checked as bytes before any slicing: a multi-byte character would
        // otherwise put a slice boundary inside it and panic.
        if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(serde::de::Error::custom("expected 32 hex characters"));
        }
        let mut out = [0u8; 16];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cheap Argon2 parameters. Real vaults use the 64 MiB default; tests
    /// would take minutes at that cost.
    fn fast() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    fn new_vault() -> NewVault {
        VaultHeader::create("correct horse battery staple", &fast(), "2026-09-19T00:00:00Z")
            .unwrap()
    }

    #[test]
    fn both_credentials_unlock_the_same_data_key() {
        let v = new_vault();

        let via_pass =
            v.header.unlock(Credential::Passphrase, "correct horse battery staple").unwrap();
        let via_recovery = v.header.unlock(Credential::RecoveryKey, &v.recovery_key).unwrap();

        assert_eq!(via_pass.as_ref(), v.data_key.as_ref());
        assert_eq!(
            via_recovery.as_ref(),
            v.data_key.as_ref(),
            "recovery must reach the same key"
        );
    }

    #[test]
    fn recovery_key_works_as_transcribed_from_print() {
        let v = new_vault();
        // Lowercased, dashes replaced by spaces — how a person retypes it.
        let typed = v.recovery_key.to_lowercase().replace('-', " ");
        let key = v.header.unlock(Credential::RecoveryKey, &typed).unwrap();
        assert_eq!(key.as_ref(), v.data_key.as_ref());
    }

    #[test]
    fn wrong_credentials_fail() {
        let v = new_vault();
        assert!(v.header.unlock(Credential::Passphrase, "wrong").is_err());
        assert!(v.header.unlock(Credential::RecoveryKey, "AAAA-BBBB-CCCC-DDDD").is_err());
    }

    #[test]
    fn header_tampering_is_detected() {
        // The whole point of the AAD design: every authenticated field, when
        // altered, must break the unwrap even with the right passphrase.
        let pass = "correct horse battery staple";

        for (name, mutate) in [
            ("vault_id", (|h: &mut VaultHeader| h.vault_id[0] ^= 0xff) as fn(&mut VaultHeader)),
            ("key_epoch", |h: &mut VaultHeader| h.key_epoch += 1),
            ("created_at", |h: &mut VaultHeader| h.created_at = "2020-01-01T00:00:00Z".into()),
            ("format_version", |h: &mut VaultHeader| h.format_version = 0),
            ("slot salt", |h: &mut VaultHeader| h.passphrase_slot.salt[0] ^= 0xff),
            ("kdf params", |h: &mut VaultHeader| {
                // Within valid bounds, so it passes validation and must be
                // caught by the AAD rather than by the bounds check.
                h.passphrase_slot.params.iterations = 2;
            }),
        ] {
            let v = new_vault();
            let mut header = v.header.clone();
            mutate(&mut header);
            assert!(
                header.unlock(Credential::Passphrase, pass).is_err(),
                "tampering with {name} was not detected"
            );
        }
    }

    #[test]
    fn hostile_kdf_params_are_rejected_before_argon2_runs() {
        // A header demanding 4 GiB must be refused, not attempted.
        let v = new_vault();
        let mut header = v.header;
        header.passphrase_slot.params.memory_cost_kib = 4 * 1024 * 1024;

        let err =
            header.unlock(Credential::Passphrase, "correct horse battery staple").unwrap_err();
        assert!(matches!(err, UnlockError::Malformed(_)), "got {err:?}");
    }

    #[test]
    fn wrong_credential_and_tampered_header_are_indistinguishable() {
        // Both must report the same error, or an attacker editing the header
        // learns whether they guessed the passphrase.
        let v = new_vault();
        let mut tampered = v.header.clone();
        tampered.vault_id[0] ^= 0xff;

        let wrong_pass = v.header.unlock(Credential::Passphrase, "wrong").unwrap_err();
        let tamper = tampered
            .unlock(Credential::Passphrase, "correct horse battery staple")
            .unwrap_err();

        assert_eq!(wrong_pass.to_string(), tamper.to_string());
    }

    #[test]
    fn json_roundtrip_preserves_unlockability() {
        let v = new_vault();
        let json = v.header.to_json().unwrap();
        let parsed = VaultHeader::from_json(&json).unwrap();

        assert_eq!(parsed, v.header);
        let key =
            parsed.unlock(Credential::Passphrase, "correct horse battery staple").unwrap();
        assert_eq!(key.as_ref(), v.data_key.as_ref());
    }

    #[test]
    fn header_json_contains_no_secrets() {
        let v = new_vault();
        let json = v.header.to_json().unwrap();

        assert!(!json.contains("correct horse"), "passphrase leaked into header");
        assert!(!json.contains(&v.recovery_key), "recovery key leaked into header");
        let data_key_hex: String = v.data_key.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!json.contains(&data_key_hex), "data key leaked into header");
    }

    #[test]
    fn rejects_future_format_version() {
        let v = new_vault();
        let mut header = v.header;
        header.format_version = FORMAT_VERSION + 1;

        let json = serde_json::to_string(&header).unwrap();
        assert!(
            matches!(VaultHeader::from_json(&json), Err(UnlockError::UnsupportedFormat(_))),
            "a newer format must be refused clearly, not partially opened"
        );
    }

    #[test]
    fn passphrase_change_keeps_data_key_and_recovery_path() {
        let v = new_vault();
        let mut header = v.header;
        let epoch_before = header.key_epoch;

        header.change_passphrase(&v.data_key, "a brand new passphrase", &fast()).unwrap();

        assert_eq!(header.key_epoch, epoch_before + 1, "epoch must advance");
        assert!(header.unlock(Credential::Passphrase, "correct horse battery staple").is_err());

        let key = header.unlock(Credential::Passphrase, "a brand new passphrase").unwrap();
        assert_eq!(key.as_ref(), v.data_key.as_ref(), "data key must be unchanged");

        // The recovery path must survive, which is the test's name and was
        // once untrue: the recovery slot's AAD followed the header epoch.
        let key = header.unlock(Credential::RecoveryKey, &v.recovery_key).unwrap();
        assert_eq!(key.as_ref(), v.data_key.as_ref());
    }

    #[test]
    fn rotating_the_recovery_key_keeps_the_passphrase_working() {
        let v = new_vault();
        let mut header = v.header;
        header.rotate_recovery_key(&v.data_key, &fast()).unwrap();
        assert!(header.unlock(Credential::Passphrase, "correct horse battery staple").is_ok());
    }

    #[test]
    fn both_credentials_survive_any_sequence_of_changes() {
        let v = new_vault();
        let mut header = v.header;
        for round in 0..3 {
            let passphrase = format!("passphrase number {round} is long enough");
            header.change_passphrase(&v.data_key, &passphrase, &fast()).unwrap();
            header.change_passphrase(&v.data_key, &passphrase, &fast()).unwrap();
            let recovery = header.rotate_recovery_key(&v.data_key, &fast()).unwrap();

            // Survives a save and reload, as a real header does.
            header = VaultHeader::from_json(&header.to_json().unwrap()).unwrap();
            assert!(
                header.unlock(Credential::Passphrase, &passphrase).is_ok(),
                "round {round}"
            );
            assert!(header.unlock(Credential::RecoveryKey, &recovery).is_ok(), "round {round}");
        }
        assert_eq!(header.key_epoch, 10, "every change still advances the epoch");
    }

    #[test]
    fn a_pinned_slot_epoch_is_authenticated() {
        let v = new_vault();
        let mut header = v.header;
        header.change_passphrase(&v.data_key, "a brand new passphrase", &fast()).unwrap();
        assert_eq!(header.recovery_slot.wrapped_at_epoch, Some(1));

        let mut tampered = header.clone();
        tampered.recovery_slot.wrapped_at_epoch = Some(2);
        assert!(tampered.unlock(Credential::RecoveryKey, &v.recovery_key).is_err());
        tampered.recovery_slot.wrapped_at_epoch = None;
        assert!(tampered.unlock(Credential::RecoveryKey, &v.recovery_key).is_err());
    }

    #[test]
    fn headers_written_before_slot_epochs_still_unlock() {
        // A freshly created vault never pins an epoch, so its JSON has no
        // `wrapped_at_epoch` at all — the shape every existing vault has.
        let v = new_vault();
        let json = v.header.to_json().unwrap();
        assert!(!json.contains("wrapped_at_epoch"));
        let reread = VaultHeader::from_json(&json).unwrap();
        assert!(reread.unlock(Credential::Passphrase, "correct horse battery staple").is_ok());
        assert!(reread.unlock(Credential::RecoveryKey, &v.recovery_key).is_ok());
    }

    #[test]
    fn recovery_rotation_invalidates_the_old_key() {
        let v = new_vault();
        let mut header = v.header;

        let new_key = header.rotate_recovery_key(&v.data_key, &fast()).unwrap();

        assert!(
            header.unlock(Credential::RecoveryKey, &v.recovery_key).is_err(),
            "the old recovery key must stop working"
        );
        let key = header.unlock(Credential::RecoveryKey, &new_key).unwrap();
        assert_eq!(key.as_ref(), v.data_key.as_ref());
        assert_eq!(header.recovery_fingerprint, am_crypto::recovery_fingerprint(&new_key));
    }

    #[test]
    fn pairing_detects_mixed_restore() {
        let a = new_vault();
        let b = new_vault();

        assert!(a.header.check_pairing(&a.header.vault_id, a.header.key_epoch).is_ok());

        // Another vault's database.
        assert!(a.header.check_pairing(&b.header.vault_id, b.header.key_epoch).is_err());

        // Same vault, stale epoch: the header was restored from an older
        // backup after a passphrase change.
        let err =
            a.header.check_pairing(&a.header.vault_id, a.header.key_epoch + 1).unwrap_err();
        assert!(err.to_string().contains("partial restore"), "error must explain the cause");
    }

    #[test]
    fn distinct_vaults_get_distinct_ids_and_recovery_keys() {
        let a = new_vault();
        let b = new_vault();
        assert_ne!(a.header.vault_id, b.header.vault_id);
        assert_ne!(a.recovery_key, b.recovery_key);
        assert_ne!(a.data_key.as_ref(), b.data_key.as_ref());
    }

    #[test]
    fn non_ascii_ids_are_an_error_not_a_panic() {
        let v = new_vault();
        let json = v.header.to_json().unwrap();
        let id = hex(&v.header.vault_id);
        // 32 bytes, but with a two-byte character straddling a slice point.
        let hostile = format!("a{}é{}", &id[..15], &id[18..]);
        assert_eq!(hostile.len(), 32);
        assert!(VaultHeader::from_json(&json.replace(&id, &hostile)).is_err());
    }

    #[test]
    fn a_hostile_epoch_cannot_demand_unbounded_retries() {
        let v = new_vault();
        let mut header = v.header.clone();
        header.key_epoch = u64::MAX;
        let started = std::time::Instant::now();
        assert!(header
            .unlock_orphaned_slot(Credential::Passphrase, "correct horse battery staple")
            .is_err());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
    }
}
