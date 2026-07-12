//! Per-account sync engine.
//!
//! Phase 3 scope: folder discovery, initial header sync (newest-first
//! windows), incremental sync of new mail, manual refresh. One IMAP session,
//! commands serialized through an mpsc queue. IDLE and the offline op queue
//! arrive in phase 4.

use crate::db::models::{Account, NewMessage};
use crate::db::{queries, Db};
use crate::error::{Result, SkimError};
use crate::mail::{imap_client, oauth, parse};
use crate::secrets;
use futures::StreamExt;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

const INBOX_WINDOW: u32 = 500;
const FOLDER_WINDOW: u32 = 200;
const CHUNK: u32 = 100;

#[derive(Debug)]
pub enum SyncCommand {
    SyncAll,
    Stop,
}

#[derive(Clone)]
pub struct SyncHandle {
    pub tx: mpsc::UnboundedSender<SyncCommand>,
}

impl SyncHandle {
    pub fn sync_all(&self) {
        let _ = self.tx.send(SyncCommand::SyncAll);
    }
    pub fn stop(&self) {
        let _ = self.tx.send(SyncCommand::Stop);
    }
}

struct Engine {
    app: AppHandle,
    db: Db,
    account: Account,
    session: Option<imap_client::Session>,
    /// Cached OAuth access token and its expiry (unix seconds).
    oauth_token: Option<(String, i64)>,
}

pub fn spawn(app: AppHandle, db: Db, account: Account) -> SyncHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<SyncCommand>();
    let handle = SyncHandle { tx };

    tauri::async_runtime::spawn(async move {
        let mut engine = Engine {
            app,
            db,
            account,
            session: None,
            oauth_token: None,
        };
        // Initial sync on startup.
        engine.run_sync().await;

        while let Some(cmd) = rx.recv().await {
            match cmd {
                SyncCommand::SyncAll => engine.run_sync().await,
                SyncCommand::Stop => break,
            }
        }
        engine.logout().await;
    });

    handle
}

impl Engine {
    fn emit_status(&self, state: &str, message: Option<String>) {
        let _ = self.app.emit(
            "sync:status",
            json!({ "accountId": self.account.id, "state": state, "message": message }),
        );
    }

    async fn run_sync(&mut self) {
        self.emit_status("syncing", None);
        match self.sync_all_folders().await {
            Ok(()) => self.emit_status("idle", None),
            Err(e) => {
                tracing::warn!(error = %e, "sync failed");
                // Drop the session so the next attempt reconnects cleanly.
                self.session = None;
                self.emit_status("error", Some(e.to_string()));
            }
        }
    }

    async fn logout(&mut self) {
        if let Some(mut s) = self.session.take() {
            let _ = s.logout().await;
        }
    }

    async fn credentials(&mut self) -> Result<imap_client::Credentials> {
        let secret = secrets::get(&secrets::mail_key(&self.account.id))?
            .ok_or_else(|| SkimError::other("auth", "no stored credentials for this account"))?;
        if self.account.auth_kind == "oauth" {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if let Some((token, expires_at)) = &self.oauth_token {
                if *expires_at > now {
                    return Ok(imap_client::Credentials::OauthToken(token.clone()));
                }
            }
            let config = oauth_config(&self.db).await?.ok_or_else(|| {
                SkimError::other("oauth", "Google OAuth client id is not configured")
            })?;
            let (token, expires_at) = oauth::refresh_access_token(&config, &secret).await?;
            self.oauth_token = Some((token.clone(), expires_at));
            Ok(imap_client::Credentials::OauthToken(token))
        } else {
            Ok(imap_client::Credentials::Password(secret))
        }
    }

    async fn session(&mut self) -> Result<&mut imap_client::Session> {
        if self.session.is_none() {
            let creds = self.credentials().await?;
            let session = imap_client::login(
                &self.account.imap_host,
                self.account.imap_port,
                &self.account.email,
                &creds,
            )
            .await?;
            self.session = Some(session);
        }
        Ok(self.session.as_mut().expect("just set"))
    }

