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
                               date, snippet, size, is_read, is_starred, has_attachments, body_state,
                               list_unsubscribe, list_unsubscribe_one_click,
                               reply_to_addr, auth_spf, auth_dkim, auth_dmarc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, 0,
                 ?19, ?20, ?21, ?22, ?23, ?24)",
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
        msg.list_unsubscribe,
        msg.list_unsubscribe_one_click,
        msg.reply_to_addr,
        msg.auth_spf,
        msg.auth_dkim,
        msg.auth_dmarc,
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

/// Unread across every inbox (all accounts) — the number shown on the taskbar
/// and tray badge. Spam/promo/other folders are deliberately excluded so they
/// don't inflate the count.
pub fn total_inbox_unread(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(unread_count), 0) FROM folders WHERE role = 'inbox'",
        [],
        |r| r.get(0),
    )
}

/// Inbox unread per account — shown next to each account in the switcher.
pub fn inbox_unread_by_account(conn: &Connection) -> rusqlite::Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT account_id, COALESCE(SUM(unread_count), 0)
         FROM folders WHERE role = 'inbox' GROUP BY account_id",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
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
                (NOT EXISTS (SELECT 1 FROM messages m3
                             WHERE m3.thread_id = t.id AND m3.folder_id = ?1
                               AND m3.is_read = 0)),
                t.starred,
                max(m.has_attachments), t.message_count, t.account_id
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
                message_id: None,
                account_id: r.get(10)?,
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

