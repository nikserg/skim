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
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot, Mutex};

const INBOX_WINDOW: u32 = 500;
const FOLDER_WINDOW: u32 = 200;
const CHUNK: u32 = 100;
/// Headers per FETCH while walking a folder's history backwards.
const BACKFILL_CHUNK: u32 = 200;
/// Chunks of history per folder per pass. Bounds one sweep so a decade-deep
/// mailbox can't hold up new mail; the next poll resumes where this stopped.
const BACKFILL_CHUNKS_PER_PASS: u32 = 10;
// IDLE keeps the inbox instant, so this poll only backfills the slow-changing
// rest (other folders, read state from other devices) — it can run infrequently.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);
const IDLE_REISSUE: std::time::Duration = std::time::Duration::from_secs(25 * 60);
// Ceiling on one whole trip to the server for a body — login, SELECT and the
// FETCH — so a stalled socket can't wedge the worker (and thus every later
// command) indefinitely.
const BODY_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
// One NOOP round-trip, used to tell a live connection from a dead one before the
// user's FETCH goes out. A live socket answers in an RTT, a dropped one errors at
// once, and one blackholed by sleep/wake or a network switch hangs — which is
// what this leash is for. It replaces paying that discovery out of the fetch.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
// Don't probe a connection that was just used: the second message in a thread
// must stay instant. A minute is far below any server's idle timeout and above
// any pause between two clicks.
const PROBE_AFTER_IDLE: std::time::Duration = std::time::Duration::from_secs(60);
// Everything one FetchBody may spend on the wire: probe, login, SELECT, FETCH
// and the one retry. Stays under BODY_FETCH_WAIT so the worker's own error
// always beats the caller's timeout to the reading pane.
const FETCH_REQUEST_BUDGET: std::time::Duration = std::time::Duration::from_secs(100);
// Above this, opening a message stopped feeling instant and is worth a line in
// the on-disk log — a windowed build has no stderr to print to.
const SLOW_FETCH_LOG: std::time::Duration = std::time::Duration::from_secs(3);
// How long a `get_message_body` caller waits for the worker to service its
// request — longer than FETCH_REQUEST_BUDGET so the worker's own error wins, but
// still bounded in case the worker is stuck on some other un-timed op.
const BODY_FETCH_WAIT: std::time::Duration = std::time::Duration::from_secs(120);
// A body worth pulling the moment it lands. Ordinary correspondence and HTML
// newsletters fit; anything larger is attachment-bearing mail, where a short
// wait on open is expected anyway and where writing files to disk for a message
// nobody may ever open is a real cost.
const PREFETCH_MAX_BYTES: i64 = 256 * 1024;
// Bodies per arrival. IDLE fires per message, so a normal batch is one or two;
// the cap only bites after a reconnect that swept up a burst, where the newest
// few are the ones about to be read. A click can queue behind the batch, so
// keep it short.
const PREFETCH_MAX_MESSAGES: usize = 3;
// Everything one arrival's prefetch may spend on the wire, batch included.
// This is the whole risk the feature carries: the batch holds the fetch
// connection, so a click landing mid-batch waits this long at worst — which is
// why it is a budget for the batch and not a leash per message.
const PREFETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub enum SyncCommand {
    SyncAll,
    SyncInbox,
    FetchBody {
        message_pk: i64,
        respond: oneshot::Sender<Result<()>>,
    },
    /// Pull the bodies of messages that just arrived, before they are clicked.
    PrefetchBodies {
        message_pks: Vec<i64>,
    },
    /// Get the fetch connection ready for a click that is probably coming.
    WarmFetch,
    RunOps,
    Stop,
}

#[derive(Clone)]
pub struct SyncHandle {
    pub tx: mpsc::UnboundedSender<SyncCommand>,
    // A separate connection dedicated to interactive body fetches, so opening a
    // message never queues behind a long background sync on the main one.
    pub fetch_tx: mpsc::UnboundedSender<SyncCommand>,
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
        let _ = self.fetch_tx.send(SyncCommand::Stop);
    }
    /// Wake the fetch connection now, so the click it is waiting for doesn't
    /// have to pay for a login. Free when the connection is already warm.
    pub fn warm_fetch(&self) {
        let _ = self.fetch_tx.send(SyncCommand::WarmFetch);
    }
    pub async fn fetch_body(&self, message_pk: i64) -> Result<()> {
        let (respond, rx) = oneshot::channel();
        // Route to the dedicated fetch connection, not the sync worker.
        self.fetch_tx
            .send(SyncCommand::FetchBody {
                message_pk,
                respond,
            })
            .map_err(|_| SkimError::other("sync", "sync engine is not running"))?;
        // The fetch connection is serial too, so bound the wait: a stalled
        // socket must surface as an error, not hang the reading pane forever.
        match tokio::time::timeout(BODY_FETCH_WAIT, rx).await {
            Ok(res) => {
                res.map_err(|_| SkimError::other("sync", "sync engine dropped the request"))?
            }
            Err(_) => Err(SkimError::other("sync", "timed out fetching message body")),
        }
    }
}

struct Engine {
    app: AppHandle,
    db: Db,
    account: Account,
    data_dir: PathBuf,
    session: Option<imap_client::Session>,
    selected: Option<String>,
    // Shared between the sync and body-fetch connections so they never refresh
    // (and, for Microsoft, rotate) the stored OAuth token concurrently.
    oauth_token: Arc<Mutex<Option<(String, i64)>>>,
    /// When this connection last completed a round-trip with the server. Only
    /// the fetch worker reads it: the sync connection talks to its server every
    /// five minutes anyway, so it never sits idle long enough to go stale
    /// unnoticed.
    last_ok: std::time::Instant,
    /// Where to send prefetch requests — the fetch connection, so a sync pass
    /// never waits for them. `None` on the fetch engine itself, which is the
    /// other end of this channel.
    prefetch_tx: Option<mpsc::UnboundedSender<SyncCommand>>,
    /// How long the last login spent resolving credentials. Set inside
    /// [`Engine::session`], which is the only place that can see the wait on
    /// the shared token mutex and the HTTPS refresh behind it.
    last_cred_ms: u128,
}

/// Where one body fetch's time actually went.
///
/// A single total is unattributable: twenty seconds could be a TLS handshake,
/// a token refresh queued behind the other connection, or a genuinely large
/// message. Numbers only — never anything from the message itself.
#[derive(Default, Clone, Copy)]
struct FetchPhases {
    /// Looking up the message's folder and UID — also the wait for the one
    /// SQLite connection, which a sync pass can be holding.
    db_ms: u128,
    /// Shared OAuth mutex plus the token refresh behind it. Zero on a reused
    /// session, and included in `login_ms`.
    cred_ms: u128,
    /// Connect, TLS and authentication. Zero on a reused session.
    login_ms: u128,
    select_ms: u128,
    fetch_ms: u128,
    bytes: usize,
}

