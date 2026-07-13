//! RSVP to calendar invitations: build an iMIP METHOD:REPLY and queue it
//! through the offline op queue.

use crate::db::{accounts, bodies};
use crate::error::{Result, SkimError};
use crate::mail::ics;
use crate::state::AppState;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn rsvp_invite(
    app: AppHandle,
    state: State<'_, AppState>,
    message_id: i64,
    response: String,
) -> Result<()> {
    let partstat = match response.as_str() {
        "accepted" => "ACCEPTED",
        "declined" => "DECLINED",
        "tentative" => "TENTATIVE",
        other => {
            return Err(SkimError::other(
                "invite",
                format!("unknown response: {other}"),
            ))
        }
    };

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
        .await?;
    let Some((path, account_id)) = found else {
        return Err(SkimError::other(
            "invite",
            "no calendar part on this message",
        ));
    };

    let bytes = tokio::fs::read(&path).await?;
    let invite = ics::parse_invite(&bytes)
        .ok_or_else(|| SkimError::other("invite", "cannot parse the invitation"))?;
    if invite.method != ics::InviteMethod::Request {
        return Err(SkimError::other(
            "invite",
            "message is not an invitation request",
        ));
    }
    let organizer = invite
        .organizer_email
        .clone()
        .ok_or_else(|| SkimError::other("invite", "invitation has no organizer"))?;

    let account = {
        let account_id = account_id.clone();
        state
            .db
            .call(move |conn| accounts::get(conn, &account_id))
            .await?
            .ok_or_else(|| SkimError::other("invite", "account not found"))?
    };

    let ics_body = ics::build_reply_ics(
        &invite,
        &account.email,
        account.display_name.as_deref(),
        partstat,
    );

    // English on the wire: this subject/body convention is what Gmail and
    // Outlook recognize and render on the organizer's side; it is not UI copy.
    let summary = invite.summary.as_deref().unwrap_or("Invitation");
    let (subject, verb) = match partstat {
        "ACCEPTED" => (format!("Accepted: {summary}"), "accepted"),
        "DECLINED" => (format!("Declined: {summary}"), "declined"),
        _ => (
            format!("Tentatively Accepted: {summary}"),
            "tentatively accepted",
        ),
    };
    let text_body = format!("{} has {verb} this invitation.", account.email);

    // Optimistic local state; the queued op delivers the reply (with retries
    // and offline tolerance) via the sync engine.
    {
        let account_id = account_id.clone();
        let uid = invite.uid.clone();
        let sequence = invite.sequence;
        let partstat = partstat.to_string();
        let payload = json!({
            "to": organizer,
            "subject": subject,
            "textBody": text_body,
            "ics": ics_body,
        });
        state
            .db
            .call(move |conn| {
                bodies::upsert_rsvp(conn, &account_id, &uid, &partstat, sequence)?;
                bodies::enqueue_op(conn, &account_id, "rsvp", &payload)
            })
            .await?;
    }

    let engines = state.engines.lock().await;
    if let Some(handle) = engines.get(&account_id) {
        handle.run_ops();
    }
    let _ = app.emit("mail:updated", json!({}));
    Ok(())
}