    async fn sync_all_folders(&mut self) -> Result<()> {
        self.discover_folders().await?;

        let account_id = self.account.id.clone();
        let folders = self
            .db
            .call(move |conn| queries::list_folders(conn, &account_id))
            .await?;

        let mut any_changes = false;
        for folder in folders {
            if folder.role.as_deref() == Some("all") {
                continue;
            }
            match self.sync_folder(folder.id, &folder.imap_name).await {
                Ok(changed) => any_changes |= changed,
                Err(e) => {
                    tracing::warn!(folder = %folder.imap_name, error = %e, "folder sync failed");
                    // Auth/network errors abort the whole run; per-folder
                    // oddities (e.g. \Noselect) are skipped.
                    match e.code() {
                        "auth" | "network" | "tls" | "oauth" | "oauth_expired" => return Err(e),
                        _ => continue,
                    }
                }
            }
        }
        if any_changes {
            let _ = self.app.emit("mail:updated", json!({}));
        }
        Ok(())
    }

    async fn discover_folders(&mut self) -> Result<()> {
        let session = self.session().await?;
        let mut names = Vec::new();
        {
            let mut stream = session.list(None, Some("*")).await.map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                let name = item.map_err(imap_err)?;
                let attrs: Vec<String> =
                    name.attributes().iter().map(|a| format!("{a:?}")).collect();
                names.push((name.name().to_string(), attrs));
            }
        }

        let account_id = self.account.id.clone();
        let provider = self.account.provider.clone();
        self.db
            .call(move |conn| {
                let tx = conn.transaction()?;
                for (imap_name, attrs) in &names {
                    let attrs_joined = attrs.join(" ").to_lowercase();
                    if attrs_joined.contains("noselect") {
                        continue;
                    }
                    let role = detect_role(imap_name, &attrs_joined);
                    let display_name = display_name(imap_name, &provider);
                    let sort_order = match role.as_deref() {
                        Some("inbox") => 0,
                        Some("sent") => 10,
                        Some("drafts") => 20,
                        Some("archive") => 30,
                        Some("trash") => 40,
                        Some("junk") => 50,
                        Some("all") => 60,
                        _ => 100,
                    };
                    tx.execute(
                        "INSERT INTO folders (account_id, imap_name, role, display_name, sort_order)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(account_id, imap_name)
                         DO UPDATE SET role = excluded.role, display_name = excluded.display_name,
                                       sort_order = excluded.sort_order",
                        rusqlite::params![account_id, imap_name, role, display_name, sort_order],
                    )?;
                }
                tx.commit()
            })
            .await?;

