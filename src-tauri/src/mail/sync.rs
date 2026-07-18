//! Per-account sync engine.
//!
//! One worker IMAP session executes everything (folder sync, body fetches,
//! the offline op queue) serialized through an mpsc command channel. A second
//! lightweight connection IDLEs on INBOX and pokes the worker when new mail
//! arrives; a periodic poll reconciles what IDLE can't see (other folders,
//! flag/read state changed on another device), gated by a cheap STATUS probe.

use crate::db::models::{Account, NewMessage};
use crate::db::{bodies, queries, Db};
use crate::error::{Result, SkimError};
use crate::mail::{imap_client, oauth, parse, smtp};
use crate::secrets;
use futures::StreamExt;
use serde_json::json;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot};

const INBOX_WINDOW: u32 = 500;
const FOLDER_WINDOW: u32 = 200;
const CHUNK: u32 = 100;
// IDLE keeps the inbox instant, so this poll only backfills the slow-changing
// rest (other folders, read state from other devices) — it can run infrequently.
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

/// A folder's server-side STATUS snapshot. When an untouched folder reports the
/// same values as last pass, its expensive SELECT + flag fetch can be skipped.
#[derive(Clone, Copy)]
struct FolderStatus {
    uidvalidity: i64,
    uidnext: i64,
    exists: i64,
    unseen: i64,
}

pub fn spawn(app: AppHandle, db: Db, account: Account, data_dir: PathBuf) -> SyncHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<SyncCommand>();
    let handle = SyncHandle { tx: tx.clone() };

    spawn_idle_watcher(account.clone(), tx.clone());

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
        let provider = oauth_provider_for(account);
        let config = oauth::baked_in_config(provider)
            .ok_or_else(|| SkimError::other("oauth", "OAuth client id is not configured"))?;
        let refreshed = oauth::refresh_access_token(&config, &secret).await?;
        // Microsoft rotates the refresh token on every use; persist the new one
        // so the account keeps working past the old token's lifetime.
        if let Some(new_rt) = refreshed.new_refresh_token {
            if new_rt != secret {
                secrets::set(&secrets::mail_key(&account.id), &new_rt)?;
            }
        }
        *oauth_cache = Some((refreshed.access_token.clone(), refreshed.expires_at));
        Ok(imap_client::Credentials::OauthToken(refreshed.access_token))
    } else {
        Ok(imap_client::Credentials::Password(secret))
    }
}

