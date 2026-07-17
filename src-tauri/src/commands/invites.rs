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
            // Carried so a permanently failed send can revert the optimistic
            // invite_rsvps row (see drain_ops in mail/sync.rs).
            "eventUid": uid.clone(),
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

/// Let the user add the invitation to their calendar. Skim has no calendar of
/// its own, so for accounts whose provider has a web calendar (Gmail, Outlook)
/// we open a pre-filled "create event" page — one click for people who live in
/// the browser and have no local `.ics` handler. Generic IMAP accounts (and any
/// invite we can't parse) fall back to handing the `.ics` file to the OS.
#[tauri::command]
pub async fn open_invite_ics(
    app: AppHandle,
    state: State<'_, AppState>,
    message_id: i64,
) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;

    // One round-trip: the cached `.ics` path plus the account's provider.
    let found: Option<(String, Option<String>)> = state
        .db
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let Some(path) = bodies::find_calendar_part(conn, message_id)? else {
                return Ok(None);
            };
            let provider: Option<String> = conn
                .query_row(
                    "SELECT a.provider FROM messages m JOIN accounts a ON a.id = m.account_id
                     WHERE m.id = ?1",
                    rusqlite::params![message_id],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(Some((path, provider)))
        })
        .await?;
    let Some((path, provider)) = found else {
        return Err(SkimError::other(
            "invite",
            "no calendar part on this message",
        ));
    };

    // Prefer the account's web calendar when we recognise the provider and can
    // parse the event out of the `.ics`.
    if let Some(provider) = provider.as_deref() {
        let bytes = tokio::fs::read(&path).await?;
        if let Some(invite) = ics::parse_invite(&bytes) {
            if let Some(url) = web_calendar_url(provider, &invite) {
                return app
                    .opener()
                    .open_url(url, None::<&str>)
                    .map_err(|e| SkimError::other("io", e.to_string()));
            }
        }
    }

    // Fallback: the cached file may have no extension (e.g. `0_attachment` for a
    // `text/calendar` part with no filename), so the OS can't recognise it as an
    // invitation. Open a copy named `*.ics` so it lands in the calendar app.
    let ics = std::env::temp_dir().join(format!("skim-invite-{message_id}.ics"));
    tokio::fs::copy(&path, &ics)
        .await
        .map_err(|e| SkimError::other("io", e.to_string()))?;
    app.opener()
        .open_path(ics.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| SkimError::other("io", e.to_string()))
}

/// A "create event" URL for the account provider's web calendar, or `None` for
/// providers we don't map (the caller then opens the `.ics` file instead).
fn web_calendar_url(provider: &str, invite: &ics::ParsedInvite) -> Option<String> {
    match provider {
        "gmail" => Some(google_calendar_url(invite)),
        "microsoft" => Some(outlook_calendar_url(invite)),
        _ => None,
    }
}

/// Google Calendar prefilled event (`render?action=TEMPLATE`).
fn google_calendar_url(invite: &ics::ParsedInvite) -> String {
    let mut url =
        url::Url::parse("https://calendar.google.com/calendar/render").expect("valid base url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("action", "TEMPLATE");
        if let Some(text) = invite.summary.as_deref() {
            q.append_pair("text", text);
        }
        if let Some(dates) = google_dates(invite) {
            q.append_pair("dates", &dates);
        }
        if let Some(loc) = invite.location.as_deref() {
            q.append_pair("location", loc);
        }
    }
    url.into()
}

/// The `dates=START/END` value Google expects: compact UTC for timed events,
/// `YYYYMMDD` with an *exclusive* end for all-day ones.
fn google_dates(invite: &ics::ParsedInvite) -> Option<String> {
    use chrono::{DateTime, Duration, NaiveDate, Utc};
    if invite.is_all_day {
        let start = NaiveDate::parse_from_str(invite.start_date.as_deref()?, "%Y-%m-%d").ok()?;
        let end_incl = invite
            .end_date
            .as_deref()
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .unwrap_or(start);
        let end = end_incl + Duration::days(1);
        Some(format!(
            "{}/{}",
            start.format("%Y%m%d"),
            end.format("%Y%m%d")
        ))
    } else {
        let start = DateTime::<Utc>::from_timestamp(invite.starts_at?, 0)?;
        let end = invite
            .ends_at
            .and_then(|e| DateTime::<Utc>::from_timestamp(e, 0))
            .unwrap_or_else(|| start + Duration::hours(1));
        Some(format!(
            "{}/{}",
            start.format("%Y%m%dT%H%M%SZ"),
            end.format("%Y%m%dT%H%M%SZ")
        ))
    }
}

