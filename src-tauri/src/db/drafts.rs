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
    })
}

const COLS: &str =
    "id, account_id, reply_to_message_id, mode, to_addrs, cc_addrs, bcc_addrs, subject, body_text";

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
