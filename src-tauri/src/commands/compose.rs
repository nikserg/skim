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
pub async fn create_draft(state: State<'_, AppState>) -> Result<Draft> {
    let account = state
        .db
        .call(|conn| db_accounts::list(conn))
        .await?
        .into_iter()
        .next()
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
