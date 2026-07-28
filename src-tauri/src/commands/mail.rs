use crate::db::models::{Folder, InviteView, RenderedBody, ThreadDetail, ThreadRow};
use crate::db::{bodies, queries};
use crate::error::{Result, SkimError};
use crate::mail::{ics, sanitize, suspicion};
use crate::state::AppState;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

/// Parse the message's calendar part (if any) into card data. Any failure
/// degrades to None — the message then renders as a plain email.
pub async fn load_invite(state: &AppState, message_id: i64) -> Option<InviteView> {
    let found: Option<(String, String)> = state
        .db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let Some(path) = bodies::find_calendar_part(conn, message_id)? else {
                return Ok(None);
            };
            let account_id: Option<String> = conn
                .query_row(
                    "SELECT account_id FROM messages WHERE id = ?1",
                    rusqlite::params![message_id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(account_id.map(|a| (path, a)))
        })
        .await
        .ok()
        .flatten();
    let (path, account_id) = found?;
    let bytes = tokio::fs::read(&path).await.ok()?;
    let inv = ics::parse_invite(&bytes)?;

    let uid = inv.uid.clone();
    let my_response: Option<String> = state
        .db
        .call(move |conn| bodies::get_rsvp(conn, &account_id, &uid))
        .await
        .ok()
        .flatten()
        .map(|p| p.to_lowercase());

    let (reply_attendee, reply_partstat) = if inv.method == ics::InviteMethod::Reply {
        match inv.attendees.first() {
            Some(a) => (
                Some(a.name.clone().unwrap_or_else(|| a.email.clone())),
                a.partstat.clone().map(|p| p.to_lowercase()),
            ),
            None => (None, None),
        }
    } else {
        (None, None)
    };

    Some(InviteView {
        method: inv.method.as_str().to_string(),
        can_rsvp: inv.method == ics::InviteMethod::Request && inv.organizer_email.is_some(),
        uid: inv.uid,
        sequence: inv.sequence,
        summary: inv.summary,
        location: inv.location,
        organizer_name: inv.organizer_name,
        organizer_email: inv.organizer_email,
        starts_at: inv.starts_at,
        ends_at: inv.ends_at,
        is_all_day: inv.is_all_day,
        start_date: inv.start_date,
        end_date: inv.end_date,
        rrule: inv.rrule,
        attendee_count: inv.attendees.len(),
        my_response,
        reply_attendee,
        reply_partstat,
    })
}

#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>, account_id: String) -> Result<Vec<Folder>> {
    state
        .db
        .call(move |conn| queries::list_folders(conn, &account_id))
        .await
}

/// Which account a folder belongs to — lets the frontend switch the active
/// account before opening a location that lives in another mailbox (toast
/// clicks, cold-start pending opens, AI citations).
#[tauri::command]
pub async fn folder_account_id(state: State<'_, AppState>, folder_id: i64) -> Result<String> {
    state
        .db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            conn.query_row(
                "SELECT account_id FROM folders WHERE id = ?1",
                rusqlite::params![folder_id],
                |r| r.get(0),
            )
            .optional()
        })
        .await?
        .ok_or_else(|| SkimError::other("mail", "folder not found"))
}

#[tauri::command]
pub async fn list_threads(
    state: State<'_, AppState>,
    folder_id: i64,
    offset: i64,
    limit: i64,
) -> Result<Vec<ThreadRow>> {
    state
        .db
        .call(move |conn| queries::list_threads(conn, folder_id, offset, limit.clamp(1, 200)))
        .await
}

/// Flat (ungrouped) folder view: one row per message, newest first.
#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    folder_id: i64,
    offset: i64,
    limit: i64,
) -> Result<Vec<ThreadRow>> {
    state
        .db
        .call(move |conn| queries::list_messages(conn, folder_id, offset, limit.clamp(1, 200)))
        .await
}