/// Which OAuth issuer backs this account, derived from its provider. `auth_kind`
/// stays a plain "password"/"oauth" flag, so existing accounts need no migration.
fn oauth_provider_for(account: &Account) -> oauth::OauthProvider {
    if account.provider == "microsoft" {
        oauth::OauthProvider::Microsoft
    } else {
        oauth::OauthProvider::Google
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
            let creds = resolve_credentials(&self.account, &mut cache).await?;
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

        let started = std::time::Instant::now();
        let total = folders.len();
        let mut synced = 0usize;
        let mut skipped = 0usize;
        let mut any_changes = false;
        for folder in folders {
            if folder.role.as_deref() == Some("all") {
                continue;
            }

            // A cheap STATUS probe gates the expensive SELECT + flag fetch:
            // skip folders that report the same snapshot as the last pass. IMAP
            // forbids STATUS on the selected mailbox, so that one always syncs.
            let probe = if self.selected.as_deref() == Some(&folder.imap_name) {
                None
            } else {
                match self.probe_status(&folder.imap_name).await {
                    Ok(st) => {
                        if self.status_matches(folder.id, &st).await {
                            skipped += 1;
                            continue;
                        }
                        Some(st)
                    }
                    // A probe failure that looks like a session problem aborts
                    // the pass; anything else falls through to a full sync.
                    Err(e) => match e.code() {
                        "auth" | "network" | "tls" | "oauth" | "oauth_expired" => return Err(e),
                        _ => {
                            tracing::debug!(folder = %folder.imap_name, error = %e, "STATUS probe failed");
                            None
                        }
                    },
                }
            };

            match self.sync_folder(folder.id, &folder.imap_name).await {
                Ok(changed) => {
                    synced += 1;
                    any_changes |= changed;
                    if let Some(st) = probe {
                        self.store_status(folder.id, &st).await;
                    }
                }
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
        tracing::info!(
            total,
            synced,
            skipped,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "folder sweep complete"
        );
        Ok(())
    }

    /// Cheap server-side snapshot of a folder — one round-trip, no SELECT.
    async fn probe_status(&mut self, imap_name: &str) -> Result<FolderStatus> {
        let session = self.session().await?;
        let mb = session
            .status(imap_name, "(UIDVALIDITY UIDNEXT MESSAGES UNSEEN)")
            .await
            .map_err(imap_err)?;
        Ok(FolderStatus {
            uidvalidity: mb.uid_validity.unwrap_or(0) as i64,
            uidnext: mb.uid_next.unwrap_or(0) as i64,
            exists: mb.exists as i64,
            unseen: mb.unseen.unwrap_or(0) as i64,
        })
    }

    /// True when `st` equals the snapshot stored on the last successful sync, so
    /// the folder is provably unchanged and can be skipped.
    async fn status_matches(&self, folder_id: i64, st: &FolderStatus) -> bool {
        let stored = self
            .db
            .call(move |conn| {
                conn.query_row(
                    "SELECT status_uidvalidity, status_uidnext, status_exists, status_unseen
                     FROM folders WHERE id = ?1",
                    rusqlite::params![folder_id],
                    |r| {
                        Ok((
                            r.get::<_, Option<i64>>(0)?,
                            r.get::<_, Option<i64>>(1)?,
                            r.get::<_, Option<i64>>(2)?,
                            r.get::<_, Option<i64>>(3)?,
                        ))
                    },
                )
            })
            .await;
        matches!(
            stored,
            Ok((Some(uv), Some(un), Some(ex), Some(us)))
                if uv == st.uidvalidity && un == st.uidnext && ex == st.exists && us == st.unseen
        )
    }

    /// Persist the snapshot that `status_matches` compares against next pass.
    async fn store_status(&self, folder_id: i64, st: &FolderStatus) {
        let st = *st;
        let _ = self
            .db
            .call(move |conn| {
                conn.execute(
                    "UPDATE folders
                     SET status_uidvalidity = ?2, status_uidnext = ?3,
                         status_exists = ?4, status_unseen = ?5
                     WHERE id = ?1",
                    rusqlite::params![folder_id, st.uidvalidity, st.uidnext, st.exists, st.unseen],
                )
                .map(|_| ())
            })
            .await;
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

        // A folder can gain the 'all' role after its contents were already
        // synced (e.g. the attribute was missed on an earlier run) — those
        // rows shadow every other folder, so drop them.
        let account_id = self.account.id.clone();
        let stale_all: Vec<i64> = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT f.id FROM folders f
                     WHERE f.account_id = ?1 AND f.role = 'all'
                       AND EXISTS (SELECT 1 FROM messages m WHERE m.folder_id = f.id)",
                )?;
                let ids = stmt
                    .query_map(rusqlite::params![account_id], |r| r.get(0))?
                    .collect::<std::result::Result<Vec<i64>, _>>()?;
                Ok(ids)
            })
            .await?;
        for folder_id in stale_all {
            wipe_folder(&self.db, folder_id).await?;
        }

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
                    let inserted = self
                        .fetch_headers(folder_id, &format!("{low}:{high}"), false, 0)
                        .await?;
                    changed |= !inserted.is_empty();
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
            let inserted = self
                .fetch_headers(
                    folder_id,
                    &format!("{}:*", last_seen_uid + 1),
                    true,
                    last_seen_uid,
                )
                .await?;
            changed |= !inserted.is_empty();
            if !inserted.is_empty() && is_inbox {
                let _ = self
                    .app
                    .emit("mail:new", json!({ "count": inserted.len() }));
                crate::notify::notify_new_mail(&self.app, &self.db, &inserted).await;
            }
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
    ) -> Result<Vec<i64>> {
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

        if rows.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .call(move |conn| {
                let mut inserted = Vec::new();
                for msg in &rows {
                    if let Some((pk, _thread)) = queries::insert_message(conn, msg)? {
                        inserted.push(pk);
                    }
                }
                Ok(inserted)
            })
            .await
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
                                // A permanently failed RSVP must not keep showing
                                // the optimistic "accepted" pill: drop the stored
                                // answer so the card reverts on the next render.
                                let event_uid = if kind == "rsvp" {
                                    parsed
                                        .get("eventUid")
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string)
                                } else {
                                    None
                                };
                                if let Some(uid) = event_uid {
                                    let account_id = self.account.id.clone();
                                    let _ = self
                                        .db
                                        .call(move |conn| {
                                            bodies::delete_rsvp(conn, &account_id, &uid)
                                        })
                                        .await;
                                    let _ = self.app.emit("mail:updated", json!({}));
                                }
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
        if kind == "send" {
            return self.execute_send(payload).await;
        }
        if kind == "rsvp" {
            return self.execute_rsvp(payload).await;
        }
        if kind == "unsubscribe" {
            return self.execute_unsubscribe(payload).await;
        }
        if kind == "save_draft" {
            return self.execute_save_draft(payload).await;
        }
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
            "junk" => {
                let dest = self.role_folder("junk", "Junk").await?;
                // Already in the junk folder: nothing to move.
                if !dest.eq_ignore_ascii_case(&imap_name) {
                    self.move_uids(&uid_set, &dest).await?;
                }
            }
            other => {
                return Err(SkimError::other("ops", format!("unknown op kind: {other}")));
            }
        }
        Ok(folder_id)
    }

    /// Send a queued draft over SMTP, mirror it to Sent (non-Gmail), and
    /// delete the draft.
    async fn execute_send(&mut self, payload: &serde_json::Value) -> Result<Option<i64>> {
        let Some(draft_id) = payload["draftId"].as_i64() else {
            return Ok(None);
        };
        let draft = self
            .db
            .call(move |conn| crate::db::drafts::get(conn, draft_id))
            .await?;
        let Some(draft) = draft else {
            return Ok(None); // already sent or discarded
        };

        // Threading headers from the message being replied to.
        let refs = self.outgoing_refs(draft.reply_to_message_id).await?;

        // A server-backed draft keeps a stable Message-ID so the Sent copy and
        // the draft we're about to remove from the Drafts folder share it.
        let imap_message_id = self
            .db
            .call(move |conn| crate::db::drafts::origin_coords(conn, draft_id))
            .await?
            .and_then(|(_, mid)| mid);

        let attachments = self
            .db
            .call(move |conn| crate::db::draft_attachments::load_for_send(conn, draft_id))
            .await?;

        let raw = smtp::build_message(
            &self.account,
            &draft,
            &refs,
            &attachments,
            imap_message_id.as_deref(),
            false,
        )?;

        let mut cache = self.oauth_token.take();
        let creds = resolve_credentials(&self.account, &mut cache).await?;
        self.oauth_token = cache;
        smtp::send(&self.account, &creds, &raw).await?;

        let sent_folder_id = self.mirror_to_sent(&raw).await;

        // Remove the now-sent copy from the Drafts folder (server + local).
        if let Some(mid) = &imap_message_id {
            if let Err(e) = self.remove_server_draft(mid).await {
                tracing::warn!(error = %e, "cannot remove sent draft from Drafts folder");
            }
        }

        self.db
            .call(move |conn| crate::db::drafts::delete(conn, draft_id))
            .await?;
        let _ = self.app.emit("drafts:updated", json!({}));
        let _ = self.app.emit("mail:sent", json!({ "draftId": draft_id }));
        Ok(sent_folder_id)
    }

    /// Build threading headers (In-Reply-To/References) for an outgoing message
    /// from the local parent it replies to.
    async fn outgoing_refs(
        &mut self,
        reply_to_message_id: Option<i64>,
    ) -> Result<smtp::OutgoingRefs> {
        let Some(parent_id) = reply_to_message_id else {
            return Ok(smtp::OutgoingRefs {
                in_reply_to: None,
                references: Vec::new(),
            });
        };
        self.db
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                let row: Option<(Option<String>, Option<String>)> = conn
                    .query_row(
                        "SELECT message_id, references_ids FROM messages WHERE id = ?1",
                        rusqlite::params![parent_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let (msgid, refs_json) = row.unwrap_or((None, None));
                let mut references: Vec<String> = refs_json
                    .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|r| format!("<{r}>"))
                    .collect();
                let in_reply_to = msgid.map(|m| format!("<{m}>"));
                if let Some(irt) = &in_reply_to {
                    references.push(irt.clone());
                }
                Ok(smtp::OutgoingRefs {
                    in_reply_to,
                    references,
                })
            })
            .await
    }

    /// Write a server-backed draft back to the IMAP Drafts folder: append the
    /// current MIME with the `\Draft` flag under its stable Message-ID, then
    /// expunge any prior copies sharing that Message-ID. Ordering the SEARCH
    /// before the APPEND keeps retries idempotent (they converge to one copy).
    async fn execute_save_draft(&mut self, payload: &serde_json::Value) -> Result<Option<i64>> {
        let Some(draft_id) = payload["draftId"].as_i64() else {
            return Ok(None);
        };
        let draft = self
            .db
            .call(move |conn| crate::db::drafts::get(conn, draft_id))
            .await?;
        let Some(draft) = draft else {
            return Ok(None); // sent or discarded before this op drained
        };
        let imap_message_id = self
            .db
            .call(move |conn| crate::db::drafts::origin_coords(conn, draft_id))
            .await?
            .and_then(|(_, mid)| mid);
        let Some(imap_message_id) = imap_message_id else {
            return Ok(None); // not a server-backed draft; nothing to write
        };

        let refs = self.outgoing_refs(draft.reply_to_message_id).await?;
        let attachments = self
            .db
            .call(move |conn| crate::db::draft_attachments::load_for_send(conn, draft_id))
            .await?;
        let raw = smtp::build_message(
            &self.account,
            &draft,
            &refs,
            &attachments,
            Some(&imap_message_id),
            true,
        )?;

        let drafts_name = self.role_folder("drafts", "Drafts").await?;
        self.ensure_selected(&drafts_name).await?;
        let old_uids = self.uid_search_message_id(&imap_message_id).await?;
        {
            let session = self.session().await?;
            session
                .append(&drafts_name, Some("(\\Draft)"), None, &raw)
                .await
                .map_err(imap_err)?;
        }
        if !old_uids.is_empty() {
            let set = old_uids
                .iter()
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(",");
            self.delete_and_expunge(&set).await?;
        }
        Ok(self.folder_id_by_name(&drafts_name).await)
    }

    /// Delete a server draft (identified by its Message-ID) from the Drafts
    /// folder. Used when a draft is sent or discarded.
    async fn remove_server_draft(&mut self, imap_message_id: &str) -> Result<()> {
        let drafts_name = self.role_folder("drafts", "Drafts").await?;
        self.ensure_selected(&drafts_name).await?;
        let uids = self.uid_search_message_id(imap_message_id).await?;
        if !uids.is_empty() {
            let set = uids
                .iter()
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(",");
            self.delete_and_expunge(&set).await?;
        }
        Ok(())
    }

    /// UID SEARCH the selected mailbox for a message by its Message-ID header.
    async fn uid_search_message_id(&mut self, imap_message_id: &str) -> Result<Vec<u32>> {
        let session = self.session().await?;
        let set = session
            .uid_search(format!("HEADER MESSAGE-ID {imap_message_id}"))
            .await
            .map_err(imap_err)?;
        Ok(set.into_iter().collect())
    }

    /// Look up a folder's local id by its IMAP name.
    async fn folder_id_by_name(&mut self, imap_name: &str) -> Option<i64> {
        let account_id = self.account.id.clone();
        let name = imap_name.to_string();
        self.db
            .call(move |conn| {
                use rusqlite::OptionalExtension;
                conn.query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND imap_name = ?2",
                    rusqlite::params![account_id, name],
                    |r| r.get(0),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    /// Send a queued calendar RSVP (iMIP METHOD:REPLY) to the organizer.
    /// The payload is self-contained so the op survives the original
    /// invitation message being archived or deleted.
    async fn execute_rsvp(&mut self, payload: &serde_json::Value) -> Result<Option<i64>> {
        let to = payload["to"].as_str().unwrap_or_default().to_string();
        let subject = payload["subject"].as_str().unwrap_or_default().to_string();
        let text_body = payload["textBody"].as_str().unwrap_or_default().to_string();
        let ics = payload["ics"].as_str().unwrap_or_default().to_string();
        if to.is_empty() || ics.is_empty() {
            return Ok(None);
        }

        let raw = smtp::build_calendar_reply(&self.account, &to, &subject, &text_body, &ics)?;

        let mut cache = self.oauth_token.take();
        let creds = resolve_credentials(&self.account, &mut cache).await?;
        self.oauth_token = cache;
        smtp::send(&self.account, &creds, &raw).await?;

        Ok(self.mirror_to_sent(&raw).await)
    }

    /// Run a queued unsubscribe op. Either POSTs `List-Unsubscribe=One-Click`
    /// to the list's https endpoint (RFC 8058) or sends a small unsubscribe
    /// email over SMTP. The payload is self-contained, so it survives the
    /// original message being archived or deleted.
    async fn execute_unsubscribe(&mut self, payload: &serde_json::Value) -> Result<Option<i64>> {
        match payload["method"].as_str() {
            Some("post") => {
                let url = payload["url"].as_str().unwrap_or_default();
                if url.is_empty() {
                    return Ok(None);
                }
                // The URL comes straight from a message header, i.e. from the
                // sender — this is an SSRF boundary. https only, the host must
                // resolve to public addresses, and the checked addresses are
                // pinned so a second DNS answer can't swap in a private one.
                let (target, addrs) = crate::net::vet_public_url(url, true, "unsubscribe").await?;
                let host = target.host_str().unwrap_or_default().to_string();
                let client = reqwest::Client::builder()
                    // A redirect could hop from the vetted https host to an
                    // internal one; RFC 8058 expects a direct 2xx anyway.
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(std::time::Duration::from_secs(30))
                    .resolve_to_addrs(&host, &addrs)
                    .build()
                    .map_err(|e| SkimError::other("unsubscribe", e.to_string()))?;
                let resp = client
                    .post(target)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body("List-Unsubscribe=One-Click")
                    .send()
                    .await
                    .map_err(|e| SkimError::other("unsubscribe", e.to_string()))?;
                if !resp.status().is_success() {
                    return Err(SkimError::other(
                        "unsubscribe",
                        format!("list server returned {}", resp.status()),
                    ));
                }
                Ok(None)
            }
            Some("mail") => {
                let to = payload["to"].as_str().unwrap_or_default();
                let subject = payload["subject"].as_str().unwrap_or("unsubscribe");
                if to.is_empty() {
                    return Ok(None);
                }
                let raw = smtp::build_unsubscribe_mail(&self.account, to, subject)?;

                let mut cache = self.oauth_token.take();
                let creds = resolve_credentials(&self.account, &mut cache).await?;
                self.oauth_token = cache;
                smtp::send(&self.account, &creds, &raw).await?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Mirror an outgoing message to the Sent folder and return that
    /// folder's id (so the caller resyncs it). Gmail files sent mail on its
    /// own — appending would duplicate it, so there we only resync.
    async fn mirror_to_sent(&mut self, raw: &[u8]) -> Option<i64> {
        // Gmail files sent mail automatically; appending would duplicate it.
        let mut sent_folder_id = None;
        if self.account.provider != "gmail" {
            match self.role_folder("sent", "Sent").await {
                Ok(dest) => {
                    match self.session().await {
                        Ok(session) => {
                            if let Err(e) = session.append(&dest, Some("(\\Seen)"), None, raw).await
                            {
                                tracing::warn!(error = %e, "cannot append to Sent");
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "cannot append to Sent"),
                    }
                    let dest_owned = dest.clone();
                    let account_id = self.account.id.clone();
                    sent_folder_id = self
                        .db
                        .call(move |conn| {
                            use rusqlite::OptionalExtension;
                            conn.query_row(
                                "SELECT id FROM folders WHERE account_id = ?1 AND imap_name = ?2",
                                rusqlite::params![account_id, dest_owned],
                                |r| r.get(0),
                            )
                            .optional()
                        })
                        .await
                        .ok()
                        .flatten();
                }
                Err(e) => tracing::warn!(error = %e, "no Sent folder"),
            }
        } else {
            // Give Gmail a moment to file the copy, then resync the Sent
            // folder so the message shows up right away — otherwise it only
            // appears on the next polling cycle.
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            let account_id = self.account.id.clone();
            sent_folder_id = self
                .db
                .call(move |conn| {
                    use rusqlite::OptionalExtension;
                    conn.query_row(
                        "SELECT id FROM folders WHERE account_id = ?1 AND role = 'sent'",
                        rusqlite::params![account_id],
                        |r| r.get(0),
                    )
                    .optional()
                })
                .await
                .ok()
                .flatten();
        }

        sent_folder_id
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

fn spawn_idle_watcher(account: Account, tx: mpsc::UnboundedSender<SyncCommand>) {
    tauri::async_runtime::spawn(async move {
        let mut oauth_cache: Option<(String, i64)> = None;
        let mut backoff = 5u64;
        loop {
            if tx.is_closed() {
                break;
            }
            match idle_session(&account, &tx, &mut oauth_cache).await {
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
    account: &Account,
    tx: &mpsc::UnboundedSender<SyncCommand>,
    oauth_cache: &mut Option<(String, i64)>,
) -> Result<()> {
    let creds = resolve_credentials(account, oauth_cache).await?;
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

    // Sync once on every (re)connect: mail that arrived while the IDLE
    // connection was down (Gmail rotates them) would otherwise wait for the
    // next push or the poll. This is what keeps new mail near-instant.
    tracing::info!(account = %account.email, "IDLE connected; syncing inbox");
    let _ = tx.send(SyncCommand::SyncInbox);

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
                tracing::info!(account = %account.email, "IDLE new data; syncing inbox");
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
    // Special-use comes through either as imap-proto enum variants
    // (Debug: `All`, `Sent`, …) or as Extension("\\All") strings.
    let has_all =
        attrs_lower.contains("\\all") || attrs_lower.split_whitespace().any(|t| t == "all");
    let by_attr = if has_all {
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
    let stripped = imap_name
        .strip_prefix("[Gmail]/")
        .or_else(|| imap_name.strip_prefix("[Google Mail]/"))
        .unwrap_or(imap_name);
    decode_imap_utf7(stripped)
}

/// Decode RFC 3501 modified UTF-7 mailbox names ("&BBIEMAQ2BD0EPgQ1-" → "Важное").
fn decode_imap_utf7(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        // Find the closing '-'.
        let rest = &s[i + 1..];
        let Some(end) = rest.find('-') else {
            out.push('&'); // malformed; keep as-is
            out.push_str(rest);
            break;
        };
        let b64 = &rest[..end];
        // Skip the consumed section in the outer iterator.
        for _ in 0..=end {
            chars.next();
        }
        if b64.is_empty() {
            out.push('&'); // "&-" is a literal ampersand
            continue;
        }
        // Modified base64: ',' instead of '/', no padding; decodes to UTF-16BE.
        let standard: String = b64
            .chars()
            .map(|c| if c == ',' { '/' } else { c })
            .collect();
        use base64::Engine;
        let engine = base64::engine::GeneralPurpose::new(
            &base64::alphabet::STANDARD,
            base64::engine::GeneralPurposeConfig::new()
                .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
        );
        match engine.decode(&standard) {
            Ok(bytes) if bytes.len() % 2 == 0 => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .collect();
                match String::from_utf16(&units) {
                    Ok(decoded) => out.push_str(&decoded),
                    Err(_) => {
                        out.push('&');
                        out.push_str(b64);
                        out.push('-');
                    }
                }
            }
            _ => {
                out.push('&');
                out.push_str(b64);
                out.push('-');
            }
        }
    }
    out
}

#[cfg(test)]
mod utf7_tests {
    use super::decode_imap_utf7;

    #[test]
    fn decodes_modified_utf7_names() {
        assert_eq!(decode_imap_utf7("INBOX"), "INBOX");
        assert_eq!(decode_imap_utf7("&BBIEMAQ2BD0EPgQ1-"), "Важное");
        assert_eq!(decode_imap_utf7("&BCEENQQ8BEwETw-"), "Семья");
        assert_eq!(decode_imap_utf7("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_imap_utf7("&Jjo-!"), "☺!");
        // malformed input survives untouched
        assert_eq!(decode_imap_utf7("&broken"), "&broken");
    }
}
