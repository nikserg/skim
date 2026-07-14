use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Metadata for a file staged on a draft. Deliberately carries no bytes — the
/// frontend only ever needs the name/size to render a chip; the blob is loaded
/// separately at send time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftAttachment {
    pub id: i64,
    pub draft_id: i64,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
}

fn row_to_meta(r: &rusqlite::Row) -> rusqlite::Result<DraftAttachment> {
    Ok(DraftAttachment {
        id: r.get(0)?,
        draft_id: r.get(1)?,
        filename: r.get(2)?,
        mime_type: r.get(3)?,
        size: r.get(4)?,
    })
}

const META_COLS: &str = "id, draft_id, filename, mime_type, size";

pub fn add(
    conn: &Connection,
    draft_id: i64,
    filename: &str,
    mime_type: &str,
    data: &[u8],
) -> rusqlite::Result<DraftAttachment> {
    conn.execute(
        "INSERT INTO draft_attachments (draft_id, filename, mime_type, size, data, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, unixepoch())",
        params![draft_id, filename, mime_type, data.len() as i64, data],
    )?;
    let id = conn.last_insert_rowid();
    conn.query_row(
        &format!("SELECT {META_COLS} FROM draft_attachments WHERE id = ?1"),
        params![id],
        row_to_meta,
    )
}

pub fn list(conn: &Connection, draft_id: i64) -> rusqlite::Result<Vec<DraftAttachment>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {META_COLS} FROM draft_attachments WHERE draft_id = ?1 ORDER BY id"
    ))?;
    let rows = stmt
        .query_map(params![draft_id], row_to_meta)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn remove(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM draft_attachments WHERE id = ?1", params![id])?;
    Ok(())
}

/// Load full attachment payloads (filename, MIME type, bytes) for the send path.
pub fn load_for_send(
    conn: &Connection,
    draft_id: i64,
) -> rusqlite::Result<Vec<(String, String, Vec<u8>)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT filename, mime_type, data FROM draft_attachments WHERE draft_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![draft_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