/// The one logical folder set spanning every account — the "All inboxes" view.
#[tauri::command]
pub async fn list_unified_folders(state: State<'_, AppState>) -> Result<Vec<Folder>> {
    state
        .db
        .call(|conn| queries::list_unified_folders(conn))
        .await
}

/// Threads in a virtual folder (all accounts), newest first. The folder is
/// addressed by role, or by label display name when it has no role.
#[tauri::command]
pub async fn list_unified_threads(
    state: State<'_, AppState>,
    role: Option<String>,
    label: Option<String>,
    offset: i64,
    limit: i64,
) -> Result<Vec<ThreadRow>> {
    state
        .db
        .call(move |conn| {
            queries::list_unified_threads(
                conn,
                role.as_deref(),
                label.as_deref(),
                offset,
                limit.clamp(1, 200),
            )
        })
        .await
}

/// Flat (ungrouped) unified view: one row per message across all accounts.
#[tauri::command]
pub async fn list_unified_messages(
    state: State<'_, AppState>,
    role: Option<String>,
    label: Option<String>,
    offset: i64,
    limit: i64,
) -> Result<Vec<ThreadRow>> {
    state
        .db
        .call(move |conn| {
            queries::list_unified_messages(
                conn,
                role.as_deref(),
                label.as_deref(),
                offset,
                limit.clamp(1, 200),
            )
        })
        .await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderRef {
    pub role: Option<String>,
    pub display_name: String,
}

/// Role + name of a real folder — lets the unified view map a concrete folder
/// (from a search hit, toast, or AI citation) onto its virtual counterpart.
#[tauri::command]
pub async fn folder_ref(state: State<'_, AppState>, folder_id: i64) -> Result<FolderRef> {
    state
        .db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            conn.query_row(
                "SELECT role, display_name FROM folders WHERE id = ?1",
                rusqlite::params![folder_id],
                |r| {
                    Ok(FolderRef {
                        role: r.get(0)?,
                        display_name: r.get(1)?,
                    })
                },
            )
            .optional()
        })
        .await?
        .ok_or_else(|| SkimError::other("mail", "folder not found"))
}

#[tauri::command]
pub async fn get_thread(state: State<'_, AppState>, thread_id: i64) -> Result<ThreadDetail> {
    state
        .db
        .call(move |conn| bodies::get_thread(conn, thread_id))
        .await?
        .ok_or_else(|| SkimError::other("mail", "thread not found"))
}

