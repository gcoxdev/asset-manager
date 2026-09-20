//! Schema migrations, embedded in the binary.
//!
//! A failed migration must leave a recoverable vault: every migration runs
//! inside a transaction, so a failure rolls back rather than leaving a
//! half-migrated database. A database newer than this build is refused
//! outright rather than partially opened.

use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 2;

struct Migration {
    version: i64,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, sql: include_str!("../migrations/001_initial.sql") },
    Migration { version: 2, sql: include_str!("../migrations/002_app_settings.sql") },
];

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error(
        "this vault uses schema version {found}, newer than this build supports \
         ({supported}) — upgrade Asset Manager to open it"
    )]
    TooNew { found: i64, supported: i64 },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

/// Bring a database up to [`SCHEMA_VERSION`]. Idempotent.
pub fn migrate(conn: &Connection) -> Result<(), MigrateError> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    if current > SCHEMA_VERSION {
        return Err(MigrateError::TooNew { found: current, supported: SCHEMA_VERSION });
    }

    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(m.sql)?;
        // user_version doesn't accept a bound parameter.
        tx.execute_batch(&format!("PRAGMA user_version = {}", m.version))?;
        tx.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{key_to_hex, open_encrypted};
    use am_crypto::{derive_subkey, Purpose};
    use zeroize::Zeroizing;

    fn key() -> Zeroizing<String> {
        key_to_hex(&derive_subkey(&[0x5au8; 32], Purpose::Database))
    }

    fn fresh() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let conn = open_encrypted(path.to_str().unwrap(), &key()).unwrap();
        migrate(&conn).unwrap();
        (dir, conn)
    }

    #[test]
    fn migration_creates_schema_and_sets_version() {
        let (_d, conn) = fresh();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);

        for table in [
            "vault_meta",
            "objects",
            "assets",
            "asset_events",
            "asset_media",
            "media_variants",
            "quotes",
            "valuations",
            "provider_quota",
            "app_settings",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing table {table}");
        }
    }

    #[test]
    fn migration_is_idempotent() {
        let (_d, conn) = fresh();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let n: i64 =
            conn.query_row("SELECT count(*) FROM asset_types", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 6, "seed rows must not be duplicated");
    }

    #[test]
    fn an_existing_vault_migrates_forward() {
        // The case a versioned schema exists for: a vault created at v1 must
        // open at v2 without losing what it already holds.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let conn = open_encrypted(path.to_str().unwrap(), &key()).unwrap();

        // Apply only the first migration, as an older build would have.
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute_batch(MIGRATIONS[0].sql).unwrap();
        tx.execute_batch("PRAGMA user_version = 1").unwrap();
        tx.commit().unwrap();

        conn.execute(
            "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
             VALUES ('a1','generic','Pre-existing','2026-01-01','2026-01-01')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap();

        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 2);

        let name: String = conn
            .query_row("SELECT name FROM assets WHERE asset_id='a1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Pre-existing", "existing data must survive the migration");

        conn.execute("INSERT INTO app_settings (key, value) VALUES ('k','v')", []).unwrap();
    }

    #[test]
    fn refuses_a_newer_schema() {
        let (_d, conn) = fresh();
        conn.execute_batch("PRAGMA user_version = 999").unwrap();

        let err = migrate(&conn).unwrap_err();
        assert!(matches!(err, MigrateError::TooNew { .. }), "got {err:?}");
        // The message must tell the user what to do.
        assert!(err.to_string().contains("upgrade Asset Manager"));
    }

    #[test]
    fn money_requires_a_currency() {
        let (_d, conn) = fresh();
        conn.execute(
            "INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
             VALUES ('a1', 'generic', 'Test', '2026-09-19', '2026-09-19')",
            [],
        )
        .unwrap();

        // An amount without its currency must be rejected by the schema.
        let bad = conn
            .execute("UPDATE assets SET acquired_amount_minor = 1000 WHERE asset_id='a1'", []);
        assert!(bad.is_err(), "amount without currency must violate the CHECK");

        conn.execute(
            "UPDATE assets SET acquired_amount_minor = 1000, acquired_currency = 'USD'
             WHERE asset_id='a1'",
            [],
        )
        .unwrap();
    }

    #[test]
    fn attrs_must_be_valid_json() {
        let (_d, conn) = fresh();
        let bad = conn.execute(
            "INSERT INTO assets (asset_id, type_id, name, attrs, created_at, updated_at)
             VALUES ('a1', 'generic', 'Test', 'not json', '2026-09-19', '2026-09-19')",
            [],
        );
        assert!(bad.is_err());
    }

    #[test]
    fn fts_tracks_asset_changes() {
        let (_d, conn) = fresh();
        conn.execute(
            "INSERT INTO assets (asset_id, type_id, name, notes, storage_location, created_at, updated_at)
             VALUES ('a1', 'generic', '1986 Fleer Jordan', 'PSA 9', 'Safe deposit box 12',
                     '2026-09-19', '2026-09-19')",
            [],
        )
        .unwrap();

        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM assets_fts WHERE assets_fts MATCH 'Jordan'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 1, "insert trigger did not index the row");

        // Storage location must be searchable too.
        let loc: i64 = conn
            .query_row(
                "SELECT count(*) FROM assets_fts WHERE assets_fts MATCH 'storage_location:deposit'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(loc, 1);

        conn.execute("UPDATE assets SET name = 'Renamed Card' WHERE asset_id='a1'", [])
            .unwrap();
        let stale: i64 = conn
            .query_row(
                "SELECT count(*) FROM assets_fts WHERE assets_fts MATCH 'Jordan'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stale, 0, "update trigger left a stale index entry");

        conn.execute("DELETE FROM assets WHERE asset_id='a1'", []).unwrap();
        let after: i64 = conn
            .query_row(
                "SELECT count(*) FROM assets_fts WHERE assets_fts MATCH 'Renamed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, 0, "delete trigger left a stale index entry");
    }

    #[test]
    fn shared_object_survives_one_asset_being_deleted() {
        // The case that motivated splitting objects from asset_media: two
        // assets referencing one photo. Deleting one must not remove the bytes.
        let (_d, conn) = fresh();
        conn.execute_batch(
            "INSERT INTO objects (object_id, plaintext_sha256, ciphertext_bytes, media_type, refcount, created_at)
             VALUES ('obj1', 'hash1', 1000, 'image/jpeg', 2, '2026-09-19');
             INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
             VALUES ('a1', 'generic', 'One', '2026-09-19', '2026-09-19'),
                    ('a2', 'generic', 'Two', '2026-09-19', '2026-09-19');
             INSERT INTO asset_media (asset_id, object_id, created_at)
             VALUES ('a1', 'obj1', '2026-09-19'), ('a2', 'obj1', '2026-09-19');",
        )
        .unwrap();

        conn.execute("DELETE FROM assets WHERE asset_id='a1'", []).unwrap();

        let obj: i64 = conn
            .query_row("SELECT count(*) FROM objects WHERE object_id='obj1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(obj, 1, "shared object must survive");

        let still_linked: i64 = conn
            .query_row("SELECT count(*) FROM asset_media WHERE object_id='obj1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(still_linked, 1, "the other asset must keep its link");
    }

    #[test]
    fn only_one_primary_photo_per_asset() {
        let (_d, conn) = fresh();
        conn.execute_batch(
            "INSERT INTO objects (object_id, plaintext_sha256, ciphertext_bytes, media_type, created_at)
             VALUES ('o1','h1',1,'image/jpeg','2026-09-19'), ('o2','h2',1,'image/jpeg','2026-09-19');
             INSERT INTO assets (asset_id, type_id, name, created_at, updated_at)
             VALUES ('a1','generic','One','2026-09-19','2026-09-19');
             INSERT INTO asset_media (asset_id, object_id, is_primary, created_at)
             VALUES ('a1','o1',1,'2026-09-19');",
        )
        .unwrap();

        let second = conn.execute(
            "INSERT INTO asset_media (asset_id, object_id, is_primary, created_at)
             VALUES ('a1','o2',1,'2026-09-19')",
            [],
        );
        assert!(second.is_err(), "a second primary photo must be rejected");
    }

    #[test]
    fn dedup_index_rejects_duplicate_hashes() {
        let (_d, conn) = fresh();
        conn.execute(
            "INSERT INTO objects (object_id, plaintext_sha256, ciphertext_bytes, media_type, created_at)
             VALUES ('o1','samehash',1,'image/jpeg','2026-09-19')",
            [],
        )
        .unwrap();

        let dup = conn.execute(
            "INSERT INTO objects (object_id, plaintext_sha256, ciphertext_bytes, media_type, created_at)
             VALUES ('o2','samehash',1,'image/jpeg','2026-09-19')",
            [],
        );
        assert!(dup.is_err(), "the same plaintext must map to one object");
    }

    #[test]
    fn quantity_sort_key_orders_numerically() {
        // The reason quantity_sort exists: TEXT would order "10" before "9".
        let (_d, conn) = fresh();
        conn.execute_batch(
            "INSERT INTO assets (asset_id, type_id, name, quantity, quantity_sort, created_at, updated_at)
             VALUES ('a','generic','A','9','9.0','2026-09-19','2026-09-19'),
                    ('b','generic','B','10','10.0','2026-09-19','2026-09-19'),
                    ('c','generic','C','100','100.0','2026-09-19','2026-09-19');",
        )
        .unwrap();

        let by_text: Vec<String> = conn
            .prepare("SELECT asset_id FROM assets ORDER BY quantity")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(by_text, ["b", "c", "a"], "TEXT ordering is lexicographic, as expected");

        let by_sort: Vec<String> = conn
            .prepare("SELECT asset_id FROM assets ORDER BY quantity_sort")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(by_sort, ["a", "b", "c"], "sort key must order numerically");
    }
}
