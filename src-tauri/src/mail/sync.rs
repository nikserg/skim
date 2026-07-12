//! Per-account sync engine.
//!
//! One worker IMAP session executes everything (folder sync, body fetches,
//! the offline op queue) serialized through an mpsc command channel. A second
//! lightweight connection IDLEs on INBOX and pokes the worker when new mail
//! arrives; a 5-minute poll covers servers without IDLE.

use crate::db::models::{Account, NewMessage};
use crate::db::{bodies, queries, Db};
use crate::error::{Result, SkimError};
use crate::mail::{imap_client, oauth, parse};
use crate::secrets;
use futures::StreamExt;
use serde_json::json;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot};

const INBOX_WINDOW: u32 = 500;
const FOLDER_WINDOW: u32 = 200;
const CHUNK: u32 = 100;
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);
const IDLE_REISSUE: std::time::Duration = std::time::Duration::from_secs(25 * 60);

pub enum SyncCommand {
    SyncAll,
    SyncInbox,
    FetchBody {
        message_pk: i64,
        respond: oneshot::Sender<Result<()>>,
    },
    RunOps,
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
    pub fn run_ops(&self) {
        let _ = self.tx.send(SyncCommand::RunOps);
    }
    pub fn stop(&self) {
        let _ = self.tx.send(SyncCommand::Stop);
    }
    pub async fn fetch_body(&self, message_pk: i64) -> Result<()> {
        let (respond, rx) = oneshot::channel();
        self.tx
            .send(SyncCommand::FetchBody {
                message_pk,
                respond,
            })
            .map_err(|_| SkimError::other("sync", "sync engine is not running"))?;
        rx.await
            .map_err(|_| SkimError::other("sync", "sync engine dropped the request"))?
    }
}

struct Engine {
    app: AppHandle,
    db: Db,
    account: Account,
    data_dir: PathBuf,
    session: Option<imap_client::Session>,
    selected: Option<String>,
    oauth_token: Option<(String, i64)>,
}

pub fn spawn(app: AppHandle, db: Db, account: Account, data_dir: PathBuf) -> SyncHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<SyncCommand>();
    let handle = SyncHandle { tx: tx.clone() };

    spawn_idle_watcher(db.clone(), account.clone(), tx.clone());

    tauri::async_runtime::spawn(async move {
        let mut engine = Engine {
            app,
            db,
            account,
            data_dir,
            session: None,
            selected: None,
            oauth_token: None,
        };

        let mut poll = tokio::time::interval(POLL_INTERVAL);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                cmd = rx.recv() => {
                    match cmd {
                        None | Some(SyncCommand::Stop) => break,
                        Some(SyncCommand::SyncAll) => {
                            engine.drain_ops().await;
                            engine.run_sync().await;
                        }
                        Some(SyncCommand::SyncInbox) => {
                            engine.drain_ops().await;
                            engine.sync_inbox().await;
                        }
                        Some(SyncCommand::RunOps) => engine.drain_ops().await,
                        Some(SyncCommand::FetchBody { message_pk, respond }) => {
                            let result = engine.fetch_body(message_pk).await;
                            if result.is_err() {
                                // A broken session poisons every later fetch.
                                engine.reset_session();
                            }
                            let _ = respond.send(result);
                        }
                    }
                }
                _ = poll.tick() => {
                    engine.drain_ops().await;
                    engine.run_sync().await;
                }
            }
        }
        engine.logout().await;
    });

    handle
}

