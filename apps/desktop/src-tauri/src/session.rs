//! Vault session state and lock enforcement.
//!
//! **Locking is a backend state transition, not a UI state.** When locked,
//! every command returning asset data must reject — hiding a screen is not an
//! access control, because the IPC surface is reachable regardless of what the
//! UI is showing.
//!
//! The rule this module exists to enforce: a command that touches vault data
//! must go through [`Session::with_vault`], which fails when locked. There is
//! no way to read the vault around it.

use std::sync::Mutex;

use am_storage::header::Credential;
use am_storage::vault::{Vault, VaultError};
use am_crypto::KdfParams;

/// Argon2id cost for real vaults. Tests override this; see `fast_params`.
pub fn default_params() -> KdfParams {
    KdfParams::default() // 64 MiB, 3 iterations, parallelism 1
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("the vault is locked")]
    Locked,
    #[error("a vault is already open")]
    AlreadyOpen,
    #[error(transparent)]
    Vault(#[from] VaultError),
}

/// Serializable error for the frontend.
///
/// Deliberately coarse: the UI learns *that* an unlock failed, never whether
/// the passphrase was wrong versus the header tampered with.
#[derive(Debug, serde::Serialize)]
pub struct IpcError {
    pub kind: String,
    pub message: String,
}

impl From<SessionError> for IpcError {
    fn from(e: SessionError) -> Self {
        let kind = match &e {
            SessionError::Locked => "locked",
            SessionError::AlreadyOpen => "already_open",
            SessionError::Vault(VaultError::NotFound(_)) => "not_found",
            SessionError::Vault(VaultError::AlreadyExists(_)) => "already_exists",
            SessionError::Vault(VaultError::Unlock(_)) => "cannot_unlock",
            SessionError::Vault(VaultError::Pairing(_)) => "pairing_mismatch",
            SessionError::Vault(VaultError::Locked) => "another_instance",
            _ => "error",
        };
        IpcError { kind: kind.to_string(), message: e.to_string() }
    }
}

#[derive(Default)]
pub struct Session {
    vault: Mutex<Option<Vault>>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_unlocked(&self) -> bool {
        self.vault.lock().expect("session mutex poisoned").is_some()
    }

    pub fn create(
        &self,
        root: &std::path::Path,
        passphrase: &str,
        params: &KdfParams,
        now: &str,
    ) -> Result<String, SessionError> {
        let mut guard = self.vault.lock().expect("session mutex poisoned");
        if guard.is_some() {
            return Err(SessionError::AlreadyOpen);
        }
        let (vault, recovery_key) = Vault::create(root, passphrase, params, now)?;
        *guard = Some(vault);
        Ok(recovery_key)
    }

    pub fn unlock(
        &self,
        root: &std::path::Path,
        credential: Credential,
        secret: &str,
    ) -> Result<(), SessionError> {
        let mut guard = self.vault.lock().expect("session mutex poisoned");
        if guard.is_some() {
            return Err(SessionError::AlreadyOpen);
        }
        *guard = Some(Vault::unlock(root, credential, secret)?);
        Ok(())
    }

    /// Lock: drop the vault, which closes the database, releases the process
    /// lock, and zeroizes the data key.
    ///
    /// In-flight work is not a concern yet because every command here is
    /// synchronous and holds the mutex for its duration — a command either
    /// completes before the lock or cannot start after it. When async work
    /// lands (thumbnailing, price fetches), this must also cancel or drain it
    /// so late results cannot repopulate the UI.
    pub fn lock(&self) {
        let mut guard = self.vault.lock().expect("session mutex poisoned");
        *guard = None;
    }

    /// The only path to vault data. Fails when locked.
    pub fn with_vault<T>(
        &self,
        f: impl FnOnce(&Vault) -> Result<T, SessionError>,
    ) -> Result<T, SessionError> {
        let guard = self.vault.lock().expect("session mutex poisoned");
        let vault = guard.as_ref().ok_or(SessionError::Locked)?;
        f(vault)
    }

    pub fn with_vault_mut<T>(
        &self,
        f: impl FnOnce(&mut Vault) -> Result<T, SessionError>,
    ) -> Result<T, SessionError> {
        let mut guard = self.vault.lock().expect("session mutex poisoned");
        let vault = guard.as_mut().ok_or(SessionError::Locked)?;
        f(vault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_params() -> KdfParams {
        KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 }
    }

    const NOW: &str = "2026-09-19T00:00:00Z";
    const PASS: &str = "correct horse battery staple";

    #[test]
    fn starts_locked() {
        let s = Session::new();
        assert!(!s.is_unlocked());
        assert!(matches!(s.with_vault(|_| Ok(())), Err(SessionError::Locked)));
    }

    #[test]
    fn create_unlock_lock_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");

        let s = Session::new();
        let recovery = s.create(&root, PASS, &fast_params(), NOW).unwrap();
        assert!(s.is_unlocked());
        assert!(!recovery.is_empty());

        s.lock();
        assert!(!s.is_unlocked());

        s.unlock(&root, Credential::Passphrase, PASS).unwrap();
        assert!(s.is_unlocked());
    }

    #[test]
    fn data_access_fails_while_locked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");

        let s = Session::new();
        s.create(&root, PASS, &fast_params(), NOW).unwrap();

        // Readable while unlocked.
        s.with_vault(|v| {
            v.conn()
                .query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
                .map_err(|e| SessionError::Vault(VaultError::Sqlite(e)))
        })
        .unwrap();

        s.lock();

        // ...and not afterwards.
        let err = s
            .with_vault(|v| {
                v.conn()
                    .query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
                    .map_err(|e| SessionError::Vault(VaultError::Sqlite(e)))
            })
            .unwrap_err();
        assert!(matches!(err, SessionError::Locked));
    }

    #[test]
    fn lock_releases_the_process_lock() {
        // Locking must close the vault fully, or reopening would hit the
        // single-instance guard.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");

        let s = Session::new();
        s.create(&root, PASS, &fast_params(), NOW).unwrap();
        s.lock();

        let other = Session::new();
        other.unlock(&root, Credential::Passphrase, PASS).unwrap();
    }

    #[test]
    fn cannot_open_two_vaults_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");

        let s = Session::new();
        s.create(&a, PASS, &fast_params(), NOW).unwrap();
        assert!(matches!(s.create(&b, PASS, &fast_params(), NOW), Err(SessionError::AlreadyOpen)));
    }

    #[test]
    fn unlock_errors_do_not_distinguish_cause() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let s = Session::new();
        s.create(&root, PASS, &fast_params(), NOW).unwrap();
        s.lock();

        let err: IpcError = s
            .unlock(&root, Credential::Passphrase, "wrong passphrase")
            .unwrap_err()
            .into();
        assert_eq!(err.kind, "cannot_unlock");
        // The message must not hint at which half failed.
        assert!(!err.message.to_lowercase().contains("passphrase is"));
    }
}
