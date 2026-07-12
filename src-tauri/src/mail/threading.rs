//! Gmail-style conversation threading.
//!
//! A message joins a thread when any of its `References` / `In-Reply-To` /
//! `Message-ID` identifiers match a message already in the database. When the
//! matches span several threads, those threads are merged into the oldest one.
//! Messages with no reference data fall back to a normalized-subject match,
//! guarded by account, a 30-day window and at least one shared participant so
//! that newsletters with identical subjects don't collapse into mega-threads.

use crate::db::models::NewMessage;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;

const SUBJECT_FALLBACK_WINDOW_SECS: i64 = 30 * 24 * 3600;

/// Strip reply/forward prefixes and collapse whitespace; empty subjects
/// return `None` so they never participate in subject-fallback matching.
pub fn normalize_subject(subject: &str) -> Option<String> {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        let mut stripped = false;
        for prefix in [
            "re:", "fwd:", "fw:", "aw:", "sv:", "vs:", "odp:", "antw:", "回复:", "回覆:", "转发:",
            "答复:", "res:", "rif:",
        ] {
            if lower.starts_with(prefix) {
                s = s[prefix.len()..].trim_start();
                stripped = true;
                break;
            }
        }
        // "Re[2]:" style
        if !stripped && (s.starts_with("Re[") || s.starts_with("re[") || s.starts_with("RE[")) {
            if let Some(end) = s.find("]:") {
                if s[3..end].chars().all(|c| c.is_ascii_digit()) {
                    s = s[end + 2..].trim_start();
                    stripped = true;
                }
            }
        }
        if !stripped {
            break;
        }
    }
    let norm = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if norm.is_empty() {
        None
    } else {
        Some(norm)
    }
}

