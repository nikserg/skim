use super::models::{Address, Folder, NewMessage, ThreadRow};
use crate::mail::threading;
use rusqlite::{params, Connection, OptionalExtension};

pub fn addresses_to_json(addrs: &[Address]) -> String {
    serde_json::to_string(addrs).unwrap_or_else(|_| "[]".into())
}

pub fn addresses_from_json(json: Option<&str>) -> Vec<Address> {
    json.and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default()
}

/// Insert (or skip, when the UID is already known) a message's headers,
/// resolve its thread, and index it in FTS. Returns `(message_id, thread_id)`
/// or `None` when the row already existed.
pub fn insert_message(
    conn: &mut Connection,
    msg: &NewMessage,
) -> rusqlite::Result<Option<(i64, i64)>> {
    let tx = conn.transaction()?;

    let exists: bool = tx
        .prepare_cached("SELECT 1 FROM messages WHERE folder_id = ?1 AND uid = ?2")?
        .exists(params![msg.folder_id, msg.uid])?;
    if exists {
        tx.commit()?;
        return Ok(None);
    }

    let thread_id = threading::resolve_thread(&tx, msg)?;

    let message_id_norm = msg
        .message_id
        .as_deref()
        .and_then(threading::normalize_msgid);
    let refs: Vec<String> = msg
        .references
        .iter()
        .filter_map(|r| threading::normalize_msgid(r))
        .chain(
            msg.in_reply_to
                .as_deref()
                .and_then(threading::normalize_msgid),
        )
        .collect();

    tx.prepare_cached(
        "INSERT INTO messages (account_id, folder_id, uid, thread_id, message_id, in_reply_to,
                               references_ids, subject, from_name, from_addr, to_addrs, cc_addrs,
                               date, snippet, size, is_read, is_starred, has_attachments, body_state)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, 0)",
    )?
    .execute(params![
        msg.account_id,
        msg.folder_id,
        msg.uid,
        thread_id,
        message_id_norm,
        msg.in_reply_to.as_deref().and_then(threading::normalize_msgid),
        serde_json::to_string(&refs).unwrap_or_else(|_| "[]".into()),
        msg.subject,
        msg.from_name,
        msg.from_addr,
        addresses_to_json(&msg.to_addrs),
        addresses_to_json(&msg.cc_addrs),
        msg.date,
        msg.snippet,
        msg.size,
        msg.is_read,
        msg.is_starred,
        msg.has_attachments,
    ])?;
    let message_pk = tx.last_insert_rowid();

    {
        let mut stmt =
            tx.prepare_cached("INSERT INTO message_refs (message_id, ref) VALUES (?1, ?2)")?;
        for r in &refs {
            stmt.execute(params![message_pk, r])?;
        }
    }

    threading::recompute_thread(&tx, thread_id)?;
    fts_index_headers(&tx, message_pk, msg)?;
    recompute_folder_unread(&tx, msg.folder_id)?;

    tx.commit()?;
    Ok(Some((message_pk, thread_id)))
}

fn fts_index_headers(conn: &Connection, message_pk: i64, msg: &NewMessage) -> rusqlite::Result<()> {
    let from_text = format!(
        "{} {}",
        msg.from_name.as_deref().unwrap_or(""),
        msg.from_addr.as_deref().unwrap_or("")
    );
    let to_text = msg
        .to_addrs
        .iter()
        .chain(msg.cc_addrs.iter())
        .map(|a| format!("{} {}", a.name.as_deref().unwrap_or(""), a.addr))
        .collect::<Vec<_>>()
        .join(" ");
    conn.prepare_cached(
        "INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
         VALUES (?1, ?2, ?3, ?4, '')",
    )?
    .execute(params![
        message_pk,
        msg.subject.as_deref().unwrap_or(""),
        from_text.trim(),
        to_text,
    ])?;
    Ok(())
}

type FtsHeaderRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Re-index a message in FTS including its (plain-text) body.
pub fn fts_index_body(conn: &Connection, message_pk: i64, body_text: &str) -> rusqlite::Result<()> {
    let row: Option<FtsHeaderRow> = conn
        .query_row(
            "SELECT subject, from_name, from_addr, to_addrs, cc_addrs FROM messages WHERE id = ?1",
            params![message_pk],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((subject, from_name, from_addr, to_json, cc_json)) = row else {
        return Ok(());
    };
    let from_text = format!(
        "{} {}",
        from_name.as_deref().unwrap_or(""),
        from_addr.as_deref().unwrap_or("")
    );
    let mut to_addrs = addresses_from_json(to_json.as_deref());
    to_addrs.extend(addresses_from_json(cc_json.as_deref()));
    let to_text = to_addrs
        .iter()
        .map(|a| format!("{} {}", a.name.as_deref().unwrap_or(""), a.addr))
        .collect::<Vec<_>>()
        .join(" ");

    conn.execute(
        "DELETE FROM messages_fts WHERE rowid = ?1",
        params![message_pk],
    )?;
    conn.prepare_cached(
        "INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![
        message_pk,
        subject.as_deref().unwrap_or(""),
        from_text.trim(),
        to_text,
        body_text,
    ])?;
    Ok(())
}

pub fn recompute_folder_unread(conn: &Connection, folder_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE folders SET unread_count =
            (SELECT count(*) FROM messages WHERE folder_id = ?1 AND is_read = 0)
         WHERE id = ?1",
        params![folder_id],
    )?;
    Ok(())
}

pub fn list_folders(conn: &Connection, account_id: &str) -> rusqlite::Result<Vec<Folder>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, account_id, imap_name, role, display_name, unread_count, sort_order
         FROM folders WHERE account_id = ?1 ORDER BY sort_order, display_name",
    )?;
    let rows = stmt
        .query_map(params![account_id], |r| {
            Ok(Folder {
                id: r.get(0)?,
                account_id: r.get(1)?,
                imap_name: r.get(2)?,
                role: r.get(3)?,
                display_name: r.get(4)?,
                unread_count: r.get(5)?,
                sort_order: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Threads visible in a folder, newest first, shaped by each thread's latest
/// message in that folder.
pub fn list_threads(
    conn: &Connection,
    folder_id: i64,
    offset: i64,
    limit: i64,
) -> rusqlite::Result<Vec<ThreadRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.id,
                m.from_name, m.from_addr, m.subject, m.snippet, t.last_date,
                (t.unread_count = 0), t.starred,
                max(m.has_attachments), t.message_count
         FROM threads t
         JOIN messages m ON m.thread_id = t.id
         WHERE m.folder_id = ?1
           AND m.date = (SELECT max(m2.date) FROM messages m2
                         WHERE m2.thread_id = t.id AND m2.folder_id = ?1)
         GROUP BY t.id
         ORDER BY t.last_date DESC
         LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt
        .query_map(params![folder_id, limit, offset], |r| {
            let from_name: Option<String> = r.get(1)?;
            let from_addr: Option<String> = r.get(2)?;
            Ok(ThreadRow {
                id: r.get(0)?,
                from_name: from_name
                    .filter(|s| !s.is_empty())
                    .or_else(|| from_addr.clone())
                    .unwrap_or_default(),
                from_addr: from_addr.unwrap_or_default(),
                subject: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                snippet: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                date: r.get(5)?,
                is_read: r.get(6)?,
                is_starred: r.get(7)?,
                has_attachments: r.get::<_, i64>(8)? != 0,
                message_count: r.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}
