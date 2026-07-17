//! Message bodies, attachments, and local (optimistic) mutations.

use super::models::{Address, AttachmentMeta, MessageMeta, ThreadDetail};
use super::queries::{self, addresses_from_json};
use crate::mail::threading;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

pub struct StoredAttachment {
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub cache_path: String,
}

/// Persist a fetched body: html/text, snippet, attachment rows, FTS body.
pub fn set_body(
    conn: &mut Connection,
    message_pk: i64,
    html: Option<&str>,
    text: Option<&str>,
    snippet: &str,
    attachments: &[StoredAttachment],
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO message_bodies (message_id, body_html, body_text)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(message_id) DO UPDATE SET body_html = excluded.body_html,
                                               body_text = excluded.body_text",
        params![message_pk, html, text],
    )?;
    tx.execute(
        "UPDATE messages SET body_state = 1, has_attachments = ?2,
                             snippet = CASE WHEN snippet IS NULL OR snippet = '' THEN ?3 ELSE snippet END
         WHERE id = ?1",
        params![
            message_pk,
            attachments.iter().any(|a| !a.is_inline),
            snippet
        ],
    )?;
    tx.execute(
        "DELETE FROM attachments WHERE message_id = ?1",
        params![message_pk],
    )?;
    for a in attachments {
        tx.execute(
            "INSERT INTO attachments (message_id, part_id, filename, mime_type, size, content_id, is_inline, cache_path)
             VALUES (?1, '', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                message_pk,
                a.filename,
                a.mime_type,
                a.size,
                a.content_id,
                a.is_inline,
                a.cache_path
            ],
        )?;
    }
    if let Some(text) = text {
        queries::fts_index_body(&tx, message_pk, text)?;
    }
    // Refresh the owning thread's snippet.
    let thread_id: Option<i64> = tx
        .query_row(
            "SELECT thread_id FROM messages WHERE id = ?1",
            params![message_pk],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    if let Some(tid) = thread_id {
        threading::recompute_thread(&tx, tid)?;
    }
    tx.commit()
}

