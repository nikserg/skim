use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: i64,
    pub account_id: String,
    pub reply_to_message_id: Option<i64>,
    pub mode: String, // 'new' | 'reply' | 'reply_all' | 'forward'
    /// Recipient fields are the raw comma-separated strings the user typed;
    /// they are parsed into addresses only at send time.
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    /// The local `messages.id` this draft mirrors, when it was opened from the
    /// IMAP Drafts folder; `None` for ordinary local-only drafts. Set once at
    /// creation and never overwritten by `update`.
    pub origin_message_id: Option<i64>,
}

fn row_to_draft(r: &rusqlite::Row) -> rusqlite::Result<Draft> {
    Ok(Draft {
        id: r.get(0)?,
        account_id: r.get(1)?,
        reply_to_message_id: r.get(2)?,
        mode: r.get(3)?,
        to: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        cc: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
        bcc: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        subject: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
        body: r.get::<_, Option<String>>(8)?.unwrap_or_default(),
        origin_message_id: r.get(9)?,
    })
}

const COLS: &str = "id, account_id, reply_to_message_id, mode, to_addrs, cc_addrs, bcc_addrs, \
     subject, body_text, origin_message_id";

pub fn create(
    conn: &Connection,
    account_id: &str,
    mode: &str,
    reply_to: Option<i64>,
    to: &str,
    subject: &str,
    body: &str,
) -> rusqlite::Result<Draft> {
    conn.execute(
        "INSERT INTO drafts (account_id, reply_to_message_id, mode, to_addrs, cc_addrs, bcc_addrs,
                             subject, body_text, updated_at)
         VALUES (?1, ?2, ?3, ?4, '', '', ?5, ?6, unixepoch())",
        params![account_id, reply_to, mode, to, subject, body],
    )?;
    let id = conn.last_insert_rowid();
    get(conn, id).map(|d| d.expect("just inserted"))
}

/// Create a local draft that mirrors a message in the IMAP Drafts folder.
/// Unlike [`create`], it carries the full recipient set plus the link back to
/// the server copy (`origin_message_id` + the stable `imap_message_id`).
#[allow(clippy::too_many_arguments)]
pub fn create_server_draft(
    conn: &Connection,
    account_id: &str,
    mode: &str,
    reply_to: Option<i64>,
    to: &str,
    cc: &str,
    bcc: &str,
    subject: &str,
    body: &str,
    origin_message_id: i64,
    imap_message_id: &str,
) -> rusqlite::Result<Draft> {
    conn.execute(
        "INSERT INTO drafts (account_id, reply_to_message_id, mode, to_addrs, cc_addrs, bcc_addrs,
                             subject, body_text, origin_message_id, imap_message_id, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, unixepoch())",
        params![
            account_id,
            reply_to,
            mode,
            to,
            cc,
            bcc,
            subject,
            body,
            origin_message_id,
            imap_message_id
        ],
    )?;
    let id = conn.last_insert_rowid();
    get(conn, id).map(|d| d.expect("just inserted"))
}

/// The draft (if any) currently linked to a given local `messages.id`.
pub fn find_by_origin(
    conn: &Connection,
    origin_message_id: i64,
) -> rusqlite::Result<Option<Draft>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM drafts WHERE origin_message_id = ?1"),
        params![origin_message_id],
        row_to_draft,
    )
    .optional()
}

/// The draft (if any) mirroring the server copy with this stable Message-ID.
/// Used to dedup across resyncs, where `origin_message_id` may point at a row
/// that reconciliation has since replaced.
pub fn find_by_imap_message_id(
    conn: &Connection,
    imap_message_id: &str,
) -> rusqlite::Result<Option<Draft>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM drafts WHERE imap_message_id = ?1"),
        params![imap_message_id],
        row_to_draft,
    )
    .optional()
}

/// Re-point a draft at the live local message that now represents its server
/// copy (after a resync swapped the underlying row).
pub fn relink_origin(
    conn: &Connection,
    draft_id: i64,
    origin_message_id: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE drafts SET origin_message_id = ?2 WHERE id = ?1",
        params![draft_id, origin_message_id],
    )?;
    Ok(())
}

/// Ensure the draft carries a stable Message-ID for its server copy, minting the
/// given `candidate` only when none is set yet. Returns the effective value.
/// Lets a local-only draft become addressable in the IMAP Drafts folder without
/// disturbing a server-backed draft that already has one.
pub fn ensure_imap_message_id(
    conn: &Connection,
    draft_id: i64,
    candidate: &str,
) -> rusqlite::Result<String> {
    conn.execute(
        "UPDATE drafts SET imap_message_id = ?2 WHERE id = ?1 AND imap_message_id IS NULL",
        params![draft_id, candidate],
    )?;
    conn.query_row(
        "SELECT imap_message_id FROM drafts WHERE id = ?1",
        params![draft_id],
        |r| r.get(0),
    )
}

/// The server-copy coordinates of a draft: `(origin_message_id, imap_message_id)`.
/// Both `None`/absent for ordinary local drafts.
pub fn origin_coords(
    conn: &Connection,
    draft_id: i64,
) -> rusqlite::Result<Option<(Option<i64>, Option<String>)>> {
    conn.query_row(
        "SELECT origin_message_id, imap_message_id FROM drafts WHERE id = ?1",
        params![draft_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Draft>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM drafts WHERE id = ?1"),
        params![id],
        row_to_draft,
    )
    .optional()
}

pub fn update(conn: &Connection, draft: &Draft) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE drafts SET to_addrs = ?2, cc_addrs = ?3, bcc_addrs = ?4, subject = ?5,
                           body_text = ?6, updated_at = unixepoch()
         WHERE id = ?1",
        params![
            draft.id,
            draft.to,
            draft.cc,
            draft.bcc,
            draft.subject,
            draft.body
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM drafts WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list(conn: &Connection, account_id: &str) -> rusqlite::Result<Vec<Draft>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLS} FROM drafts WHERE account_id = ?1 ORDER BY updated_at DESC"
    ))?;
    let rows = stmt
        .query_map(params![account_id], row_to_draft)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use crate::db::Db;

    #[test]
    fn server_draft_roundtrips_and_dedups() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host, created_at)
                 VALUES ('acc1', 'me@example.com', 'custom', 'imap.example.com', 'smtp.example.com', 0)",
                [],
            )?;

            let d = super::create_server_draft(
                conn, "acc1", "new", None, "you@example.com", "", "", "Re: hi", "body",
                42, "abc@host",
            )?;
            assert_eq!(d.origin_message_id, Some(42));
            assert_eq!(d.to, "you@example.com");

            // Reopen dedups by the origin message id and by the stable Message-ID.
            assert_eq!(super::find_by_origin(conn, 42)?.unwrap().id, d.id);
            assert_eq!(super::find_by_imap_message_id(conn, "abc@host")?.unwrap().id, d.id);

            // update() must not clobber the server-copy link.
            let mut edited = d.clone();
            edited.subject = "changed".into();
            super::update(conn, &edited)?;
            let (origin, mid) = super::origin_coords(conn, d.id)?.unwrap();
            assert_eq!(origin, Some(42));
            assert_eq!(mid.as_deref(), Some("abc@host"));
            assert_eq!(super::get(conn, d.id)?.unwrap().subject, "changed");

            // relink after a resync swaps the underlying row.
            super::relink_origin(conn, d.id, 99)?;
            assert!(super::find_by_origin(conn, 42)?.is_none());
            assert_eq!(super::find_by_origin(conn, 99)?.unwrap().id, d.id);
            Ok(())
        })
        .unwrap();
    }
}
