//! Per-vault settings.
//!
//! Kept in the vault rather than a config file so a setting travels with the
//! data it governs, and a privacy choice cannot be flipped by editing a
//! dotfile. The trade-off is that settings are unreadable while locked —
//! which is also true of everything they would configure.

use std::collections::BTreeMap;

use crate::vault::Vault;

pub fn get(vault: &Vault, key: &str) -> rusqlite::Result<Option<String>> {
    match vault
        .conn()
        .query_row("SELECT value FROM app_settings WHERE key = ?1", [key], |r| r.get(0))
    {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn set(vault: &Vault, key: &str, value: &str) -> rusqlite::Result<()> {
    vault.conn().execute(
        "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

pub fn all(vault: &Vault) -> rusqlite::Result<BTreeMap<String, String>> {
    let mut stmt = vault.conn().prepare("SELECT key, value FROM app_settings")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_crypto::KdfParams;

    #[test]
    fn settings_round_trip_and_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let fast = KdfParams { memory_cost_kib: 8 * 1024, iterations: 1, parallelism: 1 };
        let (v, _r) = Vault::create(
            &dir.path().join("v"),
            "correct horse battery staple",
            &fast,
            "2026-09-19T00:00:00Z",
        )
        .unwrap();

        assert_eq!(get(&v, "currency").unwrap(), None);
        set(&v, "currency", "USD").unwrap();
        set(&v, "currency", "EUR").unwrap();
        assert_eq!(get(&v, "currency").unwrap().as_deref(), Some("EUR"));
        assert_eq!(all(&v).unwrap().len(), 1);
    }
}