/// Fetch (if needed), sanitize, and return a message body for display.
#[tauri::command]
pub async fn get_message_body(
    state: State<'_, AppState>,
    message_id: i64,
    show_images: Option<bool>,
) -> Result<RenderedBody> {
    // Ensure the body is cached locally.
    let cached = state
        .db
        .call(move |conn| bodies::body_state(conn, message_id))
        .await?;
    match cached {
        None => return Err(SkimError::other("mail", "message not found")),
        Some(0) => {
            let account_id: String = state
                .db
                .call(move |conn| {
                    conn.query_row(
                        "SELECT account_id FROM messages WHERE id = ?1",
                        rusqlite::params![message_id],
                        |r| r.get(0),
                    )
                })
                .await?;
            let handle = {
                let engines = state.engines.lock().await;
                engines.get(&account_id).cloned()
            };
            let handle =
                handle.ok_or_else(|| SkimError::other("sync", "sync engine is not running"))?;
            handle.fetch_body(message_id).await?;
        }
        _ => {}
    }

    // Image policy: global setting, per-sender allowlist, or one-off flag.
    let (body, from_addr, policy_always, sender_allowed) = state
        .db
        .call(move |conn| {
            let body = bodies::get_body(conn, message_id)?;
            let from_addr: Option<String> = conn
                .query_row(
                    "SELECT from_addr FROM messages WHERE id = ?1",
                    rusqlite::params![message_id],
                    |r| r.get(0),
                )
                .unwrap_or(None);
            let policy = queries::get_setting(conn, "images_policy")?;
            let allowed = match &from_addr {
                Some(addr) => {
                    let mut stmt =
                        conn.prepare_cached("SELECT 1 FROM remote_image_senders WHERE addr = ?1")?;
                    stmt.exists(rusqlite::params![addr.to_lowercase()])?
                }
                None => false,
            };
            Ok((
                body,
                from_addr,
                policy.as_deref() == Some("always"),
                allowed,
            ))
        })
        .await?;

    let (html, text) = body.unwrap_or((None, None));
    let allow_images = show_images.unwrap_or(false) || policy_always || sender_allowed;

    let rendered = match (html, text) {
        (Some(html), _) => sanitize::sanitize_email_html(&html, message_id, allow_images),
        (None, Some(text)) => sanitize::SanitizedHtml {
            html: sanitize::text_to_html(&text),
            blocked_images: 0,
        },
        (None, None) => sanitize::SanitizedHtml {
            html: String::new(),
            blocked_images: 0,
        },
    };

    let mut attachments = state
        .db
        .call(move |conn| bodies::list_attachments(conn, message_id))
        .await?;

    let invite = load_invite(&state, message_id).await;
    if invite.is_some() {
        // The card supersedes the raw calendar part — hide its chip.
        attachments.retain(|a| {
            let mime = a.mime_type.as_deref().unwrap_or("");
            let name = a.filename.as_deref().unwrap_or("");
            !(mime.starts_with("text/calendar")
                || mime == "application/ics"
                || name.to_ascii_lowercase().ends_with(".ics"))
        });
    }

    let security = {
        let html = rendered.html.clone();
        state
            .db
            .call(move |conn| suspicion::security_for_message(conn, message_id, &html))
            .await?
    };

    Ok(RenderedBody {
        message_id,
        html: rendered.html,
        blocked_images: rendered.blocked_images,
        from_addr,
        attachments,
        invite,
        security,
    })
}

#[tauri::command]
pub async fn allow_remote_images(state: State<'_, AppState>, sender_addr: String) -> Result<()> {
    state
        .db
        .call(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO remote_image_senders (addr, allowed_at)
                 VALUES (?1, unixepoch())",
                rusqlite::params![sender_addr.to_lowercase()],
            )
            .map(|_| ())
        })
        .await
}

/// Mark messages read/unread outside the IPC path (toast quick action).
pub async fn apply_read(
    app: &AppHandle,
    state: &AppState,
    message_ids: Vec<i64>,
    read: bool,
) -> Result<()> {
    queue_op(
        app,
        state,
        message_ids,
        "set_flag",
        json!({ "flag": "seen", "on": read }),
        move |conn, ids| bodies::set_flag_local(conn, ids, "seen", read),
    )
    .await
}

/// The thread a cold-start `skim://open` toast click queued, if any.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOpen {
    pub folder_id: i64,
    pub thread_id: i64,
    pub message_id: i64,
}

/// One-shot handoff: on cold start a toast click stashes its target in
/// `AppState` (the frontend listener isn't attached yet); the frontend calls
/// this once after boot to pick it up.
#[tauri::command]
pub fn take_pending_open(state: State<'_, AppState>) -> Option<PendingOpen> {
    state
        .pending_open
        .lock()
        .unwrap()
        .take()
        .map(|(folder_id, thread_id, message_id)| PendingOpen {
            folder_id,
            thread_id,
            message_id,
        })
}

/// Open a message in the main window from somewhere else — a citation clicked
/// in a popped-out AI chat. Reuses the toast-click path: the main window is
/// already listening for `mail:open-thread`.
#[tauri::command]
pub fn open_thread_in_main(
    app: AppHandle,
    folder_id: i64,
    thread_id: Option<i64>,
    message_id: i64,
) -> Result<()> {
    let _ = app.emit(
        "mail:open-thread",
        json!({ "folderId": folder_id, "threadId": thread_id, "messageId": message_id }),
    );
    crate::show_main_window(&app);
    Ok(())
}