/// Normalize a Message-ID-like token: strip angle brackets and whitespace.
pub fn normalize_msgid(raw: &str) -> Option<String> {
    let s = raw
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Resolve (or create) the thread for `msg`. Returns the thread id.
/// Runs inside the caller's transaction.
pub fn resolve_thread(conn: &Connection, msg: &NewMessage) -> rusqlite::Result<i64> {
    // 1. Reference-based match.
    let mut ref_ids: BTreeSet<String> = BTreeSet::new();
    for r in &msg.references {
        if let Some(id) = normalize_msgid(r) {
            ref_ids.insert(id);
        }
    }
    if let Some(irt) = msg.in_reply_to.as_deref().and_then(normalize_msgid_opt) {
        ref_ids.insert(irt);
    }
    if let Some(own) = msg.message_id.as_deref().and_then(normalize_msgid_opt) {
        // Another copy of this message (e.g. same mail in another Gmail label
        // folder) may already be threaded.
        ref_ids.insert(own);
    }

    let mut thread_ids: BTreeSet<i64> = BTreeSet::new();
    if !ref_ids.is_empty() {
        // Forward: messages whose Message-ID appears among our references.
        let mut forward = conn.prepare_cached(
            "SELECT DISTINCT thread_id FROM messages
             WHERE account_id = ?1 AND message_id = ?2 AND thread_id IS NOT NULL",
        )?;
        // Via references: messages that share any of these identifiers in
        // their own References/In-Reply-To. Covers both the parent arriving
        // after its replies (they reference us) and siblings that reference
        // a common ancestor we also reference — the usual cases when sync
        // walks newest-first.
        let mut via_refs = conn.prepare_cached(
            "SELECT DISTINCT m.thread_id FROM message_refs r
             JOIN messages m ON m.id = r.message_id
             WHERE m.account_id = ?1 AND r.ref = ?2 AND m.thread_id IS NOT NULL",
        )?;
        for id in &ref_ids {
            let found: Vec<i64> = forward
                .query_map(params![msg.account_id, id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            thread_ids.extend(found);
            let found: Vec<i64> = via_refs
                .query_map(params![msg.account_id, id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            thread_ids.extend(found);
        }
    }

    if !thread_ids.is_empty() {
        let mut iter = thread_ids.into_iter();
        let target = iter.next().expect("nonempty");
        // Merge any additional threads into the oldest (lowest id).
        for other in iter {
            merge_threads(conn, target, other)?;
        }
        return Ok(target);
    }

    // 2. Subject fallback — only when the message carries no reference data.
    let no_refs = msg.references.is_empty() && msg.in_reply_to.is_none();
    let subject_norm = msg.subject.as_deref().and_then(normalize_subject);
    if no_refs {
        if let Some(ref norm) = subject_norm {
            if let Some(id) = subject_fallback(conn, msg, norm)? {
                return Ok(id);
            }
        }
    }

    // 3. New thread.
    conn.execute(
        "INSERT INTO threads (account_id, subject_norm, last_date, message_count, unread_count, starred, snippet)
         VALUES (?1, ?2, ?3, 0, 0, 0, NULL)",
        params![msg.account_id, subject_norm, msg.date],
    )?;
    Ok(conn.last_insert_rowid())
}

fn normalize_msgid_opt(raw: &str) -> Option<String> {
    normalize_msgid(raw)
}

fn subject_fallback(
    conn: &Connection,
    msg: &NewMessage,
    subject_norm: &str,
) -> rusqlite::Result<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id FROM threads
         WHERE account_id = ?1 AND subject_norm = ?2
           AND abs(last_date - ?3) <= ?4
         ORDER BY last_date DESC LIMIT 8",
    )?;
    let candidates: Vec<i64> = stmt
        .query_map(
            params![
                msg.account_id,
                subject_norm,
                msg.date,
                SUBJECT_FALLBACK_WINDOW_SECS
            ],
            |r| r.get(0),
        )?
        .collect::<Result<_, _>>()?;

    // Participants of the incoming message.
    let mut participants: BTreeSet<String> = BTreeSet::new();
    if let Some(from) = &msg.from_addr {
        participants.insert(from.to_lowercase());
    }
    for a in msg.to_addrs.iter().chain(msg.cc_addrs.iter()) {
        participants.insert(a.addr.to_lowercase());
    }

    for thread_id in candidates {
        let mut stmt = conn.prepare_cached(
            "SELECT from_addr, to_addrs, cc_addrs FROM messages WHERE thread_id = ?1",
        )?;
        let rows: Vec<(Option<String>, Option<String>, Option<String>)> = stmt
            .query_map(params![thread_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        for (from, to_json, cc_json) in rows {
            if let Some(f) = from {
                if participants.contains(&f.to_lowercase()) {
                    return Ok(Some(thread_id));
                }
            }
            for json in [to_json, cc_json].into_iter().flatten() {
                if let Ok(addrs) = serde_json::from_str::<Vec<crate::db::models::Address>>(&json) {
                    if addrs
                        .iter()
                        .any(|a| participants.contains(&a.addr.to_lowercase()))
                    {
                        return Ok(Some(thread_id));
                    }
                }
            }
        }
    }
    Ok(None)
}

fn merge_threads(conn: &Connection, target: i64, other: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET thread_id = ?1 WHERE thread_id = ?2",
        params![target, other],
    )?;
    conn.execute("DELETE FROM threads WHERE id = ?1", params![other])?;
    recompute_thread(conn, target)?;
    Ok(())
}

/// Recompute a thread's aggregate columns from its messages. Deletes the
/// thread row when no messages remain; returns whether the thread survives.
pub fn recompute_thread(conn: &Connection, thread_id: i64) -> rusqlite::Result<bool> {
    let row: Option<(i64, i64, i64, i64)> = conn
        .query_row(
            "SELECT count(*), max(date), sum(is_read = 0), max(is_starred)
             FROM messages WHERE thread_id = ?1",
            params![thread_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                ))
            },
        )
        .optional()?;

    let (count, last_date, unread, starred) = row.unwrap_or((0, 0, 0, 0));
    if count == 0 {
        conn.execute("DELETE FROM threads WHERE id = ?1", params![thread_id])?;
        return Ok(false);
    }
    let snippet: Option<String> = conn
        .query_row(
            "SELECT snippet FROM messages WHERE thread_id = ?1 ORDER BY date DESC LIMIT 1",
            params![thread_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    conn.execute(
        "UPDATE threads SET message_count = ?2, last_date = ?3, unread_count = ?4,
                            starred = ?5, snippet = ?6
         WHERE id = ?1",
        params![thread_id, count, last_date, unread, starred, snippet],
    )?;
    Ok(true)
}
