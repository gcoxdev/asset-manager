//! Encrypted storage layer.
//!
//! Currently the Phase 0 SQLCipher gate: this crate exists to prove the
//! database design works before any of it is built on. The assertions in
//! `tests` are the gate — if one fails, the storage plan is wrong, not the
//! test.

pub mod header;
pub mod migrate;
pub mod objects;
pub mod vault;

use rusqlite::Connection;
use zeroize::Zeroizing;

/// Open an encrypted database and configure it.
///
/// Ordering is not negotiable:
/// 1. `PRAGMA key` must be the first statement on the connection — SQLCipher
///    reads page 1 to derive the key, and any prior statement fails.
/// 2. `temp_store = MEMORY` must be set before anything can spill to disk.
///    SQLCipher does not encrypt file-based temporary storage, so a sort or
///    an FTS5 rebuild could otherwise write plaintext fragments outside the
///    vault. This is the single easiest way to silently break the encryption
///    guarantee.
pub fn open_encrypted(path: &str, key_hex: &Zeroizing<String>) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;

    // x'...' passes the key as raw bytes, skipping SQLCipher's own KDF. Our
    // key is already an Argon2id-derived subkey, so deriving again would be
    // pointless work.
    conn.pragma_update(None, "key", format!("x'{}'", key_hex.as_str()))?;

    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    // Fails here, rather than later, if the key is wrong or the file is not a
    // SQLCipher database.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;

    Ok(conn)
}

