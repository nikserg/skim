use crate::db::draft_attachments::{self, DraftAttachment};
use crate::db::drafts::{self, Draft};
use crate::db::{accounts as db_accounts, bodies};
use crate::error::{Result, SkimError};
use crate::mail::threading;
use crate::state::AppState;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressSuggestion {
    pub name: Option<String>,
    pub addr: String,
}

/// Autocomplete for the To/Cc/Bcc fields. Candidates come from the whole
/// mailbox — people the user wrote to rank above mere senders.
#[tauri::command]
pub async fn suggest_addresses(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<AddressSuggestion>> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    state
        .db
        .call(move |conn| {
            let like = format!(
                "%{}%",
                q.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            let mut stmt = conn.prepare_cached(
                "WITH cand(addr, name, weight, d) AS (
                     SELECT je.value->>'addr', je.value->>'name', 3, m.date
                     FROM messages m
                     JOIN folders f ON m.folder_id = f.id,
                          json_each(COALESCE(m.to_addrs, '[]')) je
                     WHERE f.role = 'sent'
                     UNION ALL
                     SELECT je.value->>'addr', je.value->>'name', 2, m.date
                     FROM messages m
                     JOIN folders f ON m.folder_id = f.id,
                          json_each(COALESCE(m.cc_addrs, '[]')) je
                     WHERE f.role = 'sent'
                     UNION ALL
                     SELECT m.from_addr, m.from_name, 1, m.date
                     FROM messages m
                     JOIN folders f ON m.folder_id = f.id
                     WHERE m.from_addr IS NOT NULL AND f.role != 'junk'
                 )
                 SELECT addr,
                        MAX(CASE WHEN name IS NOT NULL AND name != '' THEN name END),
                        SUM(weight) AS score,
                        MAX(d) AS last_date
                 FROM cand
                 WHERE addr IS NOT NULL
                   AND (addr LIKE ?1 ESCAPE '\\' OR COALESCE(name, '') LIKE ?1 ESCAPE '\\')
                 GROUP BY LOWER(addr)
                 ORDER BY score DESC, last_date DESC
                 LIMIT 8",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![like], |r| {
                    Ok(AddressSuggestion {
                        addr: r.get(0)?,
                        name: r.get(1)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
}

#[tauri::command]
pub async fn create_draft(state: State<'_, AppState>, account_id: Option<String>) -> Result<Draft> {
    let account = state
        .db
        .call(|conn| db_accounts::list(conn))
        .await?
        .into_iter()
        .find(|a| account_id.as_ref().is_none_or(|id| *id == a.id))
        .ok_or_else(|| SkimError::other("mail", "no account configured"))?;
    state
        .db
        .call(move |conn| drafts::create(conn, &account.id, "new", None, "", "", ""))
        .await
}

#[tauri::command]
pub async fn get_draft(state: State<'_, AppState>, draft_id: i64) -> Result<Draft> {
    state
        .db
        .call(move |conn| drafts::get(conn, draft_id))
        .await?
        .ok_or_else(|| SkimError::other("mail", "draft not found"))
}

#[tauri::command]
pub async fn update_draft(state: State<'_, AppState>, draft: Draft) -> Result<()> {
    state
        .db
        .call(move |conn| drafts::update(conn, &draft))
        .await
}

/// Move a draft to another mailbox — the From picker in the unified view.
/// Only allowed while the draft is local-only: once it mirrors a server copy
/// (reply chain or saved to a Drafts folder), moving it would orphan that copy.
#[tauri::command]
pub async fn set_draft_account(
    state: State<'_, AppState>,
    draft_id: i64,
    account_id: String,
) -> Result<()> {
    let changed = state
        .db
        .call(move |conn| {
            conn.execute(
                "UPDATE drafts SET account_id = ?2
                 WHERE id = ?1 AND origin_message_id IS NULL AND imap_message_id IS NULL",
                rusqlite::params![draft_id, account_id],
            )
        })
        .await?;
    if changed == 0 {
        return Err(SkimError::other(
            "compose",
            "draft is already tied to a mailbox",
        ));
    }
    Ok(())
}

/// Persist edits to a draft and queue the write-back to the IMAP Drafts folder so
/// the draft becomes a real, reopenable message there. A server-backed draft
/// (opened from Drafts) already mirrors a local `messages` row, patched
/// optimistically so the list and a reopen show the new content before the op
/// drains; a local-only draft has no such row yet — it gets a freshly minted
/// Message-ID and the post-op folder resync creates the row.
#[tauri::command]
pub async fn save_server_draft(
    app: AppHandle,
    state: State<'_, AppState>,
    draft: Draft,
) -> Result<()> {
    let account_id = draft.account_id.clone();
    let candidate = format!("skim-{}@skim.local", uuid::Uuid::new_v4());
    state
        .db
        .call(move |conn| {
            drafts::update(conn, &draft)?;
            // Give a local-only draft a stable server identity (no-op if it
            // already has one) so the save_draft op can APPEND it to Drafts.
            drafts::ensure_imap_message_id(conn, draft.id, &candidate)?;
            if let Some(msg_id) = draft.origin_message_id {
                bodies::patch_local_draft(conn, msg_id, &draft.subject, &draft.body)?;
            }
            bodies::enqueue_op(
                conn,
                &draft.account_id,
                "save_draft",
                &json!({ "draftId": draft.id }),
            )?;
            Ok(())
        })
        .await?;

    let engines = state.engines.lock().await;
    if let Some(handle) = engines.get(&account_id) {
        handle.run_ops();
    }
    let _ = app.emit("drafts:updated", json!({}));
    Ok(())
}

#[tauri::command]
pub async fn delete_draft(app: AppHandle, state: State<'_, AppState>, draft_id: i64) -> Result<()> {
    state
        .db
        .call(move |conn| drafts::delete(conn, draft_id))
        .await?;
    let _ = app.emit("drafts:updated", json!({}));
    Ok(())
}

/// Largest single attachment we accept, in bytes. Guards against wedging the
/// SMTP submission (most servers cap the whole message near 25 MB).
const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

#[tauri::command]
pub async fn add_draft_attachment(
    state: State<'_, AppState>,
    draft_id: i64,
    filename: String,
    mime_type: String,
    data: Vec<u8>,
) -> Result<DraftAttachment> {
    if data.len() > MAX_ATTACHMENT_BYTES {
        return Err(SkimError::other("attach", "file too large"));
    }
    state
        .db
        .call(move |conn| draft_attachments::add(conn, draft_id, &filename, &mime_type, &data))
        .await
}

#[tauri::command]
pub async fn list_draft_attachments(
    state: State<'_, AppState>,
    draft_id: i64,
) -> Result<Vec<DraftAttachment>> {
    state
        .db
        .call(move |conn| draft_attachments::list(conn, draft_id))
        .await
}

#[tauri::command]
pub async fn remove_draft_attachment(state: State<'_, AppState>, attachment_id: i64) -> Result<()> {
    state
        .db
        .call(move |conn| draft_attachments::remove(conn, attachment_id))
        .await
}

/// Prefill a reply/forward draft from an existing message.
#[tauri::command]
pub async fn get_reply_template(
    state: State<'_, AppState>,
    message_id: i64,
    mode: String, // 'reply' | 'reply_all' | 'forward'
) -> Result<Draft> {
    // The quoted body needs the message text — fetch it if not cached yet.
    let body_state = state
        .db
        .call(move |conn| bodies::body_state(conn, message_id))
        .await?
        .ok_or_else(|| SkimError::other("mail", "message not found"))?;
    if body_state == 0 {
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
        if let Some(handle) = handle {
            // Best effort — an offline reply still works, just without the quote.
            let _ = handle.fetch_body(message_id).await;
        }
    }

    type SourceRow = (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let mode_for_db = mode.clone();
    state
        .db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let row: Option<SourceRow> = conn
                .query_row(
                    "SELECT account_id, subject, from_name, from_addr, to_addrs, cc_addrs, date
                     FROM messages WHERE id = ?1",
                    rusqlite::params![message_id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                        ))
                    },
                )
                .optional()?;
            let Some((account_id, subject, from_name, from_addr, to_json, cc_json, date)) = row
            else {
                return Ok(None);
            };
            let account_email: String = conn.query_row(
                "SELECT email FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
                |r| r.get(0),
            )?;

            // Recipients.
            let to_field = match mode_for_db.as_str() {
                "reply" => from_addr.clone().unwrap_or_default(),
                "reply_all" => {
                    let mut all: Vec<String> = Vec::new();
                    if let Some(f) = &from_addr {
                        all.push(f.clone());
                    }
                    for a in crate::db::queries::addresses_from_json(to_json.as_deref())
                        .iter()
                        .chain(crate::db::queries::addresses_from_json(cc_json.as_deref()).iter())
                    {
                        all.push(a.addr.clone());
                    }
                    let own = account_email.to_lowercase();
                    let mut seen = std::collections::HashSet::new();
                    all.retain(|a| {
                        let l = a.to_lowercase();
                        l != own && seen.insert(l)
                    });
                    all.join(", ")
                }
                _ => String::new(),
            };

            // Subject.
            let base = subject.unwrap_or_default();
            let norm = threading::normalize_subject(&base).unwrap_or_default();
            let subject_field = if mode_for_db == "forward" {
                if base.to_lowercase().starts_with("fwd:") {
                    base
                } else {
                    format!("Fwd: {base}")
                }
            } else if !norm.is_empty() && base.to_lowercase().starts_with("re:") {
                base
            } else {
                format!("Re: {base}")
            };

            // Quoted body.
            let quoted = {
                let body: Option<(Option<String>, Option<String>)> =
                    bodies::get_body(conn, message_id)?;
                let text = body.and_then(|(_, t)| t).unwrap_or_default();
                let when = format_date(date);
                let who = match (&from_name, &from_addr) {
                    (Some(n), Some(a)) => format!("{n} <{a}>"),
                    (_, Some(a)) => a.clone(),
                    _ => "unknown sender".into(),
                };
                let mut q = format!("\n\nOn {when}, {who} wrote:\n");
                for line in text.lines() {
                    q.push_str("> ");
                    q.push_str(line);
                    q.push('\n');
                }
                q
            };

            let draft = drafts::create(
                conn,
                &account_id,
                &mode_for_db,
                Some(message_id),
                &to_field,
                &subject_field,
                &quoted,
            )?;
            Ok(Some(draft))
        })
        .await?
        .ok_or_else(|| SkimError::other("mail", "message not found"))
}

/// Render a stored `to`/`cc` address JSON array as the raw `Name <addr>`
/// string the compose fields (and `smtp::parse_recipients`) expect.
fn addrs_to_field(json: Option<&str>) -> String {
    crate::db::queries::addresses_from_json(json)
        .iter()
        .map(|a| match &a.name {
            Some(n) if !n.is_empty() => format!("{n} <{}>", a.addr),
            _ => a.addr.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Open a message from the IMAP Drafts folder as an editable local draft.
/// Reuses the existing local draft for the same server copy (so reopening never
/// forks a duplicate — matched first by the live row, then by the stable
/// Message-ID across resyncs); otherwise builds one from the stored headers and
/// body (fetching the body over IMAP if it isn't cached yet).
#[tauri::command]
pub async fn edit_draft(state: State<'_, AppState>, message_id: i64) -> Result<Draft> {
    // Fast path: a local draft already mirrors this server copy.
    let existing = state
        .db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            if let Some(d) = drafts::find_by_origin(conn, message_id)? {
                return Ok(Some(d));
            }
            let header: Option<String> = conn
                .query_row(
                    "SELECT message_id FROM messages WHERE id = ?1",
                    rusqlite::params![message_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            if let Some(h) = header {
                if let Some(mut d) = drafts::find_by_imap_message_id(conn, &h)? {
                    // A resync replaced the underlying row — re-point at the live one.
                    drafts::relink_origin(conn, d.id, message_id)?;
                    d.origin_message_id = Some(message_id);
                    return Ok(Some(d));
                }
            }
            Ok(None)
        })
        .await?;
    if let Some(d) = existing {
        return Ok(d);
    }

    // Ensure the body is cached so the editor opens with the draft text.
    let body_state = state
        .db
        .call(move |conn| bodies::body_state(conn, message_id))
        .await?
        .ok_or_else(|| SkimError::other("mail", "message not found"))?;
    if body_state == 0 {
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
        if let Some(handle) = handle {
            // Best effort — offline still opens the draft, just without any
            // body that wasn't synced yet.
            let _ = handle.fetch_body(message_id).await;
        }
    }

    type SourceRow = (
        String,         // account_id
        Option<String>, // subject
        Option<String>, // to_addrs json
        Option<String>, // cc_addrs json
        Option<String>, // message_id (RFC822 header)
        Option<String>, // in_reply_to
    );
    let (draft, att_coords) = state
        .db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let row: Option<SourceRow> = conn
                .query_row(
                    "SELECT account_id, subject, to_addrs, cc_addrs, message_id, in_reply_to
                     FROM messages WHERE id = ?1",
                    rusqlite::params![message_id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    },
                )
                .optional()?;
            let Some((account_id, subject, to_json, cc_json, header, in_reply_to)) = row else {
                return Ok(None);
            };

            let to_field = addrs_to_field(to_json.as_deref());
            let cc_field = addrs_to_field(cc_json.as_deref());

            // Prefer the plain-text body; an HTML-only draft opens empty.
            let body = bodies::get_body(conn, message_id)?
                .and_then(|(_, t)| t)
                .unwrap_or_default();

            // Thread the reply back to a locally-known parent, if any, so the
            // send path rebuilds In-Reply-To/References.
            let reply_to_id: Option<i64> = in_reply_to.as_deref().and_then(|irt| {
                let bare = irt.trim().trim_start_matches('<').trim_end_matches('>');
                if bare.is_empty() {
                    return None;
                }
                conn.query_row(
                    "SELECT id FROM messages WHERE account_id = ?1 AND message_id = ?2",
                    rusqlite::params![account_id, bare],
                    |r| r.get::<_, i64>(0),
                )
                .optional()
                .ok()
                .flatten()
            });
            let mode = if reply_to_id.is_some() {
                "reply"
            } else {
                "new"
            };

            // Stable identity for the server copy: reuse its Message-ID header,
            // or mint one when the draft has none yet.
            let imap_message_id = match header {
                Some(h) if !h.trim().is_empty() => h,
                _ => format!("skim-{}@skim.local", uuid::Uuid::new_v4()),
            };

            let draft = drafts::create_server_draft(
                conn,
                &account_id,
                mode,
                reply_to_id,
                &to_field,
                &cc_field,
                "",
                &subject.unwrap_or_default(),
                &body,
                message_id,
                &imap_message_id,
            )?;

            // The draft's existing (non-inline) attachments, already cached to
            // disk when the body was fetched. Return their coords so they can be
            // staged onto the draft without reading files under the DB lock.
            let mut stmt = conn.prepare(
                "SELECT filename, mime_type, cache_path FROM attachments
                 WHERE message_id = ?1 AND is_inline = 0 AND cache_path IS NOT NULL",
            )?;
            let att_coords: Vec<(Option<String>, Option<String>, String)> = stmt
                .query_map(rusqlite::params![message_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            Ok(Some((draft, att_coords)))
        })
        .await?
        .ok_or_else(|| SkimError::other("mail", "message not found"))?;

    // Stage the attachment bytes onto the draft so they show in the editor and
    // survive a re-save. File reads happen off the DB lock; a missing/unreadable
    // cache file is skipped (offline open of a never-downloaded attachment).
    for (filename, mime_type, cache_path) in att_coords {
        let Ok(bytes) = tokio::fs::read(&cache_path).await else {
            continue;
        };
        let name = filename.unwrap_or_else(|| "attachment".into());
        let mime = mime_type.unwrap_or_else(|| "application/octet-stream".into());
        let draft_id = draft.id;
        let _ = state
            .db
            .call(move |conn| draft_attachments::add(conn, draft_id, &name, &mime, &bytes))
            .await;
    }
    Ok(draft)
}

fn format_date(unix: i64) -> String {
    // RFC-ish absolute date for the quote header; avoids locale machinery.
    let days = unix / 86400;
    let (y, m, d) = civil_from_days(days);
    let secs = unix % 86400;
    format!(
        "{y}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60
    )
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    // Howard Hinnant's algorithm.
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[tauri::command]
pub async fn send_draft(app: AppHandle, state: State<'_, AppState>, draft_id: i64) -> Result<()> {
    let draft = state
        .db
        .call(move |conn| drafts::get(conn, draft_id))
        .await?
        .ok_or_else(|| SkimError::other("mail", "draft not found"))?;
    if crate::mail::smtp::parse_recipients(&draft.to).is_empty() {
        return Err(SkimError::other("send", "no valid recipients"));
    }

    let account_id = draft.account_id.clone();
    state
        .db
        .call(move |conn| {
            bodies::enqueue_op(
                conn,
                &draft.account_id,
                "send",
                &json!({ "draftId": draft.id }),
            )
        })
        .await?;

    let engines = state.engines.lock().await;
    if let Some(handle) = engines.get(&account_id) {
        handle.run_ops();
    }
    let _ = app.emit("drafts:updated", json!({}));
    Ok(())
}

#[tauri::command]
pub async fn open_compose_window(app: AppHandle, draft_id: i64) -> Result<()> {
    let label = format!("compose-{draft_id}");
    if let Some(existing) = tauri::Manager::get_webview_window(&app, &label) {
        let _ = existing.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::App(format!("index.html#/compose/{draft_id}").into()),
    )
    .title("Skim")
    .inner_size(720.0, 680.0)
    .min_inner_size(520.0, 420.0)
    .decorations(false)
    // Let HTML5 dragover/drop reach the webview (for file attachments) instead
    // of Tauri intercepting native file drops. Window-move via
    // data-tauri-drag-region is unaffected.
    .disable_drag_drop_handler()
    .center()
    .build()
    .map_err(|e| SkimError::other("window", e.to_string()))?;
    Ok(())
}
