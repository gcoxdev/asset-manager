//! Units of work that nest.
//!
//! Storage functions each keep their own writes all-or-nothing. A desktop
//! command often calls several of them — create an asset, then record its
//! opening value — and needs the *whole* command to be all-or-nothing too:
//! otherwise a failure in the second step leaves the first committed, the UI
//! reports an error, and a retry creates a duplicate.
//!
//! SQLite cannot nest `BEGIN`, but it can nest savepoints, and a savepoint
//! opened outside any transaction starts one. So every unit of work here is a
//! savepoint: on its own it behaves exactly like a transaction, and inside an
//! outer one it commits only into that outer one.

use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::Connection;

static NEXT: AtomicU64 = AtomicU64::new(1);

/// An open unit of work. Rolls back on drop unless committed.
///
/// Dereferences to the connection, so code written against a
/// `rusqlite::Transaction` reads the same.
pub struct Atomic<'c> {
    conn: &'c Connection,
    name: String,
    open: bool,
}

/// Begin a unit of work on `conn`, nested inside any already open.
pub fn begin(conn: &Connection) -> rusqlite::Result<Atomic<'_>> {
    let name = format!("am_unit_{}", NEXT.fetch_add(1, Ordering::Relaxed));
    conn.execute_batch(&format!("SAVEPOINT {name}"))?;
    Ok(Atomic { conn, name, open: true })
}

impl Atomic<'_> {
    /// Keep the writes: durable if this is the outermost unit, otherwise part
    /// of the enclosing one.
    pub fn commit(mut self) -> rusqlite::Result<()> {
        self.conn.execute_batch(&format!("RELEASE {}", self.name))?;
        self.open = false;
        Ok(())
    }
}

impl Drop for Atomic<'_> {
    fn drop(&mut self) {
        if self.open {
            // Undo, then close the savepoint. Errors are ignored: SQLite may
            // already have rolled the transaction back itself (a full disk,
            // say), which leaves nothing to undo.
            let _ = self.conn.execute_batch(&format!(
                "ROLLBACK TO {name}; RELEASE {name}",
                name = self.name
            ));
        }
    }
}

impl Deref for Atomic<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t (v INTEGER)").unwrap();
        c
    }

    fn count(c: &Connection) -> i64 {
        c.query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_unit_alone_commits_or_rolls_back_like_a_transaction() {
        let c = conn();
        let unit = begin(&c).unwrap();
        unit.execute("INSERT INTO t VALUES (1)", []).unwrap();
        unit.commit().unwrap();
        assert_eq!(count(&c), 1);
        assert!(c.is_autocommit(), "nothing left open");

        let unit = begin(&c).unwrap();
        unit.execute("INSERT INTO t VALUES (2)", []).unwrap();
        drop(unit);
        assert_eq!(count(&c), 1);
        assert!(c.is_autocommit());
    }

    #[test]
    fn an_inner_commit_is_undone_when_the_outer_unit_fails() {
        let c = conn();
        let outer = begin(&c).unwrap();
        let inner = begin(&outer).unwrap();
        inner.execute("INSERT INTO t VALUES (1)", []).unwrap();
        inner.commit().unwrap();
        assert!(!c.is_autocommit(), "still inside the outer unit");
        drop(outer);
        assert_eq!(count(&c), 0, "the whole command is all-or-nothing");
    }

    #[test]
    fn an_inner_failure_leaves_the_outer_unit_usable() {
        let c = conn();
        let outer = begin(&c).unwrap();
        outer.execute("INSERT INTO t VALUES (1)", []).unwrap();
        {
            let inner = begin(&outer).unwrap();
            inner.execute("INSERT INTO t VALUES (2)", []).unwrap();
        }
        outer.commit().unwrap();
        assert_eq!(count(&c), 1);
    }
}