/// Optimistically mirror an edited server draft onto its local message row so
/// the list and a reopen show the new content before the write-back to the IMAP
/// Drafts folder lands. Overwrites subject/snippet/body (drafts are plain text,
/// so the HTML body is cleared) and keeps FTS + the thread snippet coherent.
pub fn patch_local_draft(
    conn: &mut Connection,
    message_id: i64,
    subject: &str,
    body: &str,
) -> rusqlite::Result<()> {
    let snippet: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let snippet: String = snippet.chars().take(200).collect();
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE messages SET subject = ?2, snippet = ?3 WHERE id = ?1",
        params![message_id, subject, snippet],
    )?;
    tx.execute(
        "INSERT INTO message_bodies (message_id, body_html, body_text)
         VALUES (?1, NULL, ?2)
         ON CONFLICT(message_id) DO UPDATE SET body_html = NULL, body_text = excluded.body_text",
        params![message_id, body],
    )?;
    queries::fts_index_body(&tx, message_id, body)?;
    let thread_id: Option<i64> = tx
        .query_row(
            "SELECT thread_id FROM messages WHERE id = ?1",
            params![message_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    if let Some(tid) = thread_id {
        threading::recompute_thread(&tx, tid)?;
    }
    tx.commit()
}

pub fn get_body(
    conn: &Connection,
    message_pk: i64,
) -> rusqlite::Result<Option<(Option<String>, Option<String>)>> {
    conn.query_row(
        "SELECT body_html, body_text FROM message_bodies WHERE message_id = ?1",
        params![message_pk],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

pub fn body_state(conn: &Connection, message_pk: i64) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT body_state FROM messages WHERE id = ?1",
        params![message_pk],
        |r| r.get(0),
    )
    .optional()
}

pub fn list_attachments(
    conn: &Connection,
    message_pk: i64,
) -> rusqlite::Result<Vec<AttachmentMeta>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, message_id, filename, mime_type, size, is_inline
         FROM attachments WHERE message_id = ?1 AND is_inline = 0 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![message_pk], |r| {
            Ok(AttachmentMeta {
                id: r.get(0)?,
                message_id: r.get(1)?,
                filename: r.get(2)?,
                mime_type: r.get(3)?,
                size: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                is_inline: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A non-inline attachment with its on-disk cache path — enough for the AI
/// layer to read the bytes and extract/attach content.
pub struct AttachmentContent {
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
    pub cache_path: Option<String>,
}

/// Real (non-inline) attachments of a message, with cache paths. Inline `cid:`
/// parts (email logos etc.) are excluded — same filter as `list_attachments`.
pub fn list_attachment_files(
    conn: &Connection,
    message_pk: i64,
) -> rusqlite::Result<Vec<AttachmentContent>> {
    let mut stmt = conn.prepare_cached(
        "SELECT filename, mime_type, size, cache_path
         FROM attachments WHERE message_id = ?1 AND is_inline = 0 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![message_pk], |r| {
            Ok(AttachmentContent {
                filename: r.get(0)?,
                mime_type: r.get(1)?,
                size: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                cache_path: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub struct AttachmentFile {
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub cache_path: Option<String>,
}

pub fn get_attachment(conn: &Connection, id: i64) -> rusqlite::Result<Option<AttachmentFile>> {
    conn.query_row(
        "SELECT filename, mime_type, cache_path FROM attachments WHERE id = ?1",
        params![id],
        |r| {
            Ok(AttachmentFile {
                filename: r.get(0)?,
                mime_type: r.get(1)?,
                cache_path: r.get(2)?,
            })
        },
    )
    .optional()
}

pub fn get_attachment_by_cid(
    conn: &Connection,
    message_pk: i64,
    content_id: &str,
) -> rusqlite::Result<Option<AttachmentFile>> {
    conn.query_row(
        "SELECT filename, mime_type, cache_path FROM attachments
         WHERE message_id = ?1 AND content_id = ?2",
        params![message_pk, content_id],
        |r| {
            Ok(AttachmentFile {
                filename: r.get(0)?,
                mime_type: r.get(1)?,
                cache_path: r.get(2)?,
            })
        },
    )
    .optional()
}

fn row_to_meta(r: &rusqlite::Row) -> rusqlite::Result<MessageMeta> {
    let to_json: Option<String> = r.get(6)?;
    let cc_json: Option<String> = r.get(7)?;
    let list_unsubscribe: Option<String> = r.get(14)?;
    Ok(MessageMeta {
        id: r.get(0)?,
        folder_id: r.get(1)?,
        thread_id: r.get(2)?,
        subject: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
        from: Address {
            name: r.get(4)?,
            addr: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        },
        to: addresses_from_json(to_json.as_deref()),
        cc: addresses_from_json(cc_json.as_deref()),
        date: r.get(8)?,
        snippet: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
        is_read: r.get(10)?,
        is_starred: r.get(11)?,
        has_attachments: r.get(12)?,
        body_state: r.get(13)?,
        can_unsubscribe: list_unsubscribe.is_some(),
    })
}

/// All messages of a thread, oldest first, deduplicated by Message-ID
/// (Gmail label folders store copies of the same message under several
/// mailboxes).
pub fn get_thread(conn: &Connection, thread_id: i64) -> rusqlite::Result<Option<ThreadDetail>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, folder_id, thread_id, subject, from_name, from_addr, to_addrs, cc_addrs,
                date, snippet, is_read, is_starred, has_attachments, body_state,
                list_unsubscribe, list_unsubscribe_one_click, message_id
         FROM messages WHERE thread_id = ?1 ORDER BY date, id",
    )?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut messages = Vec::new();
    let rows = stmt.query_map(params![thread_id], |r| {
        let meta = row_to_meta(r)?;
        let msgid: Option<String> = r.get(16)?;
        Ok((meta, msgid))
    })?;
    for row in rows {
        let (meta, msgid) = row?;
        let key = msgid.unwrap_or_else(|| format!("pk:{}", meta.id));
        if seen.insert(key) {
            messages.push(meta);
        }
    }
    if messages.is_empty() {
        return Ok(None);
    }
    let subject = messages
        .last()
        .map(|m| m.subject.clone())
        .unwrap_or_default();
    Ok(Some(ThreadDetail {
        id: thread_id,
        subject,
        messages,
    }))
}

/// Message ids → their (folder, account, imap uid) coordinates, grouped per
/// folder. Resolved at enqueue time because the local rows may disappear
/// right after (archive/delete are optimistic).
pub struct FolderUids {
    pub account_id: String,
    pub folder_id: i64,
    pub imap_name: String,
    pub uids: Vec<u32>,
}

pub fn resolve_uids(conn: &Connection, message_ids: &[i64]) -> rusqlite::Result<Vec<FolderUids>> {
    let mut by_folder: HashMap<i64, FolderUids> = HashMap::new();
    let mut stmt = conn.prepare_cached(
        "SELECT m.folder_id, m.account_id, f.imap_name, m.uid
         FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE m.id = ?1",
    )?;
    for id in message_ids {
        let row: Option<(i64, String, String, u32)> = stmt
            .query_row(params![id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        if let Some((folder_id, account_id, imap_name, uid)) = row {
            by_folder
                .entry(folder_id)
                .or_insert_with(|| FolderUids {
                    account_id,
                    folder_id,
                    imap_name,
                    uids: Vec::new(),
                })
                .uids
                .push(uid);
        }
    }
    Ok(by_folder.into_values().collect())
}

/// Optimistic flag change. Returns affected thread + folder ids.
pub fn set_flag_local(
    conn: &mut Connection,
    message_ids: &[i64],
    flag: &str, // 'seen' | 'flagged'
    on: bool,
) -> rusqlite::Result<()> {
    let column = match flag {
        "seen" => "is_read",
        _ => "is_starred",
    };
    let tx = conn.transaction()?;
    let mut threads = HashSet::new();
    let mut folders = HashSet::new();
    for id in message_ids {
        tx.execute(
            &format!("UPDATE messages SET {column} = ?2 WHERE id = ?1"),
            params![id, on],
        )?;
        let row: Option<(Option<i64>, i64)> = tx
            .query_row(
                "SELECT thread_id, folder_id FROM messages WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((tid, fid)) = row {
            if let Some(tid) = tid {
                threads.insert(tid);
            }
            folders.insert(fid);
        }
    }
    for tid in threads {
        threading::recompute_thread(&tx, tid)?;
    }
    for fid in folders {
        queries::recompute_folder_unread(&tx, fid)?;
    }
    tx.commit()
}

/// Optimistic removal (archive/delete). Deletes local rows and cleans FTS;
/// the server catches up via the ops queue.
pub fn remove_messages_local(conn: &mut Connection, message_ids: &[i64]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let mut threads = HashSet::new();
    let mut folders = HashSet::new();
    for id in message_ids {
        let row: Option<(Option<i64>, i64)> = tx
            .query_row(
                "SELECT thread_id, folder_id FROM messages WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((tid, fid)) = row else { continue };
        tx.execute("DELETE FROM messages_fts WHERE rowid = ?1", params![id])?;
        tx.execute("DELETE FROM messages WHERE id = ?1", params![id])?;
        if let Some(tid) = tid {
            threads.insert(tid);
        }
        folders.insert(fid);
    }
    for tid in threads {
        threading::recompute_thread(&tx, tid)?;
    }
    for fid in folders {
        queries::recompute_folder_unread(&tx, fid)?;
    }
    tx.commit()
}

/// Cache path of the calendar part of a message, if any. Prefers the true
/// `text/calendar` MIME part over `.ics` file attachments from odd senders.
pub fn find_calendar_part(conn: &Connection, message_pk: i64) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT cache_path FROM attachments
         WHERE message_id = ?1
           AND (mime_type LIKE 'text/calendar%' OR mime_type = 'application/ics'
                OR filename LIKE '%.ics')
         ORDER BY (mime_type LIKE 'text/calendar%') DESC LIMIT 1",
        params![message_pk],
        |r| r.get(0),
    )
    .optional()
}

/// The user's stored RSVP ('ACCEPTED' | 'DECLINED' | 'TENTATIVE') for an event.
pub fn get_rsvp(
    conn: &Connection,
    account_id: &str,
    event_uid: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT partstat FROM invite_rsvps WHERE account_id = ?1 AND event_uid = ?2",
        params![account_id, event_uid],
        |r| r.get(0),
    )
    .optional()
}

pub fn upsert_rsvp(
    conn: &Connection,
    account_id: &str,
    event_uid: &str,
    partstat: &str,
    sequence: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO invite_rsvps (account_id, event_uid, partstat, sequence, responded_at)
         VALUES (?1, ?2, ?3, ?4, unixepoch())
         ON CONFLICT(account_id, event_uid) DO UPDATE SET
           partstat = excluded.partstat, sequence = excluded.sequence,
           responded_at = excluded.responded_at",
        params![account_id, event_uid, partstat, sequence],
    )?;
    Ok(())
}

/// Drop a stored RSVP answer — used to undo an optimistic response whose
/// reply email failed to send after all retries.
pub fn delete_rsvp(conn: &Connection, account_id: &str, event_uid: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM invite_rsvps WHERE account_id = ?1 AND event_uid = ?2",
        params![account_id, event_uid],
    )?;
    Ok(())
}

pub fn enqueue_op(
    conn: &Connection,
    account_id: &str,
    kind: &str,
    payload: &serde_json::Value,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO pending_ops (account_id, kind, payload, created_at)
         VALUES (?1, ?2, ?3, unixepoch())",
        params![account_id, kind, payload.to_string()],
    )?;
    Ok(())
}