impl std::fmt::Display for FetchPhases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "db {}ms, cred {}ms, login {}ms, select {}ms, fetch {}ms, {}KiB",
            self.db_ms,
            self.cred_ms,
            self.login_ms,
            self.select_ms,
            self.fetch_ms,
            self.bytes / 1024
        )
    }
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
    let app_visible = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let (tx, mut rx) = mpsc::unbounded_channel::<SyncCommand>();
    let (fetch_tx, mut fetch_rx) = mpsc::unbounded_channel::<SyncCommand>();
    let handle = SyncHandle {
        tx: tx.clone(),
        fetch_tx: fetch_tx.clone(),
    };

    // One OAuth token cache, shared by every connection this account owns.
    let oauth: Arc<Mutex<Option<(String, i64)>>> = Arc::new(Mutex::new(None));

    spawn_idle_watcher(account.clone(), tx.clone(), oauth.clone());

    // Dedicated connection for interactive body fetches. It only ever handles
    // FetchBody, on its own IMAP session, so opening a message stays instant
    // even while the main connection is mid-way through a long sync.
    {
        let mut fetcher = Engine {
            app: app.clone(),
            db: db.clone(),
            account: account.clone(),
            data_dir: data_dir.clone(),
            session: None,
            selected: None,
            oauth_token: oauth.clone(),
            last_ok: std::time::Instant::now(),
            prefetch_tx: None,
            last_cred_ms: 0,
        };
        tauri::async_runtime::spawn(async move {
            while let Some(cmd) = fetch_rx.recv().await {
                match cmd {
                    SyncCommand::Stop => break,
                    SyncCommand::FetchBody {
                        message_pk,
                        respond,
                    } => {
                        let started = std::time::Instant::now();
                        let probed = fetcher.probe_if_idle().await;
                        let probe_ms = started.elapsed().as_millis();
                        let reused = fetcher.session.is_some();
                        let mut phases = FetchPhases::default();
                        let mut result = fetcher
                            .fetch_body(message_pk, BODY_FETCH_TIMEOUT, &mut phases)
                            .await;
                        let mut retried = false;
                        if let Err(e) = &result {
                            // A cancelled command leaves the response stream
                            // desynced, so the session is finished either way.
                            let worth_retrying = worth_retrying(reused, e);
                            fetcher.reset_session();
                            // The probe makes this rare — it now only catches a
                            // server that dropped us in between. It still has to
                            // fit in what's left of the budget, or the caller's
                            // wait expires and the pane says "couldn't load"
                            // while we are still fetching.
                            let left = FETCH_REQUEST_BUDGET.saturating_sub(started.elapsed());
                            if worth_retrying && !left.is_zero() {
                                tracing::warn!(message_pk, error = %e, "body fetch failed on a reused session, reconnecting");
                                retried = true;
                                phases = FetchPhases::default();
                                result = fetcher.fetch_body(message_pk, left, &mut phases).await;
                                if result.is_err() {
                                    fetcher.reset_session();
                                }
                            }
                        }
                        if let Err(e) = &result {
                            tracing::warn!(message_pk, error = %e, "body fetch failed");
                        }
                        let took = started.elapsed();
                        if took > SLOW_FETCH_LOG {
                            // The whole point of the breakdown: it says whether
                            // the time went into finding a dead connection, into
                            // reconnecting, or into a genuinely large message.
                            crate::append_log(
                                "skim-slow.log",
                                &format!(
                                    "slow body fetch: {}ms total, probe {probe_ms}ms (probed={probed}), \
                                     reused={reused}, retried={retried}, {phases}, outcome={}",
                                    took.as_millis(),
                                    match &result {
                                        Ok(()) => "ok".to_string(),
                                        Err(e) => format!("{} ({})", e.code(), e),
                                    }
                                ),
                            );
                        }
                        let _ = respond.send(result);
                    }
                    SyncCommand::PrefetchBodies { message_pks } => {
                        fetcher.prefetch_bodies(&message_pks).await;
                    }
                    SyncCommand::WarmFetch => {
                        // The same leash a click gets: a warm-up performs
                        // exactly the work that click would have, so it may
                        // never hold this serial worker longer than one.
                        match tokio::time::timeout(BODY_FETCH_TIMEOUT, fetcher.warm()).await {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                tracing::debug!(error = %e, "warming the fetch connection failed")
                            }
                            Err(_) => {
                                // The dropped future left the response stream
                                // desynced, so this session is finished.
                                tracing::debug!("warming the fetch connection timed out");
                                fetcher.reset_session();
                            }
                        }
                    }
                    _ => {}
                }
            }
            fetcher.logout().await;
        });
    }

    tauri::async_runtime::spawn(async move {
        let mut engine = Engine {
            app,
            db,
            account,
            data_dir,
            session: None,
            selected: None,
            oauth_token: oauth,
            last_ok: std::time::Instant::now(),
            prefetch_tx: Some(fetch_tx.clone()),
            last_cred_ms: 0,
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
                        // Body work — interactive fetches, arrival prefetch and
                        // the warm-up that precedes them — runs on the dedicated
                        // fetch connection (see `spawn`), never here: opening a
                        // message must not queue behind a long sync.
                        Some(SyncCommand::FetchBody { .. })
                        | Some(SyncCommand::PrefetchBodies { .. })
                        | Some(SyncCommand::WarmFetch) => {}
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

    // A normal launch or a just-added account: the user is looking at the app,
    // so pay the login now rather than under their first click. Autostart runs
    // with the window hidden — nobody is waiting, and by the time they are the
    // socket would be stale anyway.
    if app_visible {
        handle.warm_fetch();
    }

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

    /// True when the current session answered a NOOP inside [`PROBE_TIMEOUT`].
    ///
    /// A `false` **must** be followed by `reset_session()`: on timeout the NOOP
    /// future is dropped mid-command, so the session's response stream is left
    /// desynced and the next command would read the NOOP's reply.
    async fn session_is_alive(&mut self) -> bool {
        let Some(session) = self.session.as_mut() else {
            return false;
        };
        if noop_probe(session, PROBE_TIMEOUT).await {
            self.last_ok = std::time::Instant::now();
            true
        } else {
            false
        }
    }

    /// Drop the session if it has been quiet long enough that the far end may
    /// have hung up. Returns whether a probe was actually spent.
    ///
    /// Nothing else ever touches the fetch connection, so between two opened
    /// messages it can sit idle long enough for the server to drop it — and we
    /// would only find out when the user clicks. One NOOP settles that in a
    /// round-trip; paying for the discovery out of the user's fetch is what
    /// used to leave the reading pane on "Loading…".
    async fn probe_if_idle(&mut self) -> bool {
        let probed = self.last_ok.elapsed() > PROBE_AFTER_IDLE;
        if probed && !self.session_is_alive().await {
            self.reset_session();
        }
        probed
    }

    /// Get this connection ready for the click that is probably coming: prove
    /// the socket is alive (or replace it) and leave the inbox selected, so the
    /// click itself is one FETCH.
    ///
    /// Free when everything is already warm — no network at all — which is what
    /// lets it hang off a signal as frequent as the window regaining focus.
    async fn warm(&mut self) -> Result<()> {
        self.probe_if_idle().await;
        let (_, imap_name) = self.inbox_folder().await?;
        if self.session.is_some() && self.selected.as_deref() == Some(imap_name.as_str()) {
            // Already warm. Return without touching `last_ok`: this path proved
            // nothing, and pushing it forward would suppress the probe that
            // guards the user's next click.
            return Ok(());
        }
        // Logging in — and refreshing the OAuth token — happens here, in the
        // background, instead of under the click.
        self.ensure_selected(&imap_name).await?;
        // Both the login and the SELECT are round-trips, so the connection is
        // demonstrably alive as of now.
        self.last_ok = std::time::Instant::now();
        Ok(())
    }

    /// Pull the bodies of messages that just landed, so opening one is a local
    /// read. Best-effort: nothing here is worth reporting to the user, who has
    /// not asked for anything yet.
    async fn prefetch_bodies(&mut self, message_pks: &[i64]) {
        let targets = match self.prefetch_candidates(message_pks).await {
            Ok(t) if !t.is_empty() => t,
            _ => return,
        };
        self.probe_if_idle().await;
        let started = std::time::Instant::now();
        let (mut done, mut bytes) = (0usize, 0usize);
        for pk in &targets {
            // Spend what is left of the batch's budget, never more: a click can
            // be queued behind this, and it is not why the user is here.
            let left = PREFETCH_TIMEOUT.saturating_sub(started.elapsed());
            if left.is_zero() {
                break;
            }
            let mut phases = FetchPhases::default();
            if let Err(e) = self.fetch_body(*pk, left, &mut phases).await {
                tracing::debug!(error = %e, "prefetching a body failed");
                // A cancelled command leaves the response stream desynced, and
                // the click this was meant to serve must not inherit that.
                self.reset_session();
                break;
            }
            done += 1;
            bytes += phases.bytes;
        }
        let took = started.elapsed();
        if took > SLOW_FETCH_LOG {
            crate::append_log(
                "skim-slow.log",
                &format!(
                    "slow prefetch: {}/{} bodies, {}ms, {}KiB",
                    done,
                    targets.len(),
                    took.as_millis(),
                    bytes / 1024
                ),
            );
        }
    }

    /// Which of the just-arrived messages still need a body and are worth one.
    async fn prefetch_candidates(&self, message_pks: &[i64]) -> Result<Vec<i64>> {
        let pks = message_pks.to_vec();
        let rows = self
            .db
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, uid, size FROM messages WHERE id = ?1 AND body_state = 0",
                )?;
                let mut rows = Vec::new();
                for pk in pks {
                    if let Some(row) = stmt
                        .query_map(rusqlite::params![pk], |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, u32>(1)?,
                                r.get::<_, Option<i64>>(2)?,
                            ))
                        })?
                        .next()
                        .transpose()?
                    {
                        rows.push(row);
                    }
                }
                Ok(rows)
            })
            .await?;
        Ok(prefetch_targets(&rows))
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
        // Repainted after the attempt either way: on success from fresh
        // counts, on failure from the cache — nothing better is coming.
        crate::badge::refresh(&self.app).await;
    }

    /// This account's inbox, as `(folder id, IMAP name)`.
    async fn inbox_folder(&self) -> Result<(i64, String)> {
        let account_id = self.account.id.clone();
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT id, imap_name FROM folders WHERE account_id = ?1 AND role = 'inbox'",
                    rusqlite::params![account_id],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )
            })
            .await
    }

    async fn sync_inbox(&mut self) {
        let inbox = self.inbox_folder().await;
        if let Ok((folder_id, imap_name)) = inbox {
            if let Err(e) = self.sync_folder(folder_id, &imap_name).await {
                tracing::warn!(error = %e, "inbox sync failed");
                self.reset_session();
            }
        }
        // This is the IDLE-driven path and the first sync after launch: the
        // badge must follow it, not wait for the next full sweep. Painted on
        // failure too — that is the offline fallback to the cached count.
        crate::badge::refresh(&self.app).await;
    }

    async fn logout(&mut self) {
        if let Some(mut s) = self.session.take() {
            let _ = s.logout().await;
        }
    }

    async fn session(&mut self) -> Result<&mut imap_client::Session> {
        if self.session.is_none() {
            // Hold the shared token cache only across the (possibly refreshing)
            // credential resolve, so the two connections can't refresh at once.
            let creds = {
                let started = std::time::Instant::now();
                let cache = self.oauth_token.clone();
                let mut cache = cache.lock().await;
                let creds = resolve_credentials(&self.account, &mut cache).await;
                // Recorded even on the error path: a slow *failing* refresh is
                // exactly the case worth seeing in the log.
                self.last_cred_ms = started.elapsed().as_millis();
                creds?
            };
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
                        // An unchanged folder still needs its history walked —
                        // backfill reaches into the past, which no STATUS
                        // snapshot can tell us anything about.
                        if self.status_matches(folder.id, &st).await
                            && self.backfill_state(folder.id).await?.0
                        {
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
        let mut delimiter: Option<String> = None;
        {
            let mut stream = session.list(None, Some("*")).await.map_err(imap_err)?;
            while let Some(item) = stream.next().await {
                let name = item.map_err(imap_err)?;
                let attrs: Vec<String> =
                    name.attributes().iter().map(|a| format!("{a:?}")).collect();
                if delimiter.is_none() {
                    delimiter = name.delimiter().map(str::to_string);
                }
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

        // The hierarchy separator is a property of the server, not of one
        // mailbox, so learning it once from LIST is enough. It is only needed
        // when creating a folder from a name the user typed.
        if let Some(delim) = delimiter {
            let account_id = self.account.id.clone();
            let _ = self
                .db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE accounts SET folder_delimiter = ?2 WHERE id = ?1",
                        rusqlite::params![account_id, delim],
                    )
                    .map(|_| ())
                })
                .await;
        }

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
                    // The folder was wiped, so the history walk restarts too.
                    "UPDATE folders
                        SET uidvalidity = ?2, last_seen_uid = 0,
                            backfill_done = 0, backfill_seq_floor = NULL
                      WHERE id = ?1",
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
                // Only mail the server still holds unread is news. A message
                // read on another device before this one ever saw it (a
                // machine that was off all day, a laptop waking from sleep)
                // must not toast — the user has already dealt with it.
                let unread: Vec<i64> = inserted
                    .iter()
                    .filter(|(_, is_read)| !is_read)
                    .map(|(pk, _)| *pk)
                    .collect();
                if !unread.is_empty() {
                    let _ = self.app.emit("mail:new", json!({ "count": unread.len() }));
                    crate::notify::notify_new_mail(&self.app, &self.db, &unread).await;
                }
                // Pull the bodies before they are clicked — read ones too, they
                // are just as likely to be opened. Fire-and-forget onto the
                // fetch connection: a just-arrived message is guaranteed to be
                // uncached, and the notification the user is about to tap must
                // not wait for this — nor must the rest of the pass.
                if let Some(tx) = &self.prefetch_tx {
                    let _ = tx.send(SyncCommand::PrefetchBodies {
                        message_pks: inserted.iter().map(|(pk, _)| *pk).collect(),
                    });
                }
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

        // Older-than-the-window history, a slice at a time. Last, so a failure
        // here can't cost us the new mail this pass already recorded.
        changed |= self.backfill_folder(folder_id, exists).await?;

        if changed {
            let _ = self
                .app
                .emit("mail:updated", json!({ "folderId": folder_id }));
        }
        Ok(changed)
    }

    /// Walk a folder's history backwards, a bounded number of chunks per pass,
    /// until its very first message has been cached.
    ///
    /// The first sync of a folder only takes the newest window, and every sync
    /// after that fetches `last_seen_uid+1:*` — forward only. Without this,
    /// mail older than that initial window is never downloaded, so it is
    /// invisible to search and the assistant for as long as the account exists.
    ///
    /// Runs against the already-selected mailbox, so sequence numbers hold
    /// still for the whole descent. `exists` is the server's message count from
    /// that SELECT.
    async fn backfill_folder(&mut self, folder_id: i64, exists: u32) -> Result<bool> {
        let (done, floor) = self.backfill_state(folder_id).await?;
        if done {
            return Ok(false);
        }
        if exists == 0 {
            self.set_backfill_done(folder_id).await?;
            return Ok(false);
        }

        // Where the un-fetched history ends. Resume from the stored floor, or
        // on the first pass from just below the newest cached run — the cache
        // is always a newest-first suffix of the folder, so that's the boundary.
        let mut floor = match floor {
            Some(f) => f,
            None => initial_floor(exists, self.cached_count(folder_id).await?),
        };

        let mut changed = false;
        for _ in 0..BACKFILL_CHUNKS_PER_PASS {
            let Some((low, high)) = backfill_chunk(floor) else {
                break;
            };
            let inserted = self
                .fetch_headers(folder_id, &format!("{low}:{high}"), false, 0)
                .await?;
            changed |= !inserted.is_empty();
            // Step down whatever came back: a chunk of already-cached messages
            // still means that stretch of history is covered. Persisting each
            // step is what makes the walk resumable across passes and restarts.
            floor = low;
            self.set_backfill_floor(folder_id, floor).await?;
            let _ = self.app.emit(
                "sync:progress",
                json!({ "folderId": folder_id, "done": exists - low + 1, "total": exists }),
            );
        }

        if floor <= 1 {
            self.set_backfill_done(folder_id).await?;
            tracing::info!(folder_id, exists, "history backfill complete");
        }
        Ok(changed)
    }

    /// `(backfill_done, backfill_seq_floor)` for a folder.
    async fn backfill_state(&self, folder_id: i64) -> Result<(bool, Option<u32>)> {
        self.db
            .call(move |conn| {
                conn.query_row(
                    "SELECT backfill_done, backfill_seq_floor FROM folders WHERE id = ?1",
                    rusqlite::params![folder_id],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)? != 0,
                            r.get::<_, Option<i64>>(1)?.map(|v| v.max(0) as u32),
                        ))
                    },
                )
            })
            .await
    }

    async fn cached_count(&self, folder_id: i64) -> Result<u32> {
        let n: i64 = self
            .db
            .call(move |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE folder_id = ?1",
                    rusqlite::params![folder_id],
                    |r| r.get(0),
                )
            })
            .await?;
        Ok(n.max(0) as u32)
    }

    async fn set_backfill_floor(&self, folder_id: i64, floor: u32) -> Result<()> {
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE folders SET backfill_seq_floor = ?2 WHERE id = ?1",
                    rusqlite::params![folder_id, floor as i64],
                )
                .map(|_| ())
            })
            .await
    }

    async fn set_backfill_done(&self, folder_id: i64) -> Result<()> {
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE folders SET backfill_done = 1 WHERE id = ?1",
                    rusqlite::params![folder_id],
                )
                .map(|_| ())
            })
            .await
    }

    /// Fetch headers for `set` and insert the ones not yet cached. Returns
    /// `(message pk, is_read)` for each newly inserted row — the server-side
    /// `\Seen` at the moment we first saw it.
    async fn fetch_headers(
        &mut self,
        folder_id: i64,
        set: &str,
        by_uid: bool,
        above_uid: i64,
    ) -> Result<Vec<(i64, bool)>> {
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
                        inserted.push((pk, msg.is_read));
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

    /// `timeout` bounds every step that touches the network — logging in,
    /// selecting the folder and the FETCH.
    ///
    /// Any `Err` may have cancelled a command mid-flight, which leaves the
    /// response stream desynced: callers must `reset_session()` before reusing
    /// this engine.
    async fn fetch_body(
        &mut self,
        message_pk: i64,
        timeout: std::time::Duration,
        phases: &mut FetchPhases,
    ) -> Result<()> {
        let started = std::time::Instant::now();
        let coords: Option<(String, u32, i64)> = self
            .db
            .call(move |conn| {
                // `body_state` via the shared helper, so the guard below agrees
                // with the one the command layer used to decide to call us.
                let state = bodies::body_state(conn, message_pk)?;
                conn.query_row(
                    "SELECT f.imap_name, m.uid
                     FROM messages m JOIN folders f ON f.id = m.folder_id
                     WHERE m.id = ?1",
                    rusqlite::params![message_pk],
                    |r| Ok((r.get(0)?, r.get(1)?, state.unwrap_or(0))),
                )
                .map(Some)
            })
            .await
            .unwrap_or(None);
        // Also the wait for the one SQLite connection, which a sync pass can be
        // holding — invisible in a total, and not something the leash covers.
        phases.db_ms = started.elapsed().as_millis();
        let Some((imap_name, uid, body_state)) = coords else {
            return Err(SkimError::other("mail", "message no longer exists"));
        };
        if body_state == 1 {
            return Ok(());
        }

        // One leash over the whole trip. It used to cover only the FETCH, with
        // `ensure_selected` — and the login inside it, which can wait on the
        // shared OAuth mutex across an HTTPS token refresh — running in front of
        // it unbounded. A stalled socket there held the reading pane for as long
        // as the OS kept the connection open.
        let raw: Option<Vec<u8>> = tokio::time::timeout(timeout, async {
            // Log in first, then SELECT, so the two are timed apart; on a warm
            // connection both are no-ops. `ensure_selected` would otherwise do
            // the login inside itself and hide it in `select_ms`.
            let at = std::time::Instant::now();
            self.last_cred_ms = 0;
            self.session().await?;
            phases.login_ms = at.elapsed().as_millis();
            phases.cred_ms = self.last_cred_ms;

            let at = std::time::Instant::now();
            self.ensure_selected(&imap_name).await?;
            phases.select_ms = at.elapsed().as_millis();

            let at = std::time::Instant::now();
            let session = self.session().await?;
            let mut stream = session
                .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")
                .await
                .map_err(imap_err)?;
            let mut raw: Option<Vec<u8>> = None;
            while let Some(item) = stream.next().await {
                let f = item.map_err(imap_err)?;
                // Servers weave *unsolicited* flag-only FETCH responses for the
                // same message into the reply. They carry no body, and letting
                // one land after the real answer is how a healthy connection
                // could report "server returned no message body".
                if f.uid == Some(uid) {
                    if let Some(body) = f.body() {
                        raw = Some(body.to_vec());
                    }
                }
            }
            phases.fetch_ms = at.elapsed().as_millis();
            Ok::<_, SkimError>(raw)
        })
        .await
        .map_err(|_| SkimError::other("network", "timed out fetching message body"))??;
        self.last_ok = std::time::Instant::now();
        let Some(raw) = raw else {
            return Err(SkimError::other("mail", "server returned no message body"));
        };
        phases.bytes = raw.len();

        let parsed = parse::parse_body(&raw);

        // Attachments go to the on-disk cache, keyed by message pk.
        let dir = self
            .data_dir
            .join("attachments")
            .join(message_pk.to_string());
        let mut stored = Vec::new();
        // A refetch replaces the attachment rows wholesale, so whatever the last
        // parse left on disk is unreachable from here on — drop it rather than
        // keep paying for it.
        let _ = std::fs::remove_dir_all(&dir);
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

        // Messages synced before migration 0008 have no auth columns; the raw
        // payload here starts with the full header block, so backfill them the
        // first time the body is fetched. NULL-guarded: never overwrite what
        // header sync already stored.
        let hdr = parse::parse_headers("", 0, 0, &raw, None, None, false, false, false);
        let (reply_to, spf, dkim, dmarc) = (
            hdr.reply_to_addr,
            hdr.auth_spf,
            hdr.auth_dkim,
            hdr.auth_dmarc,
        );

        self.db
            .call(move |conn| {
                bodies::set_body(
                    conn,
                    message_pk,
                    html.as_deref(),
                    text.as_deref(),
                    &snippet,
                    &stored,
                )?;
                conn.execute(
                    "UPDATE messages SET
                         reply_to_addr = COALESCE(reply_to_addr, ?2),
                         auth_spf      = COALESCE(auth_spf, ?3),
                         auth_dkim     = COALESCE(auth_dkim, ?4),
                         auth_dmarc    = COALESCE(auth_dmarc, ?5)
                     WHERE id = ?1",
                    rusqlite::params![message_pk, reply_to, spf, dkim, dmarc],
                )?;
                Ok(())
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
                Ok(folder_ids) => {
                    affected.extend(folder_ids);
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
                                // Same for a folder we optimistically added to
                                // the sidebar for a move that never landed:
                                // discover_folders only ever inserts, so an
                                // empty phantom would stay there forever.
                                let orphan = if kind == "move"
                                    && parsed["createDest"].as_bool().unwrap_or(false)
                                {
                                    parsed["destFolderId"].as_i64()
                                } else {
                                    None
                                };
                                // A rename that never reached the server: put the
                                // old name back, or discover_folders would add a
                                // second row for the mailbox that still exists
                                // under it.
                                if kind == "rename_folder" {
                                    let undo = (
                                        parsed["folderId"].as_i64(),
                                        parsed["imapName"].as_str().map(str::to_string),
                                        parsed["displayName"].as_str().map(str::to_string),
                                    );
                                    if let (Some(id), Some(imap), Some(display)) = undo {
                                        let _ = self
                                            .db
                                            .call(move |conn| {
                                                conn.execute(
                                                    "UPDATE folders
                                                        SET imap_name = ?2, display_name = ?3
                                                      WHERE id = ?1",
                                                    rusqlite::params![id, imap, display],
                                                )
                                                .map(|_| ())
                                            })
                                            .await;
                                        let _ = self.app.emit("folders:updated", json!({}));
                                    }
                                }
                                if let Some(folder_id) = orphan {
                                    let _ = self
                                        .db
                                        .call(move |conn| {
                                            conn.execute(
                                                "DELETE FROM folders WHERE id = ?1
                                                 AND NOT EXISTS (SELECT 1 FROM messages
                                                                 WHERE folder_id = ?1)",
                                                rusqlite::params![folder_id],
                                            )
                                            .map(|_| ())
                                        })
                                        .await;
                                    let _ = self.app.emit("folders:updated", json!({}));
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

    /// Execute one queued op. Returns the folders to resync afterwards — the
    /// source, plus the destination when the op moved mail between two of them.
    async fn execute_op(&mut self, kind: &str, payload: &serde_json::Value) -> Result<Vec<i64>> {
        // The self-contained ops each resync at most one folder of their own.
        let one = |f: Option<i64>| f.into_iter().collect();
        if kind == "send" {
            return self.execute_send(payload).await.map(one);
        }
        if kind == "rsvp" {
            return self.execute_rsvp(payload).await.map(one);
        }
        if kind == "unsubscribe" {
            return self.execute_unsubscribe(payload).await.map(one);
        }
        if kind == "save_draft" {
            return self.execute_save_draft(payload).await.map(one);
        }
        // Folder-level ops carry no UIDs, so they short-circuit before the
        // message coordinates are read.
        if kind == "rename_folder" {
            return self.execute_rename_folder(payload).await;
        }
        if kind == "delete_folder" {
            return self.execute_delete_folder(payload).await;
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
            return Ok(Vec::new());
        }
        self.ensure_selected(&imap_name).await?;

        let mut affected: Vec<i64> = folder_id.into_iter().collect();

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
                for set in uid_sets(&uids) {
                    let session = self.session().await?;
                    let mut stream = session
                        .uid_store(&set, format!("{sign}FLAGS ({flag})"))
                        .await
                        .map_err(imap_err)?;
                    while let Some(item) = stream.next().await {
                        item.map_err(imap_err)?;
                    }
                }
            }
            "archive" => {
                let is_gmail_inbox =
                    self.account.provider == "gmail" && imap_name.eq_ignore_ascii_case("INBOX");
                if is_gmail_inbox {
                    // Gmail archive = remove the INBOX label; the message
                    // stays in All Mail.
                    self.delete_and_expunge(&uids).await?;
                } else {
                    let dest = self.role_folder("archive", "Archive").await?;
                    self.move_uids(&uids, &dest).await?;
                }
            }
            "delete" => {
                let dest = self.role_folder("trash", "Trash").await.ok();
                match dest {
                    Some(dest) if !dest.eq_ignore_ascii_case(&imap_name) => {
                        self.move_uids(&uids, &dest).await?;
                    }
                    // Already in trash (or no trash folder): permanent delete.
                    _ => self.delete_and_expunge(&uids).await?,
                }
            }
            "junk" => {
                let dest = self.role_folder("junk", "Junk").await?;
                // Already in the junk folder: nothing to move.
                if !dest.eq_ignore_ascii_case(&imap_name) {
                    self.move_uids(&uids, &dest).await?;
                }
            }
            // File into a folder the user picked. Unlike archive/delete/junk the
            // destination travels in the payload rather than being resolved from
            // a role here, so the op stays self-describing however long it waits
            // in the queue.
            "move" => {
                let dest = payload["destImapName"].as_str().unwrap_or_default();
                // Nothing to do without a destination, or when the mail is
                // already there.
                if !dest.is_empty() && !dest.eq_ignore_ascii_case(&imap_name) {
                    if payload["createDest"].as_bool().unwrap_or(false) {
                        let session = self.session().await?;
                        // ALREADYEXISTS is the normal case on a retry, or when
                        // another client got there first — and a name the server
                        // truly rejects fails loudly on the move right below.
                        if let Err(e) = session.create(dest).await {
                            tracing::debug!(error = %e, folder = %dest, "CREATE failed; moving anyway");
                        }
                    }
                    self.move_uids(&uids, dest).await?;
                    if let Some(dest_folder_id) = payload["destFolderId"].as_i64() {
                        affected.push(dest_folder_id);
                    }
                }
            }
            other => {
                return Err(SkimError::other("ops", format!("unknown op kind: {other}")));
            }
        }
        Ok(affected)
    }

    /// True when the server still has this mailbox. Used to tell "the rename
    /// hasn't happened yet" from "it happened and we missed the reply".
    async fn mailbox_exists(&mut self, imap_name: &str) -> bool {
        self.probe_status(imap_name).await.is_ok()
    }

    /// Rename a mailbox. The local row was renamed at enqueue time, so the
    /// payload carries the old name — that is still what the server calls it.
    async fn execute_rename_folder(&mut self, payload: &serde_json::Value) -> Result<Vec<i64>> {
        let from = payload["imapName"].as_str().unwrap_or_default().to_string();
        let to = payload["destImapName"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let folder_id: Vec<i64> = payload["folderId"].as_i64().into_iter().collect();
        if from.is_empty() || to.is_empty() || from == to {
            return Ok(Vec::new());
        }
        // A server may refuse to rename the mailbox it currently has selected,
        // and STATUS is illegal on it too.
        self.ensure_selected("INBOX").await?;
        // An earlier attempt may have succeeded with its reply lost to a dropped
        // connection; renaming again would fail on a name that no longer exists.
        if !self.mailbox_exists(&from).await {
            return Ok(folder_id);
        }
        let session = self.session().await?;
        session.rename(&from, &to).await.map_err(imap_err)?;
        Ok(folder_id)
    }

    /// Delete a mailbox, but only while the server agrees it is empty: on
    /// everything except Gmail, DELETE takes the mail inside with it, and mail
    /// can arrive between the user's click and this op draining.
    async fn execute_delete_folder(&mut self, payload: &serde_json::Value) -> Result<Vec<i64>> {
        let name = payload["imapName"].as_str().unwrap_or_default().to_string();
        if name.is_empty() {
            return Ok(Vec::new());
        }
        self.ensure_selected("INBOX").await?;
        match self.probe_status(&name).await {
            Ok(st) if st.exists > 0 => {
                return Err(SkimError::other(
                    "folder",
                    format!("{name} is no longer empty"),
                ));
            }
            Ok(_) => {}
            // Already gone — an earlier attempt landed, or someone else deleted it.
            Err(_) => return Ok(Vec::new()),
        }
        let session = self.session().await?;
        session.delete(&name).await.map_err(imap_err)?;
        Ok(Vec::new())
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

        let creds = {
            let cache = self.oauth_token.clone();
            let mut cache = cache.lock().await;
            resolve_credentials(&self.account, &mut cache).await?
        };
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
            self.delete_and_expunge(&old_uids).await?;
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
            self.delete_and_expunge(&uids).await?;
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

        let creds = {
            let cache = self.oauth_token.clone();
            let mut cache = cache.lock().await;
            resolve_credentials(&self.account, &mut cache).await?
        };
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

                let creds = {
                    let cache = self.oauth_token.clone();
                    let mut cache = cache.lock().await;
                    resolve_credentials(&self.account, &mut cache).await?
                };
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

    async fn delete_and_expunge(&mut self, uids: &[u32]) -> Result<()> {
        for set in uid_sets(uids) {
            self.delete_and_expunge_set(&set).await?;
        }
        Ok(())
    }

    async fn delete_and_expunge_set(&mut self, uid_set: &str) -> Result<()> {
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

    async fn move_uids(&mut self, uids: &[u32], dest: &str) -> Result<()> {
        for set in uid_sets(uids) {
            self.move_uids_set(&set, dest).await?;
        }
        Ok(())
    }

    async fn move_uids_set(&mut self, uid_set: &str, dest: &str) -> Result<()> {
        let session = self.session().await?;
        match session.uid_mv(uid_set, dest).await {
            Ok(()) => Ok(()),
            Err(e) => {
                tracing::debug!(error = %e, "UID MOVE unsupported; using COPY+DELETE");
                // async-imap quotes the mailbox for UID MOVE but interpolates it
                // raw for UID COPY, so a destination with a space ("Deleted
                // Items", any folder the user picked) would produce a malformed
                // command on this path. Quote it ourselves.
                session
                    .uid_copy(uid_set, quote_mailbox(dest))
                    .await
                    .map_err(imap_err)?;
                self.delete_and_expunge_set(uid_set).await
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

fn spawn_idle_watcher(
    account: Account,
    tx: mpsc::UnboundedSender<SyncCommand>,
    oauth_token: Arc<Mutex<Option<(String, i64)>>>,
) {
    tauri::async_runtime::spawn(async move {
        let mut backoff = 5u64;
        loop {
            if tx.is_closed() {
                break;
            }
            match idle_session(&account, &tx, &oauth_token).await {
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
    oauth_token: &Arc<Mutex<Option<(String, i64)>>>,
) -> Result<()> {
    // Same lock discipline as `Engine::session`: Microsoft rotates the refresh
    // token on every use and `resolve_credentials` persists the new one, so two
    // connections reconnecting at once (which is exactly what a wake from sleep
    // looks like) must not both spend the same one.
    let creds = {
        let mut cache = oauth_token.lock().await;
        resolve_credentials(account, &mut cache).await?
    };
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

/// The next slice of history to fetch, walking down from `floor` — the lowest
/// sequence number already covered. `None` once the folder's first message is
/// covered and there is nothing older left.
fn backfill_chunk(floor: u32) -> Option<(u32, u32)> {
    if floor <= 1 {
        return None;
    }
    let high = floor - 1;
    let low = high.saturating_sub(BACKFILL_CHUNK - 1).max(1);
    Some((low, high))
}

/// Where to start walking when a folder has no stored floor yet. The cache is a
/// newest-first suffix of the folder, so the `cached` newest messages occupy the
/// top of the sequence range and history resumes just below them.
fn initial_floor(exists: u32, cached: u32) -> u32 {
    exists.saturating_sub(cached).saturating_add(1)
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

/// One NOOP on a short leash: does this session still have a server on the
/// other end? Generic over the transport, and taking its own leash, so it can
/// be tested without TLS and without waiting out [`PROBE_TIMEOUT`].
async fn noop_probe<T>(session: &mut async_imap::Session<T>, leash: std::time::Duration) -> bool
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    matches!(
        tokio::time::timeout(leash, session.noop()).await,
        Ok(Ok(()))
    )
}

/// Errors that mean "this connection is no good", as opposed to "this message
/// is no good" (`mail`) or "this machine is" (`db`, `io`, `internal`). Only the
/// former earns a reconnect: logging in again to re-ask a question that was
/// already answered just spends the user's time on the same answer.
fn is_connection_error(e: &SkimError) -> bool {
    !matches!(e.code(), "mail" | "db" | "io" | "internal")
}

/// Whether a failed body fetch is worth one reconnect.
///
/// Beyond the outright connection errors, a *reused* session's `mail` answer is
/// not trustworthy either: it may have been dead or left desynced by an earlier
/// cancelled command, in which case "no body" is the previous conversation
/// talking rather than the server. A fresh session's answer is final — and if
/// the message really is gone, the next flag reconciliation drops the row.
fn worth_retrying(reused: bool, e: &SkimError) -> bool {
    reused && (is_connection_error(e) || e.code() == "mail")
}

/// Which just-arrived messages are worth pulling before they are clicked:
/// newest first, small enough to be cheap, and only where the server told us
/// the size — a missing RFC822.SIZE means an unbounded download, and this runs
/// on the connection the user's next click needs.
///
/// Takes `(message pk, uid, size)` as stored; returns pks in fetch order.
fn prefetch_targets(candidates: &[(i64, u32, Option<i64>)]) -> Vec<i64> {
    let mut worth: Vec<&(i64, u32, Option<i64>)> = candidates
        .iter()
        .filter(|(_, _, size)| size.is_some_and(|s| s > 0 && s <= PREFETCH_MAX_BYTES))
        .collect();
    // UID order is arrival order, so the newest is the one about to be read.
    worth.sort_unstable_by_key(|(_, uid, _)| std::cmp::Reverse(*uid));
    worth.truncate(PREFETCH_MAX_MESSAGES);
    worth.iter().map(|(pk, _, _)| *pk).collect()
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

/// Longest sequence-set we will put on the wire. IMAP command lines are bounded
/// — commonly around 8 KB — and a bulk selection can run to thousands of
/// messages, so one op may have to issue several commands.
const MAX_UID_SET_LEN: usize = 900;

/// Turn UIDs into IMAP sequence-sets. Consecutive UIDs collapse into `a:b`
/// ranges, which is the common case for a bulk selection, and the result is
/// split so no single set can overflow the server's command-line limit.
fn uid_sets(uids: &[u32]) -> Vec<String> {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
        i += 1;
    }

    let mut sets: Vec<String> = Vec::new();
    let mut cur = String::new();
    for part in parts {
        if !cur.is_empty() && cur.len() + 1 + part.len() > MAX_UID_SET_LEN {
            sets.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(',');
        }
        cur.push_str(&part);
    }
    if !cur.is_empty() {
        sets.push(cur);
    }
    sets
}

/// Quote a mailbox name as an IMAP quoted-string, mirroring what async-imap
/// does internally for the commands that bother to.
fn quote_mailbox(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for c in name.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Encode a mailbox name as RFC 3501 modified UTF-7 ("Важное" → "&BBIEMAQ2BD0EPgQ1-").
/// The inverse of [`decode_imap_utf7`]: needed for CREATE, since a name the user
/// typed can hold anything their locale allows.
pub fn encode_imap_utf7(s: &str) -> String {
    use base64::Engine;
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new().with_encode_padding(false),
    );
    let mut out = String::with_capacity(s.len());
    let mut run: Vec<u16> = Vec::new();
    // Flush the pending non-ASCII run as one "&<modified base64>-" section.
    let flush = |run: &mut Vec<u16>, out: &mut String| {
        if run.is_empty() {
            return;
        }
        let bytes: Vec<u8> = run.iter().flat_map(|u| u.to_be_bytes()).collect();
        out.push('&');
        out.push_str(&engine.encode(&bytes).replace('/', ","));
        out.push('-');
        run.clear();
    };
    for c in s.chars() {
        // Printable US-ASCII goes through verbatim; '&' is escaped as "&-".
        if ('\u{20}'..='\u{7e}').contains(&c) {
            flush(&mut run, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut buf = [0u16; 2];
            run.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut run, &mut out);
    out
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
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .copied()
                    .map(u16::from_be_bytes)
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
mod backfill_tests {
    use super::{backfill_chunk, initial_floor, BACKFILL_CHUNK};

    #[test]
    fn starts_just_below_the_newest_cached_run() {
        // The reported case: an inbox of 609 messages, all of them cached by
        // the initial window plus new arrivals, on a server holding 5000.
        assert_eq!(initial_floor(5000, 609), 4392);
        // Nothing cached yet — start at the very top.
        assert_eq!(initial_floor(5000, 0), 5001);
        // Everything cached — nothing older to walk to.
        assert_eq!(initial_floor(609, 609), 1);
        // A cache larger than the server's count (expunges we haven't noticed)
        // must not wrap around into a huge floor.
        assert_eq!(initial_floor(100, 250), 1);
    }

    #[test]
    fn walk_stops_at_the_first_message() {
        assert_eq!(backfill_chunk(1), None);
        assert_eq!(backfill_chunk(0), None);
        // The floor is exclusive: the chunk sits strictly below it.
        assert_eq!(backfill_chunk(2), Some((1, 1)));
    }

    #[test]
    fn walk_covers_every_sequence_number_exactly_once() {
        let mut floor = initial_floor(1000, 40);
        assert_eq!(floor, 961);
        let mut covered = Vec::new();
        while let Some((low, high)) = backfill_chunk(floor) {
            assert!(low <= high);
            covered.push((low, high));
            floor = low;
        }
        // Contiguous, descending, no gaps and no overlap.
        for w in covered.windows(2) {
            assert_eq!(w[0].0, w[1].1 + 1);
        }
        assert_eq!(covered.first().unwrap().1, 960);
        assert_eq!(covered.last().unwrap().0, 1);
        assert!(covered.iter().all(|(l, h)| h - l < BACKFILL_CHUNK));
    }
}

#[cfg(test)]
mod fetch_tests {
    use super::{
        is_connection_error, noop_probe, prefetch_targets, worth_retrying, BODY_FETCH_TIMEOUT,
        BODY_FETCH_WAIT, FETCH_REQUEST_BUDGET, POLL_INTERVAL, PREFETCH_MAX_BYTES,
        PREFETCH_MAX_MESSAGES, PREFETCH_TIMEOUT, PROBE_AFTER_IDLE, PROBE_TIMEOUT,
    };
    use crate::error::SkimError;
    use crate::mail::oauth;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// The bug this fix is about was constants that stopped composing: the
    /// caller timed out while the worker was still usefully working, or the
    /// worker spent the user's time discovering what a probe answers in an RTT.
    #[test]
    fn the_worker_always_reports_before_the_caller_gives_up() {
        assert!(PROBE_TIMEOUT < BODY_FETCH_TIMEOUT);
        assert!(PROBE_TIMEOUT + BODY_FETCH_TIMEOUT < FETCH_REQUEST_BUDGET);
        assert!(FETCH_REQUEST_BUDGET < BODY_FETCH_WAIT);
        // A probe that fired more often than the reading pause it guards would
        // cost a round-trip on every second message in a thread.
        assert!(PROBE_AFTER_IDLE > PROBE_TIMEOUT);
        // A prefetch batch holds the fetch connection, so a click can queue
        // behind it — and the next poll must not lap the previous batch.
        assert!(PREFETCH_TIMEOUT < POLL_INTERVAL);
        // A token refresh runs inside the body-fetch leash; it has to give up
        // early enough to leave room for the connect and FETCH it precedes.
        assert!(oauth::HTTP_TIMEOUT < BODY_FETCH_TIMEOUT);
    }

    /// States the wire cost of one arrival, so raising either cap has to be a
    /// deliberate decision rather than a quiet one.
    #[test]
    fn one_prefetch_batch_stays_under_a_megabyte() {
        assert!(PREFETCH_MAX_MESSAGES as i64 * PREFETCH_MAX_BYTES <= 1024 * 1024);
    }

    #[test]
    fn prefetch_skips_what_it_cannot_bound() {
        let candidates = [
            (1, 10, Some(1024)),
            // No RFC822.SIZE: fetching it is an unbounded download on the
            // connection the user's next click needs.
            (2, 11, None),
            (3, 12, Some(0)),
            (4, 13, Some(PREFETCH_MAX_BYTES + 1)),
            (5, 14, Some(PREFETCH_MAX_BYTES)),
        ];
        assert_eq!(prefetch_targets(&candidates), vec![5, 1]);
    }

    #[test]
    fn prefetch_takes_the_newest_first() {
        let candidates: Vec<(i64, u32, Option<i64>)> =
            (1..=8).map(|i| (i, i as u32 + 100, Some(2048))).collect();
        let picked = prefetch_targets(&candidates);
        assert_eq!(picked.len(), PREFETCH_MAX_MESSAGES);
        assert_eq!(picked, vec![8, 7, 6]);
    }

    #[test]
    fn an_empty_body_from_a_reused_session_earns_one_fresh_one() {
        let no_body = SkimError::other("mail", "server returned no message body");
        // A reused session may have been dead or desynced, so its answer about
        // the message is the old conversation talking.
        assert!(worth_retrying(true, &no_body));
        // A fresh session's answer is final — retrying it just spends the
        // user's time re-asking a question that was already answered.
        assert!(!worth_retrying(false, &no_body));
        assert!(worth_retrying(true, &SkimError::other("imap", "boom")));
        // A failed local write is not the connection's fault either way.
        assert!(!worth_retrying(true, &SkimError::other("db", "boom")));
        assert!(!worth_retrying(false, &SkimError::other("imap", "boom")));
    }

    #[test]
    fn only_connection_errors_earn_a_reconnect() {
        // "mail" is the engine's code for answers *about the message* — it was
        // deleted, or the server returned no body. The connection itself is
        // fine, so this alone never earns a reconnect; `worth_retrying` is what
        // decides that a *reused* session's answer is worth a second opinion.
        assert!(!is_connection_error(&SkimError::other(
            "mail",
            "message no longer exists"
        )));
        for code in ["db", "io", "internal"] {
            assert!(
                !is_connection_error(&SkimError::other(code, "boom")),
                "{code} is local, reconnecting can't help"
            );
        }
        for code in ["imap", "network", "tls", "auth", "folder"] {
            assert!(
                is_connection_error(&SkimError::other(code, "boom")),
                "{code} should reconnect"
            );
        }
    }

    /// What the scripted server does once the client is logged in.
    enum AfterLogin {
        AnswerNoop,
        StaySilent,
        CloseSocket,
    }

    /// Minimal IMAP server: greeting, LOGIN, then one scripted reaction to the
    /// probe. Returns the port and the server task.
    async fn scripted_server(after: AfterLogin) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(sock);
            reader.get_mut().write_all(b"* OK ready\r\n").await.unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap_or("a1").to_string();
            reader
                .get_mut()
                .write_all(format!("{tag} OK logged in\r\n").as_bytes())
                .await
                .unwrap();
            match after {
                AfterLogin::AnswerNoop => {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let tag = line.split_whitespace().next().unwrap_or("a2").to_string();
                    reader
                        .get_mut()
                        .write_all(format!("{tag} OK NOOP completed\r\n").as_bytes())
                        .await
                        .unwrap();
                }
                // Blackholed by sleep/wake or a network switch: the socket is
                // open, the server is gone, nothing ever comes back.
                AfterLogin::StaySilent => std::future::pending::<()>().await,
                AfterLogin::CloseSocket => drop(reader),
            }
        });
        (port, server)
    }

    async fn probe_against(after: AfterLogin, leash: Duration) -> bool {
        let (port, server) = scripted_server(after).await;
        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut client = async_imap::Client::new(tcp);
        client.read_response().await.unwrap().unwrap();
        let mut session = client.login("u", "p").await.map_err(|(e, _)| e).unwrap();
        let alive = noop_probe(&mut session, leash).await;
        server.abort();
        alive
    }

    #[tokio::test]
    async fn a_live_connection_answers_the_probe() {
        assert!(probe_against(AfterLogin::AnswerNoop, PROBE_TIMEOUT).await);
    }

    /// The case that cost forty seconds: a socket that is open but answers
    /// nothing. The probe must give up on its own leash instead of hanging.
    #[tokio::test]
    async fn a_silent_connection_fails_the_probe_on_its_leash() {
        let started = std::time::Instant::now();
        assert!(!probe_against(AfterLogin::StaySilent, Duration::from_millis(200)).await);
        assert!(started.elapsed() < Duration::from_secs(5), "probe hung");
    }

    #[tokio::test]
    async fn a_dropped_connection_fails_the_probe_at_once() {
        assert!(!probe_against(AfterLogin::CloseSocket, PROBE_TIMEOUT).await);
    }
}

#[cfg(test)]
mod uid_set_tests {
    use super::{uid_sets, MAX_UID_SET_LEN};

    #[test]
    fn collapses_runs_into_ranges() {
        assert_eq!(uid_sets(&[1, 2, 3]), vec!["1:3"]);
        assert_eq!(uid_sets(&[5]), vec!["5"]);
        assert_eq!(uid_sets(&[1, 2, 3, 7, 8, 10]), vec!["1:3,7:8,10"]);
    }

    #[test]
    fn sorts_and_dedupes_first() {
        // The op payload carries UIDs in whatever order the rows were ticked.
        assert_eq!(uid_sets(&[3, 1, 2, 2]), vec!["1:3"]);
    }

    #[test]
    fn empty_input_issues_no_command() {
        assert!(uid_sets(&[]).is_empty());
    }

    #[test]
    fn splits_scattered_uids_into_safe_sets() {
        // Every other UID, so nothing collapses — the worst case for line length.
        let uids: Vec<u32> = (0..5000).map(|i| i * 2 + 1).collect();
        let sets = uid_sets(&uids);
        assert!(
            sets.len() > 1,
            "a 5000-UID selection must not be one command"
        );
        assert!(sets.iter().all(|s| s.len() <= MAX_UID_SET_LEN));
        // Nothing may be dropped on the way: the sets still name every UID.
        let seen: Vec<u32> = sets
            .iter()
            .flat_map(|s| s.split(','))
            .map(|p| p.parse::<u32>().expect("no ranges in scattered input"))
            .collect();
        assert_eq!(seen, uids);
    }
}

#[cfg(test)]
mod utf7_tests {
    use super::{decode_imap_utf7, encode_imap_utf7, quote_mailbox};

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

    #[test]
    fn encodes_modified_utf7_names() {
        assert_eq!(encode_imap_utf7("INBOX"), "INBOX");
        assert_eq!(encode_imap_utf7("Work/Taxes"), "Work/Taxes");
        assert_eq!(encode_imap_utf7("Важное"), "&BBIEMAQ2BD0EPgQ1-");
        assert_eq!(encode_imap_utf7("Семья"), "&BCEENQQ8BEwETw-");
        assert_eq!(encode_imap_utf7("Tom & Jerry"), "Tom &- Jerry");
        assert_eq!(encode_imap_utf7("☺!"), "&Jjo-!");
    }

    #[test]
    fn encode_decode_round_trips() {
        for name in [
            "INBOX",
            "Taxes 2025",
            "Work/Clients/Acme",
            "Важное",
            "Tom & Jerry",
            "☺",
            // Outside the BMP: encodes as a surrogate pair.
            "Rechnungen 🧾",
            "混在 mixed Текст",
        ] {
            assert_eq!(decode_imap_utf7(&encode_imap_utf7(name)), name, "{name}");
        }
    }

    #[test]
    fn quotes_mailbox_names() {
        assert_eq!(quote_mailbox("Archive"), "\"Archive\"");
        assert_eq!(quote_mailbox("Deleted Items"), "\"Deleted Items\"");
        assert_eq!(quote_mailbox("[Gmail]/All Mail"), "\"[Gmail]/All Mail\"");
        assert_eq!(quote_mailbox("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_mailbox("back\\slash"), "\"back\\\\slash\"");
    }
}
