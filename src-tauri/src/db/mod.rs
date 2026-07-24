pub mod accounts;
pub mod bodies;
pub mod draft_attachments;
pub mod drafts;
pub mod models;
pub mod queries;

use crate::error::{Result, SkimError};
use rusqlite::Connection;
use std::path::Path;
use std::sync::{Arc, Mutex};

const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_init.sql"),
    include_str!("migrations/0002_invites.sql"),
    include_str!("migrations/0003_draft_attachments.sql"),
    include_str!("migrations/0004_folder_status.sql"),
    include_str!("migrations/0005_unsubscribe.sql"),
    include_str!("migrations/0006_server_drafts.sql"),
    include_str!("migrations/0007_unified_indexes.sql"),
    include_str!("migrations/0008_security.sql"),
];

/// Handle to the single SQLite connection (WAL mode). All access goes through
/// [`Db::call`], which runs the closure on a blocking thread — SQLite calls
/// must never block the async runtime.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;

        assert_fts5(&conn)?;
        migrate(&mut conn, MIGRATIONS)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a closure against the connection on a blocking thread.
    pub async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().expect("db mutex poisoned");
            f(&mut guard)
        })
        .await?;
        Ok(result?)
    }

    /// Synchronous access for tests and non-async contexts.
    pub fn with<T>(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T> {
        let mut guard = self.conn.lock().expect("db mutex poisoned");
        Ok(f(&mut guard)?)
    }
}

fn assert_fts5(conn: &Connection) -> Result<()> {
    let has: bool = conn
        .prepare("SELECT 1 FROM pragma_compile_options WHERE compile_options = 'ENABLE_FTS5'")?
        .exists([])?;
    if !has {
        return Err(SkimError::other(
            "db",
            "bundled SQLite is missing FTS5 support",
        ));
    }
    Ok(())
}

fn migrate(conn: &mut Connection, migrations: &[&str]) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in migrations.iter().enumerate() {
        let target = (i + 1) as i64;
        if version < target {
            // The migration and its version bump commit together: a failing
            // statement rolls the whole step back, so a later start retries it
            // from scratch instead of hitting "table already exists" forever.
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", target)?;
            tx.commit()?;
            tracing::info!(migration = target, "applied database migration");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_cleanly() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            let count: i64 = conn.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN \
                 ('accounts','folders','threads','messages','message_bodies','attachments',\
                  'drafts','pending_ops','remote_image_senders','settings','invite_rsvps')",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(count, 11);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn failed_migration_rolls_back_entirely() {
        let mut conn = Connection::open_in_memory().unwrap();
        let bad = &["CREATE TABLE half_done (x INTEGER); THIS IS NOT SQL;"];
        assert!(migrate(&mut conn, bad).is_err());
        // Neither the early DDL nor the version bump may survive, so the next
        // start retries the migration from scratch instead of wedging on
        // "table already exists".
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 0);
        let leftover: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'half_done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0);
    }
}