        let _ = self.app.emit("folders:updated", json!({}));
        Ok(())
    }

    /// Sync one folder. Returns whether anything changed.
    async fn sync_folder(&mut self, folder_id: i64, imap_name: &str) -> Result<bool> {
        let is_inbox = imap_name.eq_ignore_ascii_case("INBOX");
        let session = self.session().await?;
        let mailbox = session
            .select(imap_name)
            .await
            .map_err(|e| SkimError::other("folder", format!("cannot open {imap_name}: {e}")))?;

        let uidvalidity = mailbox.uid_validity.unwrap_or(0) as i64;
        let exists = mailbox.exists;

        // UIDVALIDITY change → local cache for this folder is void.
        let db = self.db.clone();
        let stored: (Option<i64>, i64) = db
            .call(move |conn| {
                conn.query_row(
                    "SELECT uidvalidity, last_seen_uid FROM folders WHERE id = ?1",
                    rusqlite::params![folder_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .await?;
        let (stored_validity, mut last_seen_uid) = stored;

        if stored_validity != Some(uidvalidity) {
            if stored_validity.is_some() {
                tracing::info!(folder = imap_name, "UIDVALIDITY changed; resyncing folder");
                wipe_folder(&db, folder_id).await?;
            }
            last_seen_uid = 0;
            let dbc = db.clone();
            dbc.call(move |conn| {
                conn.execute(
                    "UPDATE folders SET uidvalidity = ?2, last_seen_uid = 0 WHERE id = ?1",
                    rusqlite::params![folder_id, uidvalidity],
                )
                .map(|_| ())
            })
            .await?;
        }

        let mut changed = false;

        if last_seen_uid == 0 {
            // Fresh folder: newest window of headers, top-down.
            if exists > 0 {
                let window = if is_inbox {
                    INBOX_WINDOW
                } else {
                    FOLDER_WINDOW
                };
                let start = exists.saturating_sub(window.saturating_sub(1)).max(1);
                let mut high = exists;
                while high >= start {
                    let low = high.saturating_sub(CHUNK - 1).max(start);
                    let n = self
                        .fetch_headers_seq(folder_id, &format!("{low}:{high}"))
                        .await?;
                    changed |= n > 0;
                    let _ = self.app.emit(
                        "sync:progress",
                        json!({ "folderId": folder_id, "done": exists - low + 1, "total": exists - start + 1 }),
                    );
                    if low == start {
                        break;
                    }
                    high = low - 1;
                }
            }
        } else {
            // Incremental: anything above the high-water mark.
            let n = self
                .fetch_headers_uid(
                    folder_id,
                    &format!("{}:*", last_seen_uid + 1),
                    last_seen_uid,
                )
                .await?;
            changed |= n > 0;
        }

        // Record the new high-water mark.
        let max_uid: Option<i64> = db
            .call(move |conn| {
                conn.query_row(
                    "SELECT max(uid) FROM messages WHERE folder_id = ?1",
                    rusqlite::params![folder_id],
                    |r| r.get(0),
                )
            })
            .await?;
        if let Some(max_uid) = max_uid {
            db.call(move |conn| {
                conn.execute(
                    "UPDATE folders SET last_seen_uid = ?2 WHERE id = ?1",
                    rusqlite::params![folder_id, max_uid],
                )
                .map(|_| ())
            })
            .await?;
        }

        if changed {
            let _ = self
                .app
                .emit("mail:updated", json!({ "folderId": folder_id }));
        }
        Ok(changed)
    }

    async fn fetch_headers_seq(&mut self, folder_id: i64, seq_set: &str) -> Result<usize> {
        let session = self.session().await?;
        let mut fetched = Vec::new();
        {
            let mut stream = session
                .fetch(
                    seq_set,
                    "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])",
                )
                .await
                .map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                fetched.push(item.map_err(imap_err)?);
            }
        }
        self.store_headers(folder_id, fetched, 0).await
    }

    async fn fetch_headers_uid(
        &mut self,
        folder_id: i64,
        uid_set: &str,
        above_uid: i64,
    ) -> Result<usize> {
        let session = self.session().await?;
        let mut fetched = Vec::new();
        {
            let mut stream = session
                .uid_fetch(
                    uid_set,
                    "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])",
                )
                .await
                .map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                fetched.push(item.map_err(imap_err)?);
            }
        }
        self.store_headers(folder_id, fetched, above_uid).await
    }

    async fn store_headers(
        &mut self,
        folder_id: i64,
        fetched: Vec<async_imap::types::Fetch>,
        above_uid: i64,
    ) -> Result<usize> {
        let account_id = self.account.id.clone();
        let mut rows: Vec<NewMessage> = Vec::with_capacity(fetched.len());
        for f in &fetched {
            let Some(uid) = f.uid else { continue };
            if (uid as i64) <= above_uid {
                continue; // '*' can echo back the last existing message
            }
            let flags: Vec<async_imap::types::Flag> = f.flags().collect();
            let is_read = flags
                .iter()
                .any(|fl| matches!(fl, async_imap::types::Flag::Seen));
            let is_starred = flags
                .iter()
                .any(|fl| matches!(fl, async_imap::types::Flag::Flagged));
            let header_bytes = f.header().unwrap_or_default();
            let internal_date = f.internal_date().map(|d| d.timestamp());
            let msg = parse::parse_headers(
                &account_id,
                folder_id,
                uid,
                header_bytes,
                internal_date,
                f.size,
                is_read,
                is_starred,
                false,
            );
            rows.push(msg);
        }

        let count = rows.len();
        if count == 0 {
            return Ok(0);
        }
        self.db
            .call(move |conn| {
                for msg in &rows {
                    queries::insert_message(conn, msg)?;
                }
                Ok(())
            })
            .await?;
        Ok(count)
    }
}

async fn wipe_folder(db: &Db, folder_id: i64) -> Result<()> {
    db.call(move |conn| {
        let tx = conn.transaction()?;
        let thread_ids: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT thread_id FROM messages
                 WHERE folder_id = ?1 AND thread_id IS NOT NULL",
            )?;
            let ids = stmt
                .query_map(rusqlite::params![folder_id], |r| r.get(0))?
                .collect::<std::result::Result<Vec<i64>, _>>()?;
            ids
        };
        tx.execute(
            "DELETE FROM messages_fts WHERE rowid IN
               (SELECT id FROM messages WHERE folder_id = ?1)",
            rusqlite::params![folder_id],
        )?;
        tx.execute(
            "DELETE FROM messages WHERE folder_id = ?1",
            rusqlite::params![folder_id],
        )?;
        for tid in thread_ids {
            crate::mail::threading::recompute_thread(&tx, tid)?;
        }
        queries::recompute_folder_unread(&tx, folder_id)?;
        tx.commit()
    })
    .await
}