/// Shared credential resolution (worker + IDLE connection).
async fn resolve_credentials(
    db: &Db,
    account: &Account,
    oauth_cache: &mut Option<(String, i64)>,
) -> Result<imap_client::Credentials> {
    let secret = secrets::get(&secrets::mail_key(&account.id))?
        .ok_or_else(|| SkimError::other("auth", "no stored credentials for this account"))?;
    if account.auth_kind == "oauth" {
        let now = now_unix();
        if let Some((token, expires_at)) = oauth_cache {
            if *expires_at > now {
                return Ok(imap_client::Credentials::OauthToken(token.clone()));
            }
        }
        let config = oauth_config(db)
            .await?
            .ok_or_else(|| SkimError::other("oauth", "Google OAuth client id is not configured"))?;
        let (token, expires_at) = oauth::refresh_access_token(&config, &secret).await?;
        *oauth_cache = Some((token.clone(), expires_at));
        Ok(imap_client::Credentials::OauthToken(token))
    } else {
        Ok(imap_client::Credentials::Password(secret))
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Engine {
    fn emit_status(&self, state: &str, message: Option<String>) {
        let _ = self.app.emit(
            "sync:status",
            json!({ "accountId": self.account.id, "state": state, "message": message }),
        );
    }

    fn reset_session(&mut self) {
        self.session = None;
        self.selected = None;
    }

    async fn run_sync(&mut self) {
        self.emit_status("syncing", None);
        match self.sync_all_folders().await {
            Ok(()) => self.emit_status("idle", None),
            Err(e) => {
                tracing::warn!(error = %e, "sync failed");
                self.reset_session();
                self.emit_status("error", Some(e.to_string()));
            }
        }
    }

    async fn sync_inbox(&mut self) {
        let account_id = self.account.id.clone();
        let inbox = self
            .db
            .call(move |conn| {
                conn.query_row(
                    "SELECT id, imap_name FROM folders WHERE account_id = ?1 AND role = 'inbox'",
                    rusqlite::params![account_id],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )
            })
            .await;
        if let Ok((folder_id, imap_name)) = inbox {
            if let Err(e) = self.sync_folder(folder_id, &imap_name).await {
                tracing::warn!(error = %e, "inbox sync failed");
                self.reset_session();
            }
        }
    }

    async fn logout(&mut self) {
        if let Some(mut s) = self.session.take() {
            let _ = s.logout().await;
        }
    }

    async fn session(&mut self) -> Result<&mut imap_client::Session> {
        if self.session.is_none() {
            let mut cache = self.oauth_token.take();
            let creds = resolve_credentials(&self.db, &self.account, &mut cache).await?;
            self.oauth_token = cache;
            let session = imap_client::login(
                &self.account.imap_host,
                self.account.imap_port,
                &self.account.email,
                &creds,
            )
            .await?;
            self.session = Some(session);
            self.selected = None;
        }
        Ok(self.session.as_mut().expect("just set"))
    }

    async fn ensure_selected(&mut self, imap_name: &str) -> Result<()> {
        if self.selected.as_deref() == Some(imap_name) {
            return Ok(());
        }
        let session = self.session().await?;
        session
            .select(imap_name)
            .await
            .map_err(|e| SkimError::other("folder", format!("cannot open {imap_name}: {e}")))?;
        self.selected = Some(imap_name.to_string());
        Ok(())
    }

    // ---- folder discovery & header sync -------------------------------

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

    /// Sync one folder: new headers above the UID high-water mark, plus a
    /// flag/expunge reconciliation pass over the newest cached window.
    async fn sync_folder(&mut self, folder_id: i64, imap_name: &str) -> Result<bool> {
        let is_inbox = imap_name.eq_ignore_ascii_case("INBOX");
        // Force a real SELECT so EXISTS/UIDVALIDITY are fresh.
        self.selected = None;
        let session = self.session().await?;
        let mailbox = session
            .select(imap_name)
            .await
            .map_err(|e| SkimError::other("folder", format!("cannot open {imap_name}: {e}")))?;
        self.selected = Some(imap_name.to_string());

        let uidvalidity = mailbox.uid_validity.unwrap_or(0) as i64;
        let exists = mailbox.exists;

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
                        .fetch_headers(folder_id, &format!("{low}:{high}"), false, 0)
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
            let n = self
                .fetch_headers(
                    folder_id,
                    &format!("{}:*", last_seen_uid + 1),
                    true,
                    last_seen_uid,
                )
                .await?;
            changed |= n > 0;
            changed |= self.reconcile_flags(folder_id).await?;
        }

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

    async fn fetch_headers(
        &mut self,
        folder_id: i64,
        set: &str,
        by_uid: bool,
        above_uid: i64,
    ) -> Result<usize> {
        const QUERY: &str = "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])";
        let session = self.session().await?;
        let mut fetched = Vec::new();
        if by_uid {
            let mut stream = session.uid_fetch(set, QUERY).await.map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                fetched.push(item.map_err(imap_err)?);
            }
        } else {
            let mut stream = session.fetch(set, QUERY).await.map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                fetched.push(item.map_err(imap_err)?);
            }
        }

        let account_id = self.account.id.clone();
        let mut rows: Vec<NewMessage> = Vec::with_capacity(fetched.len());
        for f in &fetched {
            let Some(uid) = f.uid else { continue };
            if (uid as i64) <= above_uid {
                continue; // '*' echoes back the last existing message
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
            rows.push(parse::parse_headers(
                &account_id,
                folder_id,
                uid,
                header_bytes,
                internal_date,
                f.size,
                is_read,
                is_starred,
                false,
            ));
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

    /// Diff server flags against the newest cached window; detect expunges.
    async fn reconcile_flags(&mut self, folder_id: i64) -> Result<bool> {
        let cached: Vec<(i64, u32, bool, bool)> = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT id, uid, is_read, is_starred FROM messages
                     WHERE folder_id = ?1 ORDER BY uid DESC LIMIT 500",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![folder_id], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        if cached.is_empty() {
            return Ok(false);
        }

        let min_uid = cached.iter().map(|(_, uid, _, _)| *uid).min().unwrap_or(1);
        let max_uid = cached.iter().map(|(_, uid, _, _)| *uid).max().unwrap_or(1);
        let session = self.session().await?;
        let mut server: std::collections::HashMap<u32, (bool, bool)> =
            std::collections::HashMap::new();
        {
            let mut stream = session
                .uid_fetch(format!("{min_uid}:{max_uid}"), "(UID FLAGS)")
                .await
                .map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                let f = item.map_err(imap_err)?;
                if let Some(uid) = f.uid {
                    let flags: Vec<async_imap::types::Flag> = f.flags().collect();
                    server.insert(
                        uid,
                        (
                            flags
                                .iter()
                                .any(|fl| matches!(fl, async_imap::types::Flag::Seen)),
                            flags
                                .iter()
                                .any(|fl| matches!(fl, async_imap::types::Flag::Flagged)),
                        ),
                    );
                }
            }
        }

        let mut read_on = Vec::new();
        let mut read_off = Vec::new();
        let mut star_on = Vec::new();
        let mut star_off = Vec::new();
        let mut gone = Vec::new();
        for (pk, uid, is_read, is_starred) in &cached {
            match server.get(uid) {
                None => gone.push(*pk),
                Some((seen, flagged)) => {
                    if seen != is_read {
                        if *seen {
                            read_on.push(*pk)
                        } else {
                            read_off.push(*pk)
                        }
                    }
                    if flagged != is_starred {
                        if *flagged {
                            star_on.push(*pk)
                        } else {
                            star_off.push(*pk)
                        }
                    }
                }
            }
        }

        let changed = !(read_on.is_empty()
            && read_off.is_empty()
            && star_on.is_empty()
            && star_off.is_empty()
            && gone.is_empty());
        if changed {
            self.db
                .call(move |conn| {
                    if !read_on.is_empty() {
                        bodies::set_flag_local(conn, &read_on, "seen", true)?;
                    }
                    if !read_off.is_empty() {
                        bodies::set_flag_local(conn, &read_off, "seen", false)?;
                    }
                    if !star_on.is_empty() {
                        bodies::set_flag_local(conn, &star_on, "flagged", true)?;
                    }
                    if !star_off.is_empty() {
                        bodies::set_flag_local(conn, &star_off, "flagged", false)?;
                    }
                    if !gone.is_empty() {
                        bodies::remove_messages_local(conn, &gone)?;
                    }
                    Ok(())
                })
                .await?;
        }
        Ok(changed)
    }

    // ---- bodies --------------------------------------------------------

    async fn fetch_body(&mut self, message_pk: i64) -> Result<()> {
        let coords: Option<(String, u32, i64)> = self
            .db
            .call(move |conn| {
                conn.query_row(
                    "SELECT f.imap_name, m.uid, m.body_state
                     FROM messages m JOIN folders f ON f.id = m.folder_id
                     WHERE m.id = ?1",
                    rusqlite::params![message_pk],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map(Some)
            })
            .await
            .unwrap_or(None);
        let Some((imap_name, uid, body_state)) = coords else {
            return Err(SkimError::other("mail", "message no longer exists"));
        };
        if body_state == 1 {
            return Ok(());
        }

        self.ensure_selected(&imap_name).await?;
        let session = self.session().await?;
        let mut raw: Option<Vec<u8>> = None;
        {
            let mut stream = session
                .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")
                .await
                .map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                let f = item.map_err(imap_err)?;
                if f.uid == Some(uid) {
                    raw = f.body().map(|b| b.to_vec());
                }
            }
        }
        let Some(raw) = raw else {
            return Err(SkimError::other("mail", "server returned no message body"));
        };

        let parsed = parse::parse_body(&raw);

        // Attachments go to the on-disk cache, keyed by message pk.
        let dir = self
            .data_dir
            .join("attachments")
            .join(message_pk.to_string());
        let mut stored = Vec::new();
        if !parsed.attachments.is_empty() {
            std::fs::create_dir_all(&dir)?;
        }
        for (i, a) in parsed.attachments.iter().enumerate() {
            let safe_name = a
                .filename
                .as_deref()
                .unwrap_or("attachment")
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || ".-_ ()".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect::<String>();
            let path = dir.join(format!("{i}_{safe_name}"));
            std::fs::write(&path, &a.data)?;
            stored.push(bodies::StoredAttachment {
                filename: a.filename.clone(),
                mime_type: a.mime_type.clone(),
                size: a.size,
                content_id: a.content_id.clone(),
                is_inline: a.is_inline,
                cache_path: path.to_string_lossy().into_owned(),
            });
        }

        let html = parsed.html;
        let text = parsed.text;
        let snippet = parsed.snippet;
        self.db
            .call(move |conn| {
                bodies::set_body(
                    conn,
                    message_pk,
                    html.as_deref(),
                    text.as_deref(),
                    &snippet,
                    &stored,
                )
            })
            .await?;
        Ok(())
    }

    // ---- offline op queue ----------------------------------------------

    async fn drain_ops(&mut self) {
        let mut affected: std::collections::HashSet<i64> = std::collections::HashSet::new();
        loop {
            let account_id = self.account.id.clone();
            let next: Option<(i64, String, String, i64)> = match self
                .db
                .call(move |conn| {
                    use rusqlite::OptionalExtension;
                    conn.query_row(
                        "SELECT id, kind, payload, attempts FROM pending_ops
                         WHERE account_id = ?1 AND state = 'pending' ORDER BY id LIMIT 1",
                        rusqlite::params![account_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()
                })
                .await
            {
                Ok(v) => v,
                Err(_) => break,
            };
            let Some((op_id, kind, payload, attempts)) = next else {
                break;
            };

            let parsed: serde_json::Value = match serde_json::from_str(&payload) {
                Ok(v) => v,
                Err(_) => {
                    let _ = self.finish_op(op_id, false).await;
                    continue;
                }
            };
            match self.execute_op(&kind, &parsed).await {
                Ok(folder_id) => {
                    if let Some(fid) = folder_id {
                        affected.insert(fid);
                    }
                    let _ = self.finish_op(op_id, true).await;
                }
                Err(e) => {
                    tracing::warn!(op = %kind, error = %e, "op failed");
                    self.reset_session();
                    match e.code() {
                        // Transient — retry on the next drain.
                        "network" | "tls" | "oauth" => break,
                        _ => {
                            if attempts + 1 >= 5 {
                                let _ = self.finish_op(op_id, false).await;
                                let _ = self.app.emit(
                                    "ops:failed",
                                    json!({ "kind": kind, "message": e.to_string() }),
                                );
                            } else {
                                let dbc = self.db.clone();
                                let _ = dbc
                                    .call(move |conn| {
                                        conn.execute(
                                            "UPDATE pending_ops SET attempts = attempts + 1 WHERE id = ?1",
                                            rusqlite::params![op_id],
                                        )
                                        .map(|_| ())
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Ops mutate server state; refresh the folders they touched.
        for folder_id in affected {
            let name: std::result::Result<String, _> = self
                .db
                .call(move |conn| {
                    conn.query_row(
                        "SELECT imap_name FROM folders WHERE id = ?1",
                        rusqlite::params![folder_id],
                        |r| r.get(0),
                    )
                })
                .await;
            if let Ok(name) = name {
                let _ = self.sync_folder(folder_id, &name).await;
            }
        }
    }

    async fn finish_op(&self, op_id: i64, success: bool) -> Result<()> {
        self.db
            .call(move |conn| {
                if success {
                    conn.execute(
                        "DELETE FROM pending_ops WHERE id = ?1",
                        rusqlite::params![op_id],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE pending_ops SET state = 'failed' WHERE id = ?1",
                        rusqlite::params![op_id],
                    )?;
                }
                Ok(())
            })
            .await
    }

    /// Execute one queued op. Returns the folder id to resync afterwards.
    async fn execute_op(&mut self, kind: &str, payload: &serde_json::Value) -> Result<Option<i64>> {
        let imap_name = payload["imapName"].as_str().unwrap_or_default().to_string();
        let folder_id = payload["folderId"].as_i64();
        let uids: Vec<u32> = payload["uids"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_u64().map(|u| u as u32))
                    .collect()
            })
            .unwrap_or_default();
        if imap_name.is_empty() || uids.is_empty() {
            return Ok(None);
        }
        let uid_set = uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        self.ensure_selected(&imap_name).await?;

        match kind {
            "set_flag" => {
                let flag = match payload["flag"].as_str() {
                    Some("flagged") => "\\Flagged",
                    _ => "\\Seen",
                };
                let sign = if payload["on"].as_bool().unwrap_or(true) {
                    "+"
                } else {
                    "-"
                };
                let session = self.session().await?;
                let mut stream = session
                    .uid_store(&uid_set, format!("{sign}FLAGS ({flag})"))
                    .await
                    .map_err(imap_err)?;
                while let Some(item) = stream.next().await {
                    item.map_err(imap_err)?;
                }
            }
            "archive" => {
                let is_gmail_inbox =
                    self.account.provider == "gmail" && imap_name.eq_ignore_ascii_case("INBOX");
                if is_gmail_inbox {
                    // Gmail archive = remove the INBOX label; the message
                    // stays in All Mail.
                    self.delete_and_expunge(&uid_set).await?;
                } else {
                    let dest = self.role_folder("archive", "Archive").await?;
                    self.move_uids(&uid_set, &dest).await?;
                }
            }
            "delete" => {
                let dest = self.role_folder("trash", "Trash").await.ok();
                match dest {
                    Some(dest) if !dest.eq_ignore_ascii_case(&imap_name) => {
                        self.move_uids(&uid_set, &dest).await?;
                    }
                    // Already in trash (or no trash folder): permanent delete.
                    _ => self.delete_and_expunge(&uid_set).await?,
                }
            }
            other => {
                return Err(SkimError::other("ops", format!("unknown op kind: {other}")));
            }
        }
        Ok(folder_id)
    }

    async fn delete_and_expunge(&mut self, uid_set: &str) -> Result<()> {
        let session = self.session().await?;
        {
            let mut stream = session
                .uid_store(uid_set, "+FLAGS (\\Deleted)")
                .await
                .map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                item.map_err(imap_err)?;
            }
        }
        // UID EXPUNGE (UIDPLUS) only touches our messages; fall back to a
        // full EXPUNGE on servers without it.
        let uidplus_failed = {
            match session.uid_expunge(uid_set).await {
                Ok(stream) => {
                    let mut stream = std::pin::pin!(stream);
                    while let Some(item) = stream.next().await {
                        item.map_err(imap_err)?;
                    }
                    false
                }
                Err(e) => {
                    tracing::debug!(error = %e, "UID EXPUNGE unsupported; using EXPUNGE");
                    true
                }
            }
        };
        if uidplus_failed {
            let stream = session.expunge().await.map_err(imap_err)?;
            let mut stream = std::pin::pin!(stream);
            while let Some(item) = stream.next().await {
                item.map_err(imap_err)?;
            }
        }
        Ok(())
    }

    async fn move_uids(&mut self, uid_set: &str, dest: &str) -> Result<()> {
        let session = self.session().await?;
        match session.uid_mv(uid_set, dest).await {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::debug!(error = %e, "UID MOVE unsupported; using COPY+DELETE");
                session.uid_copy(uid_set, dest).await.map_err(imap_err)?;
                self.delete_and_expunge(uid_set).await
            }
        }
    }

    /// Find (or create) the folder with the given role.
    async fn role_folder(&mut self, role: &str, create_name: &str) -> Result<String> {
        let account_id = self.account.id.clone();
        let role_owned = role.to_string();
        let existing: Option<String> = self
            .db
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                conn.query_row(
                    "SELECT imap_name FROM folders WHERE account_id = ?1 AND role = ?2",
                    rusqlite::params![account_id, role_owned],
                    |r| r.get(0),
                )
                .optional()
            })
            .await?;
        if let Some(name) = existing {
            return Ok(name);
        }

        let session = self.session().await?;
        session
            .create(create_name)
            .await
            .map_err(|e| SkimError::other("folder", format!("cannot create {create_name}: {e}")))?;
        let account_id = self.account.id.clone();
        let name = create_name.to_string();
        let role_owned = role.to_string();
        let display = create_name.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO folders (account_id, imap_name, role, display_name, sort_order)
                     VALUES (?1, ?2, ?3, ?4, 30)",
                    rusqlite::params![account_id, name, role_owned, display],
                )
                .map(|_| ())
            })
            .await?;
        let _ = self.app.emit("folders:updated", json!({}));
        Ok(create_name.to_string())
    }
}

// ---- IDLE watcher -------------------------------------------------------

fn spawn_idle_watcher(db: Db, account: Account, tx: mpsc::UnboundedSender<SyncCommand>) {
    tauri::async_runtime::spawn(async move {
        let mut oauth_cache: Option<(String, i64)> = None;
        let mut backoff = 5u64;
        loop {
            if tx.is_closed() {
                break;
            }
            match idle_session(&db, &account, &tx, &mut oauth_cache).await {
                Ok(()) => backoff = 5,
                Err(e) => {
                    tracing::debug!(error = %e, "IDLE connection ended");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(300);
        }
    });
}

async fn idle_session(
    db: &Db,
    account: &Account,
    tx: &mpsc::UnboundedSender<SyncCommand>,
    oauth_cache: &mut Option<(String, i64)>,
) -> Result<()> {
    let creds = resolve_credentials(db, account, oauth_cache).await?;
    let mut session = imap_client::login(
        &account.imap_host,
        account.imap_port,
        &account.email,
        &creds,
    )
    .await?;
    session
        .select("INBOX")
        .await
        .map_err(|e| SkimError::other("imap", e.to_string()))?;

    loop {
        if tx.is_closed() {
            let _ = session.logout().await;
            return Ok(());
        }
        let mut idle = session.idle();
        idle.init().await.map_err(imap_err)?;
        let (wait, _interrupt) = idle.wait_with_timeout(IDLE_REISSUE);
        let outcome = wait.await;
        session = idle.done().await.map_err(imap_err)?;
        match outcome {
            Ok(async_imap::extensions::idle::IdleResponse::NewData(_)) => {
                let _ = tx.send(SyncCommand::SyncInbox);
            }
            Ok(_) => {} // timeout → re-issue IDLE
            Err(e) => return Err(imap_err(e)),
        }
    }
}

// ---- helpers ------------------------------------------------------------

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
