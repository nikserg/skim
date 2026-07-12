use super::models::Account;
use rusqlite::{params, Connection, OptionalExtension};

fn row_to_account(r: &rusqlite::Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get(0)?,
        email: r.get(1)?,
        display_name: r.get(2)?,
        provider: r.get(3)?,
        imap_host: r.get(4)?,
        imap_port: r.get::<_, i64>(5)? as u16,
        smtp_host: r.get(6)?,
        smtp_port: r.get::<_, i64>(7)? as u16,
        smtp_security: r.get(8)?,
        auth_kind: r.get(9)?,
    })
}

const COLS: &str = "id, email, display_name, provider, imap_host, imap_port, smtp_host, smtp_port, smtp_security, auth_kind";

pub fn insert(conn: &Connection, a: &Account) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO accounts (id, email, display_name, provider, imap_host, imap_port,
                               smtp_host, smtp_port, smtp_security, auth_kind, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, unixepoch())",
        params![
            a.id,
            a.email,
            a.display_name,
            a.provider,
            a.imap_host,
            a.imap_port,
            a.smtp_host,
            a.smtp_port,
            a.smtp_security,
            a.auth_kind,
        ],
    )?;
    Ok(())
}

pub fn list(conn: &Connection) -> rusqlite::Result<Vec<Account>> {
    let mut stmt =
        conn.prepare_cached(&format!("SELECT {COLS} FROM accounts ORDER BY created_at"))?;
    let rows = stmt
        .query_map([], row_to_account)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Account>> {
    conn.query_row(
        &format!("SELECT {COLS} FROM accounts WHERE id = ?1"),
        params![id],
        row_to_account,
    )
    .optional()
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    // FTS rows are contentless and don't cascade — clear them first.
    conn.execute(
        "DELETE FROM messages_fts WHERE rowid IN
           (SELECT id FROM messages WHERE account_id = ?1)",
        params![id],
    )?;
    conn.execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
    Ok(())
}