/// Individual messages in a folder, newest first — the flat (ungrouped) view.
/// One row per message; no thread aggregation. Each row keeps `id = thread_id`
/// (so open/patch/archive still work by thread) and carries `message_id` for
/// the specific message to open.
pub fn list_messages(
    conn: &Connection,
    folder_id: i64,
    offset: i64,
    limit: i64,
) -> rusqlite::Result<Vec<ThreadRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.thread_id, m.id,
                m.from_name, m.from_addr, m.subject, m.snippet, m.date,
                m.is_read, m.is_starred, m.has_attachments, m.account_id
         FROM messages m
         WHERE m.folder_id = ?1
         ORDER BY m.date DESC, m.id DESC
         LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt
        .query_map(params![folder_id, limit, offset], |r| {
            let from_name: Option<String> = r.get(2)?;
            let from_addr: Option<String> = r.get(3)?;
            Ok(ThreadRow {
                id: r.get(0)?,
                message_id: Some(r.get(1)?),
                account_id: r.get(10)?,
                from_name: from_name
                    .filter(|s| !s.is_empty())
                    .or_else(|| from_addr.clone())
                    .unwrap_or_default(),
                from_addr: from_addr.unwrap_or_default(),
                subject: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                snippet: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                date: r.get(6)?,
                is_read: r.get(7)?,
                is_starred: r.get(8)?,
                has_attachments: r.get::<_, i64>(9)? != 0,
                message_count: 1,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Stable synthetic id for a virtual (cross-account) folder. Negative so it can
/// never collide with a real `folders.id`; deterministic so the frontend's
/// selection survives refreshes. Known roles get fixed small ids, user labels
/// (and any future role) hash their lowercased name.
pub fn virtual_folder_id(role: Option<&str>, display_name: &str) -> i64 {
    match role {
        Some("inbox") => -1,
        Some("starred") => -2,
        Some("sent") => -3,
        Some("drafts") => -4,
        Some("archive") => -5,
        Some("trash") => -6,
        Some("junk") => -7,
        Some("all") => -8,
        _ => {
            let key = match role {
                Some(r) => format!("r:{r}"),
                None => format!("l:{}", display_name.to_lowercase()),
            };
            // FNV-1a, folded to 31 bits — plenty for a handful of labels.
            let mut hash: u32 = 0x811c_9dc5;
            for b in key.as_bytes() {
                hash ^= u32::from(*b);
                hash = hash.wrapping_mul(0x0100_0193);
            }
            -(1000 + i64::from(hash & 0x7fff_ffff))
        }
    }
}

/// One logical folder set spanning every account: role-counterparts merge into
/// a single virtual folder (unread summed), user labels merge by name
/// (ASCII case-insensitive). Virtual folders carry synthetic negative ids and
/// `account_id = "*"`.
pub fn list_unified_folders(conn: &Connection) -> rusqlite::Result<Vec<Folder>> {
    let mut stmt = conn.prepare_cached(
        "SELECT role, MIN(display_name), COALESCE(SUM(unread_count), 0), MIN(sort_order)
         FROM folders
         GROUP BY COALESCE('r:' || role, 'l:' || lower(display_name))
         ORDER BY MIN(sort_order), MIN(display_name)",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let role: Option<String> = r.get(0)?;
            let display_name: String = r.get(1)?;
            Ok(Folder {
                id: virtual_folder_id(role.as_deref(), &display_name),
                account_id: "*".into(),
                imap_name: String::new(),
                role,
                display_name,
                unread_count: r.get(2)?,
                sort_order: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The `WHERE` half shared by the unified list queries: all real folders that
/// make up one virtual folder — by role, or by label name when `role` is None.
const UNIFIED_SEL: &str = "SELECT id FROM folders
     WHERE (?1 IS NOT NULL AND role = ?1)
        OR (?1 IS NULL AND role IS NULL AND display_name = ?2 COLLATE NOCASE)";

/// Threads visible in a virtual folder (all accounts), newest first. Threads
/// never span accounts, so this interleaves per-account threads by date.
pub fn list_unified_threads(
    conn: &Connection,
    role: Option<&str>,
    label: Option<&str>,
    offset: i64,
    limit: i64,
) -> rusqlite::Result<Vec<ThreadRow>> {
    let sql = format!(
        "WITH sel(id) AS ({UNIFIED_SEL})
         SELECT t.id,
                m.from_name, m.from_addr, m.subject, m.snippet, t.last_date,
                (NOT EXISTS (SELECT 1 FROM messages m3
                             WHERE m3.thread_id = t.id
                               AND m3.folder_id IN (SELECT id FROM sel)
                               AND m3.is_read = 0)),
                t.starred,
                max(m.has_attachments), t.message_count, t.account_id
         FROM threads t
         JOIN messages m ON m.thread_id = t.id
         WHERE m.folder_id IN (SELECT id FROM sel)
           AND m.date = (SELECT max(m2.date) FROM messages m2
                         WHERE m2.thread_id = t.id
                           AND m2.folder_id IN (SELECT id FROM sel))
         GROUP BY t.id
         ORDER BY t.last_date DESC
         LIMIT ?3 OFFSET ?4"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(params![role, label, limit, offset], |r| {
            let from_name: Option<String> = r.get(1)?;
            let from_addr: Option<String> = r.get(2)?;
            Ok(ThreadRow {
                id: r.get(0)?,
                message_id: None,
                account_id: r.get(10)?,
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

/// Individual messages in a virtual folder (all accounts), newest first — the
/// flat (ungrouped) unified view.
pub fn list_unified_messages(
    conn: &Connection,
    role: Option<&str>,
    label: Option<&str>,
    offset: i64,
    limit: i64,
) -> rusqlite::Result<Vec<ThreadRow>> {
    let sql = format!(
        "WITH sel(id) AS ({UNIFIED_SEL})
         SELECT m.thread_id, m.id,
                m.from_name, m.from_addr, m.subject, m.snippet, m.date,
                m.is_read, m.is_starred, m.has_attachments, m.account_id
         FROM messages m
         WHERE m.folder_id IN (SELECT id FROM sel)
         ORDER BY m.date DESC, m.id DESC
         LIMIT ?3 OFFSET ?4"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(params![role, label, limit, offset], |r| {
            let from_name: Option<String> = r.get(2)?;
            let from_addr: Option<String> = r.get(3)?;
            Ok(ThreadRow {
                id: r.get(0)?,
                message_id: Some(r.get(1)?),
                account_id: r.get(10)?,
                from_name: from_name
                    .filter(|s| !s.is_empty())
                    .or_else(|| from_addr.clone())
                    .unwrap_or_default(),
                from_addr: from_addr.unwrap_or_default(),
                subject: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                snippet: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                date: r.get(6)?,
                is_read: r.get(7)?,
                is_starred: r.get(8)?,
                has_attachments: r.get::<_, i64>(9)? != 0,
                message_count: 1,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    /// Two accounts, each with an inbox; a case-variant "Receipts" label pair
    /// and a label only the second account has.
    fn seed(conn: &Connection) -> rusqlite::Result<()> {
        for (id, email) in [("a1", "work@example.com"), ("a2", "nikita@example.com")] {
            conn.execute(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host, created_at)
                 VALUES (?1, ?2, 'custom', 'imap.example.com', 'smtp.example.com', 0)",
                params![id, email],
            )?;
        }
        for (account, imap_name, role, name, sort) in [
            ("a1", "INBOX", Some("inbox"), "Inbox", 0),
            ("a2", "INBOX", Some("inbox"), "Inbox", 0),
            ("a1", "Receipts", None, "Receipts", 50),
            ("a2", "receipts", None, "receipts", 50),
            ("a2", "Solo", None, "Solo", 50),
        ] {
            conn.execute(
                "INSERT INTO folders (account_id, imap_name, role, display_name, unread_count, sort_order)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![account, imap_name, role, name, sort],
            )?;
        }
        Ok(())
    }

    fn folder_id(conn: &Connection, account: &str, imap_name: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND imap_name = ?2",
            params![account, imap_name],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn add_message(
        conn: &mut Connection,
        account: &str,
        folder: i64,
        uid: u32,
        subject: &str,
        date: i64,
        read: bool,
    ) {
        insert_message(
            conn,
            &NewMessage {
                account_id: account.into(),
                folder_id: folder,
                uid,
                message_id: Some(format!("<{account}-{uid}@example.com>")),
                subject: Some(subject.into()),
                from_addr: Some("sender@example.com".into()),
                date,
                is_read: read,
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn unified_folders_merge_roles_and_labels() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            conn.execute_batch(
                "UPDATE folders SET unread_count = 2 WHERE account_id = 'a1' AND role = 'inbox';
                 UPDATE folders SET unread_count = 3 WHERE account_id = 'a2' AND role = 'inbox';",
            )?;
            let folders = list_unified_folders(conn)?;

            // One inbox row summing both accounts, at the fixed virtual id.
            let inboxes: Vec<_> = folders
                .iter()
                .filter(|f| f.role.as_deref() == Some("inbox"))
                .collect();
            assert_eq!(inboxes.len(), 1);
            assert_eq!(inboxes[0].id, -1);
            assert_eq!(inboxes[0].account_id, "*");
            assert_eq!(inboxes[0].unread_count, 5);

            // "Receipts"/"receipts" merge case-insensitively; "Solo" survives.
            let labels: Vec<_> = folders.iter().filter(|f| f.role.is_none()).collect();
            assert_eq!(labels.len(), 2);
            assert!(labels
                .iter()
                .any(|f| f.display_name.eq_ignore_ascii_case("receipts")));
            assert!(labels.iter().any(|f| f.display_name == "Solo"));
            // Virtual label ids are stable, deterministic, and out of the
            // fixed-role range.
            assert_eq!(
                virtual_folder_id(None, "Receipts"),
                virtual_folder_id(None, "receipts")
            );
            assert!(labels.iter().all(|f| f.id <= -1000));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn unified_messages_sort_globally_and_paginate() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            Ok(())
        })
        .unwrap();
        db.with(|conn| {
            let inbox1 = folder_id(conn, "a1", "INBOX");
            let inbox2 = folder_id(conn, "a2", "INBOX");
            // Interleave dates across the two inboxes.
            add_message(conn, "a1", inbox1, 1, "one", 100, true);
            add_message(conn, "a2", inbox2, 1, "two", 200, false);
            add_message(conn, "a1", inbox1, 2, "three", 300, false);
            add_message(conn, "a2", inbox2, 2, "four", 400, true);

            let rows = list_unified_messages(conn, Some("inbox"), None, 0, 10)?;
            assert_eq!(
                rows.iter().map(|r| r.subject.as_str()).collect::<Vec<_>>(),
                ["four", "three", "two", "one"]
            );
            assert_eq!(
                rows.iter()
                    .map(|r| r.account_id.as_str())
                    .collect::<Vec<_>>(),
                ["a2", "a1", "a2", "a1"]
            );

            // Offset pagination continues the same global order.
            let page2 = list_unified_messages(conn, Some("inbox"), None, 2, 2)?;
            assert_eq!(
                page2.iter().map(|r| r.subject.as_str()).collect::<Vec<_>>(),
                ["two", "one"]
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn unified_threads_interleave_accounts() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            Ok(())
        })
        .unwrap();
        db.with(|conn| {
            let inbox1 = folder_id(conn, "a1", "INBOX");
            let inbox2 = folder_id(conn, "a2", "INBOX");
            add_message(conn, "a1", inbox1, 1, "Alpha", 100, false);
            add_message(conn, "a2", inbox2, 1, "Gamma", 200, false);
            add_message(conn, "a1", inbox1, 2, "Beta", 300, true);

            let rows = list_unified_threads(conn, Some("inbox"), None, 0, 10)?;
            assert_eq!(
                rows.iter().map(|r| r.subject.as_str()).collect::<Vec<_>>(),
                ["Beta", "Gamma", "Alpha"]
            );
            assert_eq!(
                rows.iter()
                    .map(|r| r.account_id.as_str())
                    .collect::<Vec<_>>(),
                ["a1", "a2", "a1"]
            );
            assert_eq!(
                rows.iter().map(|r| r.is_read).collect::<Vec<_>>(),
                [true, false, false]
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn unified_label_selector_matches_case_insensitively() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            Ok(())
        })
        .unwrap();
        db.with(|conn| {
            let rec1 = folder_id(conn, "a1", "Receipts");
            let rec2 = folder_id(conn, "a2", "receipts");
            let solo = folder_id(conn, "a2", "Solo");
            add_message(conn, "a1", rec1, 1, "r-one", 100, false);
            add_message(conn, "a2", rec2, 1, "r-two", 200, false);
            add_message(conn, "a2", solo, 1, "s-one", 300, false);

            // The case-variant labels merge into one virtual folder…
            let receipts = list_unified_messages(conn, None, Some("Receipts"), 0, 10)?;
            assert_eq!(
                receipts
                    .iter()
                    .map(|r| r.subject.as_str())
                    .collect::<Vec<_>>(),
                ["r-two", "r-one"]
            );
            // …while a single-account label lists only its owner's mail.
            let solo_rows = list_unified_messages(conn, None, Some("Solo"), 0, 10)?;
            assert_eq!(solo_rows.len(), 1);
            assert_eq!(solo_rows[0].subject, "s-one");
            assert_eq!(solo_rows[0].account_id, "a2");
            Ok(())
        })
        .unwrap();
    }
}