/// Optimistic mutation + queued server op, shared by all flag/move actions.
async fn queue_op(
    app: &AppHandle,
    state: &AppState,
    message_ids: Vec<i64>,
    kind: &'static str,
    extra: serde_json::Value,
    local: impl FnOnce(&mut rusqlite::Connection, &[i64]) -> rusqlite::Result<()> + Send + 'static,
) -> Result<()> {
    let ids = message_ids.clone();
    let account_ids: Vec<String> = state
        .db
        .call(move |conn| {
            // Resolve coordinates BEFORE the optimistic mutation removes rows.
            let groups = bodies::resolve_uids(conn, &ids)?;
            let mut accounts = Vec::new();
            for g in &groups {
                let mut payload = json!({
                    "imapName": g.imap_name,
                    "folderId": g.folder_id,
                    "uids": g.uids,
                });
                if let Some(obj) = payload.as_object_mut() {
                    if let Some(extra_obj) = extra.as_object() {
                        for (k, v) in extra_obj {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
                bodies::enqueue_op(conn, &g.account_id, kind, &payload)?;
                accounts.push(g.account_id.clone());
            }
            local(conn, &ids)?;
            Ok(accounts)
        })
        .await?;

    let engines = state.engines.lock().await;
    for account_id in account_ids {
        if let Some(handle) = engines.get(&account_id) {
            handle.run_ops();
        }
    }
    let _ = app.emit("mail:updated", json!({}));
    // Reflect the optimistic read/archive/delete on the taskbar & tray badge
    // right away, without waiting for the next sync.
    crate::badge::refresh(app).await;
    Ok(())
}

#[tauri::command]
pub async fn mark_read(
    app: AppHandle,
    state: State<'_, AppState>,
    message_ids: Vec<i64>,
    read: bool,
) -> Result<()> {
    queue_op(
        &app,
        state.inner(),
        message_ids,
        "set_flag",
        json!({ "flag": "seen", "on": read }),
        move |conn, ids| bodies::set_flag_local(conn, ids, "seen", read),
    )
    .await
}

#[tauri::command]
pub async fn set_starred(
    app: AppHandle,
    state: State<'_, AppState>,
    message_ids: Vec<i64>,
    starred: bool,
) -> Result<()> {
    queue_op(
        &app,
        state.inner(),
        message_ids,
        "set_flag",
        json!({ "flag": "flagged", "on": starred }),
        move |conn, ids| bodies::set_flag_local(conn, ids, "flagged", starred),
    )
    .await
}

#[tauri::command]
pub async fn archive_messages(
    app: AppHandle,
    state: State<'_, AppState>,
    message_ids: Vec<i64>,
) -> Result<()> {
    queue_op(
        &app,
        state.inner(),
        message_ids,
        "archive",
        json!({}),
        bodies::remove_messages_local,
    )
    .await
}

#[tauri::command]
pub async fn delete_messages(
    app: AppHandle,
    state: State<'_, AppState>,
    message_ids: Vec<i64>,
) -> Result<()> {
    queue_op(
        &app,
        state.inner(),
        message_ids,
        "delete",
        json!({}),
        bodies::remove_messages_local,
    )
    .await
}

#[tauri::command]
pub async fn report_spam(
    app: AppHandle,
    state: State<'_, AppState>,
    message_ids: Vec<i64>,
) -> Result<()> {
    queue_op(
        &app,
        state.inner(),
        message_ids,
        "junk",
        json!({}),
        bodies::remove_messages_local,
    )
    .await
}

/// The accounts these messages belong to — one entry unless a selection somehow
/// spans mailboxes, and the folders they currently sit in.
fn message_origin(
    conn: &mut rusqlite::Connection,
    ids: &[i64],
) -> rusqlite::Result<(Vec<String>, Vec<i64>)> {
    let groups = bodies::resolve_uids(conn, ids)?;
    let sources: Vec<i64> = groups.iter().map(|g| g.folder_id).collect();
    let mut accounts: Vec<String> = groups.into_iter().map(|g| g.account_id).collect();
    accounts.sort();
    accounts.dedup();
    Ok((accounts, sources))
}

/// Turn a name the user typed into the pair the database stores: the wire name
/// (server's hierarchy separator, modified UTF-7) and the display name. Keeping
/// the two in step is what stops a folder we created or renamed from turning
/// into a second sidebar row on the next `discover_folders` sweep.
fn wire_folder_name(
    conn: &rusqlite::Connection,
    account_id: &str,
    typed: &str,
) -> rusqlite::Result<(String, String)> {
    let delimiter: Option<String> = conn.query_row(
        "SELECT folder_delimiter FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |r| r.get(0),
    )?;
    // The user types "/" for nesting; the server may want ".".
    let delimiter = delimiter
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/".into());
    let display_name = typed.replace('/', &delimiter);
    let imap_name = crate::mail::sync::encode_imap_utf7(&display_name);
    Ok((imap_name, display_name))
}

/// A folder the user is allowed to rename or delete: one of their own, i.e.
/// without a special-use role. Inbox, Sent, Trash and friends belong to the
/// provider and are refused here, not just hidden in the UI.
fn own_folder(
    conn: &rusqlite::Connection,
    folder_id: i64,
) -> rusqlite::Result<Option<(String, String, String)>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT account_id, imap_name, display_name FROM folders
         WHERE id = ?1 AND role IS NULL",
        rusqlite::params![folder_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()
}

/// Queue a folder-level op (no messages involved) and kick the sync engine.
async fn queue_folder_op(
    state: &AppState,
    account_id: &str,
    kind: &'static str,
    payload: serde_json::Value,
) -> Result<()> {
    let account = account_id.to_string();
    state
        .db
        .call(move |conn| bodies::enqueue_op(conn, &account, kind, &payload))
        .await?;
    let engines = state.engines.lock().await;
    if let Some(handle) = engines.get(account_id) {
        handle.run_ops();
    }
    Ok(())
}

/// How much mail a folder holds. The rename/delete dialog asks before offering
/// to delete: deleting a mailbox destroys its contents on every server that
/// isn't Gmail, so we only ever offer it for an empty one.
#[tauri::command]
pub async fn folder_message_count(state: State<'_, AppState>, folder_id: i64) -> Result<i64> {
    state
        .db
        .call(move |conn| {
            conn.query_row(
                "SELECT count(*) FROM messages WHERE folder_id = ?1",
                rusqlite::params![folder_id],
                |r| r.get(0),
            )
        })
        .await
}

/// Rename one of the user's own folders. Applied locally at once and queued for
/// the server, like every other mutation.
#[tauri::command]
pub async fn rename_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    folder_id: i64,
    new_name: String,
) -> Result<()> {
    let typed = new_name.trim().to_string();
    if typed.is_empty() {
        return Err(SkimError::other("folder", "folder name is empty"));
    }
    let payload = state
        .db
        .call(move |conn| {
            let Some((account_id, from_imap, from_display)) = own_folder(conn, folder_id)? else {
                return Ok(None);
            };
            let (to_imap, to_display) = wire_folder_name(conn, &account_id, &typed)?;
            if to_imap == from_imap {
                return Ok(None); // nothing to do
            }
            conn.execute(
                // Clearing the STATUS snapshot makes the next sweep actually
                // open the folder once, so a UIDVALIDITY the server changed
                // during RENAME is noticed instead of skipped.
                "UPDATE folders
                    SET imap_name = ?2, display_name = ?3, status_uidvalidity = NULL
                  WHERE id = ?1",
                rusqlite::params![folder_id, to_imap, to_display],
            )?;
            Ok(Some((
                account_id,
                json!({
                    "folderId": folder_id,
                    // The old pair doubles as the rollback value if the op
                    // never lands.
                    "imapName": from_imap,
                    "displayName": from_display,
                    "destImapName": to_imap,
                }),
            )))
        })
        .await?;
    let Some((account_id, payload)) = payload else {
        return Ok(());
    };
    let _ = app.emit("folders:updated", json!({}));
    queue_folder_op(state.inner(), &account_id, "rename_folder", payload).await
}

/// Delete one of the user's own folders — only while it is empty. IMAP DELETE
/// takes the mail inside with it on everything but Gmail, and Skim has no undo,
/// so this stays a way to undo a mistyped folder rather than a way to destroy
/// mail in bulk.
#[tauri::command]
pub async fn delete_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    folder_id: i64,
) -> Result<()> {
    let queued = state
        .db
        .call(move |conn| {
            let Some((account_id, imap_name, _)) = own_folder(conn, folder_id)? else {
                return Ok(None);
            };
            let count: i64 = conn.query_row(
                "SELECT count(*) FROM messages WHERE folder_id = ?1",
                rusqlite::params![folder_id],
                |r| r.get(0),
            )?;
            if count > 0 {
                return Ok(Some((account_id, imap_name, count)));
            }
            // Local row goes now; if the server op fails for good,
            // discover_folders puts it back on the next sweep.
            conn.execute(
                "DELETE FROM folders WHERE id = ?1",
                rusqlite::params![folder_id],
            )?;
            Ok(Some((account_id, imap_name, 0)))
        })
        .await?;
    let Some((account_id, imap_name, count)) = queued else {
        return Ok(());
    };
    if count > 0 {
        return Err(SkimError::other("folder", "folder is not empty"));
    }
    let _ = app.emit("folders:updated", json!({}));
    queue_folder_op(
        state.inner(),
        &account_id,
        "delete_folder",
        json!({ "folderId": folder_id, "imapName": imap_name }),
    )
    .await
}

/// Folders these messages can be filed into. Resolved here rather than in the
/// UI because neither view can work it out on its own: unified-inbox folders
/// carry synthetic negative ids, and a thread row never says which folder its
/// messages live in.
///
/// Excluded: the folders the messages are already in, the "all mail" role (it is
/// deliberately never synced), Starred (a server-side view, not a destination)
/// and Drafts. Inbox stays — moving back into it is how you un-archive.
#[tauri::command]
pub async fn move_targets(
    state: State<'_, AppState>,
    message_ids: Vec<i64>,
) -> Result<Vec<Folder>> {
    state
        .db
        .call(move |conn| {
            let groups = bodies::resolve_uids(conn, &message_ids)?;
            let mut accounts: Vec<&str> = groups.iter().map(|g| g.account_id.as_str()).collect();
            accounts.sort_unstable();
            accounts.dedup();
            // A cross-account move is a re-upload, not a MOVE — out of scope, so
            // offer nothing rather than something that cannot work.
            let [account_id] = accounts[..] else {
                return Ok(Vec::new());
            };
            let sources: Vec<i64> = groups.iter().map(|g| g.folder_id).collect();
            let folders = queries::list_folders(conn, account_id)?
                .into_iter()
                .filter(|f| !sources.contains(&f.id))
                .filter(|f| {
                    !matches!(
                        f.role.as_deref(),
                        Some("all") | Some("starred") | Some("drafts")
                    )
                })
                .collect();
            Ok(folders)
        })
        .await
}

/// File messages into another folder of the same account. With
/// `new_folder_name` the folder is created as part of the move: the row is
/// inserted locally right away so the sidebar shows it, and the server-side
/// CREATE rides inside the queued op so it works offline like everything else.
#[tauri::command]
pub async fn move_messages(
    app: AppHandle,
    state: State<'_, AppState>,
    message_ids: Vec<i64>,
    dest_folder_id: Option<i64>,
    new_folder_name: Option<String>,
) -> Result<()> {
    // Resolve the account before writing anything: creating the folder needs it,
    // and it rules out a cross-account move up front.
    let ids = message_ids.clone();
    let (accounts, sources) = state
        .db
        .call(move |conn| message_origin(conn, &ids))
        .await?;
    let account_id = match accounts.as_slice() {
        [only] => only.clone(),
        [] => return Err(SkimError::other("mail", "no messages to move")),
        _ => {
            return Err(SkimError::other(
                "mail",
                "cannot move mail between accounts",
            ))
        }
    };

    let (dest_folder_id, dest_imap_name, create_dest) = match new_folder_name {
        Some(typed) => {
            let typed = typed.trim().to_string();
            if typed.is_empty() {
                return Err(SkimError::other("mail", "folder name is empty"));
            }
            let account = account_id.clone();
            let (id, imap_name) = state
                .db
                .call(move |conn| {
                    // Store the wire name so discover_folders' ON CONFLICT
                    // reconciles onto this very row instead of adding a second
                    // one for the same mailbox.
                    let (imap_name, display_name) = wire_folder_name(conn, &account, &typed)?;
                    conn.execute(
                        "INSERT OR IGNORE INTO folders
                           (account_id, imap_name, role, display_name, sort_order)
                         VALUES (?1, ?2, NULL, ?3, 100)",
                        rusqlite::params![account, imap_name, display_name],
                    )?;
                    let id: i64 = conn.query_row(
                        "SELECT id FROM folders WHERE account_id = ?1 AND imap_name = ?2",
                        rusqlite::params![account, imap_name],
                        |r| r.get(0),
                    )?;
                    Ok((id, imap_name))
                })
                .await?;
            let _ = app.emit("folders:updated", json!({}));
            (id, imap_name, true)
        }
        None => {
            let Some(id) = dest_folder_id else {
                return Err(SkimError::other("mail", "no destination folder"));
            };
            let account = account_id.clone();
            let imap_name: String = state
                .db
                .call(move |conn| {
                    use rusqlite::OptionalExtension;
                    // Matching on the account too is the cross-account guard.
                    conn.query_row(
                        "SELECT imap_name FROM folders WHERE id = ?1 AND account_id = ?2",
                        rusqlite::params![id, account],
                        |r| r.get(0),
                    )
                    .optional()
                })
                .await?
                .ok_or_else(|| SkimError::other("mail", "destination folder not found"))?;
            (id, imap_name, false)
        }
    };

    // Filing mail where it already is does nothing — and must not go through
    // queue_op, whose optimistic step deletes the local rows for a move that
    // will then no-op on the server, hiding the mail until a full resync.
    // Reachable by typing the current folder's own name into the create row.
    if sources.contains(&dest_folder_id) {
        return Ok(());
    }

    queue_op(
        &app,
        state.inner(),
        message_ids,
        "move",
        json!({
            "destFolderId": dest_folder_id,
            "destImapName": dest_imap_name,
            "createDest": create_dest,
        }),
        bodies::remove_messages_local,
    )
    .await
}

/// Unsubscribe from a mailing list using the message's `List-Unsubscribe`
/// header. Acts for the user: a one-click POST (RFC 8058) or an unsubscribe
/// email goes through the offline op queue; a plain https link is opened in the
/// browser (the only path that needs a manual step). Returns `"submitted"` when
/// an op was queued, or `"opened"` when the browser was launched.
#[tauri::command]
pub async fn unsubscribe(
    app: AppHandle,
    state: State<'_, AppState>,
    message_id: i64,
) -> Result<String> {
    use rusqlite::OptionalExtension;

    let row: Option<(Option<String>, bool, String)> = state
        .db
        .call(move |conn| {
            conn.query_row(
                "SELECT list_unsubscribe, list_unsubscribe_one_click, account_id
                 FROM messages WHERE id = ?1",
                rusqlite::params![message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
        })
        .await?;

    let (raw, one_click, account_id) = match row {
        Some((Some(raw), one_click, account_id)) => (raw, one_click, account_id),
        _ => {
            return Err(SkimError::other(
                "unsubscribe",
                "this message has no unsubscribe option",
            ))
        }
    };

    use crate::mail::parse::{parse_unsub_targets, UnsubTarget};
    let targets = parse_unsub_targets(&raw);
    // The header is attacker-controlled; only https endpoints may receive the
    // silent one-click POST (RFC 8058 requires https). Plain http links are
    // never POSTed — at most opened visibly in the browser below.
    let https = targets.iter().find_map(|t| match t {
        UnsubTarget::Http(url) if url.starts_with("https://") => Some(url.clone()),
        _ => None,
    });
    let http = targets.iter().find_map(|t| match t {
        UnsubTarget::Http(url) => Some(url.clone()),
        _ => None,
    });
    let mail = targets.iter().find_map(|t| match t {
        UnsubTarget::Mail { to, subject } => Some((to.clone(), subject.clone())),
        _ => None,
    });

    // Priority: one-click POST → unsubscribe email → open link in the browser.
    if one_click {
        if let Some(url) = &https {
            let payload = json!({ "method": "post", "url": url });
            enqueue_unsub_op(&state, &account_id, payload).await?;
            let _ = app.emit("mail:updated", json!({}));
            return Ok("submitted".to_string());
        }
    }
    if let Some((to, subject)) = mail {
        let payload = json!({ "method": "mail", "to": to, "subject": subject });
        enqueue_unsub_op(&state, &account_id, payload).await?;
        let _ = app.emit("mail:updated", json!({}));
        return Ok("submitted".to_string());
    }
    if let Some(url) = http {
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| SkimError::other("io", e.to_string()))?;
        return Ok("opened".to_string());
    }

    Err(SkimError::other(
        "unsubscribe",
        "no usable unsubscribe target",
    ))
}

/// Queue a self-contained unsubscribe op and kick the sync engine.
async fn enqueue_unsub_op(
    state: &AppState,
    account_id: &str,
    payload: serde_json::Value,
) -> Result<()> {
    let account = account_id.to_string();
    state
        .db
        .call(move |conn| bodies::enqueue_op(conn, &account, "unsubscribe", &payload))
        .await?;
    let engines = state.engines.lock().await;
    if let Some(handle) = engines.get(account_id) {
        handle.run_ops();
    }
    Ok(())
}

#[tauri::command]
pub async fn save_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    attachment_id: i64,
) -> Result<Option<String>> {
    use tauri_plugin_dialog::DialogExt;

    let file = state
        .db
        .call(move |conn| bodies::get_attachment(conn, attachment_id))
        .await?
        .ok_or_else(|| SkimError::other("mail", "attachment not found"))?;
    let cache_path = file
        .cache_path
        .ok_or_else(|| SkimError::other("mail", "attachment is not downloaded yet"))?;

    let suggested = file.filename.unwrap_or_else(|| "attachment".into());
    let dialog = app.dialog().file().set_file_name(&suggested);
    let picked = tokio::task::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .map_err(|e| SkimError::other("internal", e.to_string()))?;

    let Some(dest) = picked else {
        return Ok(None); // user cancelled
    };
    let dest_path = dest
        .into_path()
        .map_err(|e| SkimError::other("io", e.to_string()))?;
    tokio::fs::copy(&cache_path, &dest_path).await?;
    Ok(Some(dest_path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn open_attachment(
    app: AppHandle,
    state: State<'_, AppState>,
    attachment_id: i64,
) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;

    let file = state
        .db
        .call(move |conn| bodies::get_attachment(conn, attachment_id))
        .await?
        .ok_or_else(|| SkimError::other("mail", "attachment not found"))?;
    let cache_path = file
        .cache_path
        .ok_or_else(|| SkimError::other("mail", "attachment is not downloaded yet"))?;
    app.opener()
        .open_path(cache_path, None::<&str>)
        .map_err(|e| SkimError::other("io", e.to_string()))
}

#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>, account_id: Option<String>) -> Result<()> {
    let engines = state.engines.lock().await;
    match account_id {
        Some(id) => {
            if let Some(handle) = engines.get(&id) {
                handle.sync_all();
            }
        }
        None => {
            for handle in engines.values() {
                handle.sync_all();
            }
        }
    }
    Ok(())
}
