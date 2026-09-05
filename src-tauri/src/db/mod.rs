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
    include_str!("migrations/0009_backfill.sql"),
    include_str!("migrations/0010_folder_delimiter.sql"),
    include_str!("migrations/0011_translations.sql"),
    include_str!("migrations/0012_translated_subject.sql"),
    include_str!("migrations/0013_account_signature.sql"),
];

/// A read the user is waiting on has this long to answer before it is worth
/// knowing about. Well past a healthy query; short enough to catch a stall.
const SLOW_READ: std::time::Duration = std::time::Duration::from_secs(1);

/// Handle to the database (WAL mode), with two connections behind it.
///
/// Everything that writes shares one connection, serialized by a mutex —
/// [`Db::call`] and [`Db::with`]. Reads the user is waiting on take the
/// separate read-only connection instead ([`Db::read`]): WAL lets a reader run
/// alongside the writer, so opening the app no longer means queuing the first
/// query behind whatever the startup sync is pouring into the same lock.
/// Both run their closure on a blocking thread — SQLite calls must never block
/// the async runtime.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    reader: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        let db = Self::init(conn)?;
        // Only now: the reader must not race the migrations, and it inherits
        // WAL from the file the writer has already set it on.
        let reader = Connection::open(path)?;
        Self::prepare(&reader)?;
        reader.pragma_update(None, "query_only", "ON")?;
        Ok(Self {
            reader: Arc::new(Mutex::new(reader)),
            ..db
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        // A second connection to `:memory:` would be a second, empty database,
        // so the in-memory handle reads through the writer. Tests don't
        // contend, and this keeps them honest about what they wrote.
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::prepare(&conn)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        assert_fts5(&conn)?;
        migrate(&mut conn, MIGRATIONS)?;

        let conn = Arc::new(Mutex::new(conn));
        Ok(Self {
            reader: conn.clone(),
            conn,
        })
    }

    /// The settings every connection to the database needs.
    fn prepare(conn: &Connection) -> Result<()> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(())
    }

    /// Run a closure against the writing connection on a blocking thread.
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

    /// Run a read-only closure on the reader connection — the path for queries
    /// the user is waiting on. `label` names the query in `skim-slow.log` if it
    /// takes long enough to be felt; a windowed release build has no other way
    /// to say so, and not being able to see this is what once made a slow start
    /// indistinguishable from a broken app.
    pub async fn read<T, F>(&self, label: &'static str, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = self.reader.clone();
        let result = tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let mut guard = conn.lock().expect("db mutex poisoned");
            let waited = started.elapsed();
            let out = f(&mut guard);
            let total = started.elapsed();
            if total >= SLOW_READ {
                crate::append_log(
                    "skim-slow.log",
                    &format!(
                        "slow db read: {label} took {}ms ({}ms waiting for the connection)",
                        total.as_millis(),
                        waited.as_millis()
                    ),
                );
            }
            out
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
                  'drafts','pending_ops','remote_image_senders','settings','invite_rsvps',\
                  'message_translations')",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(count, 12);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reader_is_the_same_database_and_cannot_write() {
        // Only the on-disk handle has a real second connection — the in-memory
        // one reads through the writer, since a second `:memory:` connection
        // would be a second, empty database. So this needs a file.
        let path = std::env::temp_dir().join(format!("skim-reader-{}.db", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        let db = Db::open(&path).unwrap();
        assert!(
            !Arc::ptr_eq(&db.conn, &db.reader),
            "expected two connections"
        );

        db.with(|conn| queries::set_setting(conn, "locale", "sr"))
            .unwrap();

        let reader = db.reader.lock().unwrap();
        let seen: String = reader
            .query_row("SELECT value FROM settings WHERE key = 'locale'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(seen, "sr", "the reader must see the writer's commits");
        assert!(
            reader
                .execute("UPDATE settings SET value = 'en' WHERE key = 'locale'", [])
                .is_err(),
            "query_only must keep writes off the reader"
        );

        drop(reader);
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// The exact statement `prepare_update` runs before the updater hands over
    /// to the installer. It has one job — leave nothing in the journal for the
    /// next process to recover — and no way to report failure at the time, so
    /// pin it here instead.
    #[test]
    fn wal_checkpoint_truncate_empties_the_journal() {
        let path = std::env::temp_dir().join(format!("skim-ckpt-{}.db", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        let wal = std::path::PathBuf::from(format!("{}-wal", path.display()));
        let db = Db::open(&path).unwrap();

        db.with(|conn| {
            for i in 0..500 {
                queries::set_setting(conn, &format!("k{i}"), "some value worth a page or two")?;
            }
            Ok(())
        })
        .unwrap();
        assert!(
            std::fs::metadata(&wal).is_ok_and(|m| m.len() > 0),
            "the writes should still be sitting in the WAL"
        );

        db.with(|conn| conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())))
            .unwrap();
        assert_eq!(
            std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0),
            0,
            "the WAL must be empty once it is folded back in"
        );
        // And the data survived the fold.
        let seen: Option<String> = db.with(|conn| queries::get_setting(conn, "k499")).unwrap();
        assert_eq!(seen.as_deref(), Some("some value worth a page or two"));

        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn in_memory_reads_through_the_writer() {
        let db = Db::open_in_memory().unwrap();
        assert!(Arc::ptr_eq(&db.conn, &db.reader));
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