/// Hex-encode a 32-byte key for `PRAGMA key`.
pub fn key_to_hex(key: &[u8; 32]) -> Zeroizing<String> {
    Zeroizing::new(key.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod gate {
    use super::*;
    use am_crypto::{derive_subkey, Purpose};

    fn test_key() -> Zeroizing<String> {
        let data_key = [0x5au8; 32];
        let subkey = derive_subkey(&data_key, Purpose::Database);
        key_to_hex(&subkey)
    }

    fn other_key() -> Zeroizing<String> {
        let data_key = [0x11u8; 32];
        let subkey = derive_subkey(&data_key, Purpose::Database);
        key_to_hex(&subkey)
    }

    #[test]
    fn sqlcipher_is_actually_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let conn = open_encrypted(path.to_str().unwrap(), &test_key()).unwrap();

        let version: String =
            conn.query_row("PRAGMA cipher_version", [], |r| r.get(0)).expect(
                "PRAGMA cipher_version returned nothing — this is plain SQLite, not SQLCipher",
            );
        assert!(!version.is_empty());
        println!("cipher_version = {version}");
    }

    #[test]
    fn database_file_is_not_readable_as_plain_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let path_str = path.to_str().unwrap();

        {
            let conn = open_encrypted(path_str, &test_key()).unwrap();
            conn.execute("CREATE TABLE secrets (v TEXT)", []).unwrap();
            conn.execute("INSERT INTO secrets VALUES ('SENTINEL-PLAINTEXT-MARKER')", []).unwrap();
            conn.pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
        }

        // The header of an unencrypted SQLite file starts "SQLite format 3".
        let bytes = std::fs::read(path_str).unwrap();
        assert!(
            !bytes.starts_with(b"SQLite format 3"),
            "database is not encrypted: plain SQLite header present"
        );

        // And the sentinel must not appear anywhere on disk.
        assert!(
            !bytes.windows(25).any(|w| w == b"SENTINEL-PLAINTEXT-MARKER"),
            "plaintext row content found in the database file"
        );
    }

    #[test]
    fn wrong_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let path_str = path.to_str().unwrap();

        {
            let conn = open_encrypted(path_str, &test_key()).unwrap();
            conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();
        }

        assert!(
            open_encrypted(path_str, &other_key()).is_err(),
            "a wrong key must fail to open the database"
        );
        // ...and the right one still works afterwards.
        assert!(open_encrypted(path_str, &test_key()).is_ok());
    }

    #[test]
    fn reopen_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let path_str = path.to_str().unwrap();

        {
            let conn = open_encrypted(path_str, &test_key()).unwrap();
            conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();
            conn.execute("INSERT INTO t VALUES ('kept')", []).unwrap();
        }
        {
            let conn = open_encrypted(path_str, &test_key()).unwrap();
            let v: String = conn.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
            assert_eq!(v, "kept");
        }
    }

    #[test]
    fn required_pragmas_hold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let conn = open_encrypted(path.to_str().unwrap(), &test_key()).unwrap();

        // 2 == MEMORY. If this regresses, sorts and FTS5 rebuilds can spill
        // plaintext to disk outside the vault.
        let temp_store: i64 = conn.query_row("PRAGMA temp_store", [], |r| r.get(0)).unwrap();
        assert_eq!(temp_store, 2, "temp_store must be MEMORY");

        let journal: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
    }

    #[test]
    fn json1_is_available() {
        // Per-type attributes live in a JSON column, so this must work.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let conn = open_encrypted(path.to_str().unwrap(), &test_key()).unwrap();

        conn.execute("CREATE TABLE a (attrs TEXT)", []).unwrap();
        conn.execute(r#"INSERT INTO a VALUES ('{"grade":"9.8","grader":"CGC"}')"#, []).unwrap();

        let grade: String = conn
            .query_row("SELECT json_extract(attrs, '$.grade') FROM a", [], |r| r.get(0))
            .expect("JSON1 must be compiled in");
        assert_eq!(grade, "9.8");
    }

    /// The Phase 0 question flagged as the likeliest Phase 1 surprise: does
    /// FTS5 work under SQLCipher with memory-only temp storage, at the scale
    /// a real collection reaches?
    #[test]
    fn fts5_works_at_scale_with_memory_temp_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let conn = open_encrypted(path.to_str().unwrap(), &test_key()).unwrap();

        conn.execute_batch(
            "CREATE TABLE assets (
                 id INTEGER PRIMARY KEY,
                 name TEXT NOT NULL,
                 notes TEXT NOT NULL
             );
             CREATE VIRTUAL TABLE assets_fts USING fts5(
                 name, notes, content='assets', content_rowid='id'
             );",
        )
        .expect("FTS5 must be compiled in");

        let tx = conn.unchecked_transaction().unwrap();
        {
            let mut stmt =
                tx.prepare("INSERT INTO assets (id, name, notes) VALUES (?1, ?2, ?3)").unwrap();
            for i in 1..=5000 {
                stmt.execute((
                    i,
                    format!("1986 Fleer Michael Jordan Rookie #{i}"),
                    format!("PSA 9, purchased lot {i}, stored in box {}", i % 40),
                ))
                .unwrap();
            }
        }
        tx.commit().unwrap();

        // External-content FTS5 needs an explicit rebuild. This is the step
        // that could blow up RAM with temp_store=MEMORY.
        conn.execute("INSERT INTO assets_fts(assets_fts) VALUES('rebuild')", [])
            .expect("FTS5 rebuild failed under temp_store=MEMORY");

        let hits: i64 = conn
            .query_row("SELECT count(*) FROM assets_fts WHERE assets_fts MATCH 'Jordan'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(hits, 5000);

        let scoped: i64 = conn
            .query_row(
                "SELECT count(*) FROM assets_fts WHERE assets_fts MATCH 'notes:box'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(scoped, 5000);

        // 'optimize' is the other operation with a reputation for memory use.
        conn.execute("INSERT INTO assets_fts(assets_fts) VALUES('optimize')", [])
            .expect("FTS5 optimize failed under temp_store=MEMORY");
    }

    /// **Phase 0 finding: SQLite's online backup API does not work on an
    /// encrypted database.** The plan offered it as an alternative to the
    /// pause-checkpoint-copy protocol; it is not available, so that protocol
    /// is the only option.
    ///
    /// This test pins the limitation so a future rusqlite/SQLCipher upgrade
    /// that lifts it shows up as a failure worth revisiting, rather than the
    /// restriction being silently assumed forever.
    #[test]
    fn online_backup_api_is_unavailable_on_encrypted_databases() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let dst = dir.path().join("backup.db");

        let conn = open_encrypted(src.to_str().unwrap(), &test_key()).unwrap();
        conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();

        let err = conn
            .backup(rusqlite::MAIN_DB, &dst, None)
            .expect_err("if this now succeeds, revisit the backup design — see VENDOR/plan notes");
        assert!(
            err.to_string().contains("not supported with encrypted"),
            "unexpected backup failure mode: {err}"
        );
    }

    /// The backup protocol that *does* work: quiesce, checkpoint, close, copy
    /// the file. Verifies the copy stays encrypted and restores intact.
    #[test]
    fn checkpoint_and_copy_backup_roundtrips_and_stays_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let dst = dir.path().join("backup.db");
        let src_str = src.to_str().unwrap();

        {
            let conn = open_encrypted(src_str, &test_key()).unwrap();
            conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();
            conn.execute("INSERT INTO t VALUES ('SENTINEL-BACKUP-MARKER')", []).unwrap();

            // Fold the WAL back into the main file. Without this, a file copy
            // can miss committed transactions still living in the WAL.
            conn.pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
        } // connection closed: no writer, no live WAL

        std::fs::copy(&src, &dst).unwrap();

        let bytes = std::fs::read(&dst).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3"), "backup is not encrypted");
        assert!(
            !bytes.windows(22).any(|w| w == b"SENTINEL-BACKUP-MARKER"),
            "plaintext content found in the backup file"
        );

        let restored = open_encrypted(dst.to_str().unwrap(), &test_key()).unwrap();
        let v: String = restored.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "SENTINEL-BACKUP-MARKER");
    }

    /// Guards the claim that a checkpoint is actually needed: data written and
    /// committed must survive a copy taken after checkpoint + close.
    #[test]
    fn committed_data_survives_checkpoint_copy() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let dst = dir.path().join("copy.db");
        let src_str = src.to_str().unwrap();

        {
            let conn = open_encrypted(src_str, &test_key()).unwrap();
            conn.execute("CREATE TABLE t (i INTEGER)", []).unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            for i in 0..500 {
                tx.execute("INSERT INTO t VALUES (?1)", [i]).unwrap();
            }
            tx.commit().unwrap();
            conn.pragma_update(None, "wal_checkpoint", "TRUNCATE").unwrap();
        }

        std::fs::copy(&src, &dst).unwrap();

        let restored = open_encrypted(dst.to_str().unwrap(), &test_key()).unwrap();
        let count: i64 = restored.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 500, "committed rows lost across checkpoint+copy");
    }
}