fn imap_err(e: async_imap::error::Error) -> SkimError {
    SkimError::other("imap", e.to_string())
}

fn detect_role(imap_name: &str, attrs_lower: &str) -> Option<String> {
    if imap_name.eq_ignore_ascii_case("INBOX") {
        return Some("inbox".into());
    }
    let by_attr = if attrs_lower.contains("\\\\all") || attrs_lower.contains("\\all") {
        Some("all")
    } else if attrs_lower.contains("sent") {
        Some("sent")
    } else if attrs_lower.contains("drafts") {
        Some("drafts")
    } else if attrs_lower.contains("trash") {
        Some("trash")
    } else if attrs_lower.contains("junk") || attrs_lower.contains("spam") {
        Some("junk")
    } else if attrs_lower.contains("archive") {
        Some("archive")
    } else if attrs_lower.contains("flagged") {
        Some("starred")
    } else {
        None
    };
    if let Some(role) = by_attr {
        return Some(role.to_string());
    }

    // Name heuristics for servers without SPECIAL-USE.
    let last = imap_name.rsplit(['/', '.']).next().unwrap_or(imap_name);
    let l = last.to_lowercase();
    let role = match l.as_str() {
        "sent" | "sent items" | "sent messages" | "sent mail" => "sent",
        "drafts" | "draft" => "drafts",
        "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" => "trash",
        "junk" | "spam" | "junk e-mail" => "junk",
        "archive" | "archives" | "all mail" => {
            if l == "all mail" {
                "all"
            } else {
                "archive"
            }
        }
        "important" | "starred" => "starred",
        _ => return None,
    };
    Some(role.to_string())
}

fn display_name(imap_name: &str, _provider: &str) -> String {
    imap_name
        .strip_prefix("[Gmail]/")
        .or_else(|| imap_name.strip_prefix("[Google Mail]/"))
        .unwrap_or(imap_name)
        .to_string()
}

/// OAuth client configuration: baked in at compile time or stored in
/// settings by the user.
pub async fn oauth_config(db: &Db) -> Result<Option<oauth::OauthConfig>> {
    if let Some(cfg) = oauth::baked_in_config() {
        return Ok(Some(cfg));
    }
    let id = db
        .call(|conn| queries::get_setting(conn, "google_client_id"))
        .await?;
    let secret = db
        .call(|conn| queries::get_setting(conn, "google_client_secret"))
        .await?;
    Ok(id
        .filter(|s| !s.is_empty())
        .map(|client_id| oauth::OauthConfig {
            client_id,
            client_secret: secret.unwrap_or_default(),
        }))
}
