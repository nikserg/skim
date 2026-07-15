use crate::db::models::{Folder, InviteView, RenderedBody, ThreadDetail, ThreadRow};
use crate::db::{bodies, queries};
use crate::error::{Result, SkimError};
use crate::mail::{ics, sanitize};
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

    Ok(RenderedBody {
        message_id,
        html: rendered.html,
        blocked_images: rendered.blocked_images,
        from_addr,
        attachments,
        invite,
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
        .map(|(folder_id, thread_id)| PendingOpen {
            folder_id,
            thread_id,
        })
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