/// Outlook web prefilled event (`deeplink/compose`).
fn outlook_calendar_url(invite: &ics::ParsedInvite) -> String {
    use chrono::{DateTime, Duration, NaiveDate, Utc};
    let mut url = url::Url::parse("https://outlook.office.com/calendar/0/deeplink/compose")
        .expect("valid base url");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("path", "/calendar/action/compose");
        q.append_pair("rru", "addevent");
        if let Some(subject) = invite.summary.as_deref() {
            q.append_pair("subject", subject);
        }
        if invite.is_all_day {
            if let Some(sd) = invite.start_date.as_deref() {
                q.append_pair("allday", "true");
                q.append_pair("startdt", sd);
                if let Some(end) = invite
                    .end_date
                    .as_deref()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .or_else(|| NaiveDate::parse_from_str(sd, "%Y-%m-%d").ok())
                    .map(|d| (d + Duration::days(1)).format("%Y-%m-%d").to_string())
                {
                    q.append_pair("enddt", &end);
                }
            }
        } else if let Some(start) = invite
            .starts_at
            .and_then(|s| DateTime::<Utc>::from_timestamp(s, 0))
        {
            q.append_pair("startdt", &start.format("%Y-%m-%dT%H:%M:%SZ").to_string());
            let end = invite
                .ends_at
                .and_then(|e| DateTime::<Utc>::from_timestamp(e, 0))
                .unwrap_or_else(|| start + Duration::hours(1));
            q.append_pair("enddt", &end.format("%Y-%m-%dT%H:%M:%SZ").to_string());
        }
        if let Some(loc) = invite.location.as_deref() {
            q.append_pair("location", loc);
        }
    }
    url.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed_request() -> ics::ParsedInvite {
        let ics = concat!(
            "BEGIN:VCALENDAR\r\n",
            "METHOD:REQUEST\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:x@example.com\r\n",
            "DTSTART:20260727T140000Z\r\n",
            "DTEND:20260727T150000Z\r\n",
            "SUMMARY:1 h slot\r\n",
            "LOCATION:https://meet.google.com/mwm-fmeg-azy\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );
        ics::parse_invite(ics.as_bytes()).expect("parses")
    }

    fn all_day_request() -> ics::ParsedInvite {
        let ics = concat!(
            "BEGIN:VCALENDAR\r\n",
            "METHOD:REQUEST\r\n",
            "BEGIN:VEVENT\r\n",
            "UID:y@example.com\r\n",
            "DTSTART;VALUE=DATE:20260727\r\n",
            "DTEND;VALUE=DATE:20260728\r\n",
            "SUMMARY:Offsite\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n",
        );
        ics::parse_invite(ics.as_bytes()).expect("parses")
    }

    #[test]
    fn unknown_provider_falls_back_to_file() {
        assert!(web_calendar_url("imap", &timed_request()).is_none());
        assert!(web_calendar_url("fastmail", &timed_request()).is_none());
    }

    #[test]
    fn google_url_has_template_action_and_utc_dates() {
        let url = google_calendar_url(&timed_request());
        assert!(url.starts_with("https://calendar.google.com/calendar/render?"));
        assert!(url.contains("action=TEMPLATE"));
        // Space encodes as '+', the '/' between dates as %2F.
        assert!(url.contains("text=1+h+slot"), "{url}");
        assert!(
            url.contains("dates=20260727T140000Z%2F20260727T150000Z"),
            "{url}"
        );
        // Meet link carried as the location, percent-encoded.
        assert!(
            url.contains("location=https%3A%2F%2Fmeet.google.com"),
            "{url}"
        );
    }

    #[test]
    fn google_all_day_end_is_exclusive() {
        // DTEND 07-28 is parsed as the inclusive last day (07-27); Google wants
        // the exclusive end, so the range is 20260727/20260728.
        let url = google_calendar_url(&all_day_request());
        assert!(url.contains("dates=20260727%2F20260728"), "{url}");
    }

    #[test]
    fn outlook_url_has_addevent_and_iso_dates() {
        let url = outlook_calendar_url(&timed_request());
        assert!(url.starts_with("https://outlook.office.com/calendar/0/deeplink/compose?"));
        assert!(url.contains("rru=addevent"));
        assert!(url.contains("subject=1+h+slot"), "{url}");
        // ISO 8601 UTC; ':' encodes as %3A.
        assert!(url.contains("startdt=2026-07-27T14%3A00%3A00Z"), "{url}");
        assert!(url.contains("enddt=2026-07-27T15%3A00%3A00Z"), "{url}");
    }

    #[test]
    fn routing_picks_provider() {
        assert!(web_calendar_url("gmail", &timed_request())
            .unwrap()
            .contains("calendar.google.com"));
        assert!(web_calendar_url("microsoft", &timed_request())
            .unwrap()
            .contains("outlook.office.com"));
    }
}
