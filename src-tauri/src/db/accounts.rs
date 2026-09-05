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
        signature: r.get(10)?,
    })
}

const COLS: &str = "id, email, display_name, provider, imap_host, imap_port, smtp_host, smtp_port, smtp_security, auth_kind, signature";

pub fn insert(conn: &Connection, a: &Account) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO accounts (id, email, display_name, provider, imap_host, imap_port,
                               smtp_host, smtp_port, smtp_security, auth_kind, signature,
                               created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, unixepoch())",
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
            a.signature,
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

/// Marks this account's name as settled: either the sync engine has already
/// taken its one guess from Sent mail, or the user has edited the field by
/// hand. Either way the guess never runs again — a name the user deliberately
/// cleared reappearing on the next sync would be the app arguing with them.
fn settled_key(account_id: &str) -> String {
    format!("identity_settled_{account_id}")
}

pub fn identity_settled(conn: &Connection, account_id: &str) -> rusqlite::Result<bool> {
    Ok(super::queries::get_setting(conn, &settled_key(account_id))?.is_some())
}

pub fn mark_identity_settled(conn: &Connection, account_id: &str) -> rusqlite::Result<()> {
    super::queries::set_setting(conn, &settled_key(account_id), "1")
}

/// Adopt a name learned from the mailbox's own sent mail. Touches nothing else
/// — a signature the user has already written is not ours to overwrite — and
/// settles the field so the guess is never made twice.
///
/// The settle check and the write happen together, on one connection, because
/// scanning Sent takes long enough for the user to have opened Settings and
/// typed (or cleared) a name in the meantime. Whoever they are, they win:
/// `false` means the guess was dropped and nothing was written.
pub fn adopt_display_name(conn: &Connection, id: &str, name: &str) -> rusqlite::Result<bool> {
    if identity_settled(conn, id)? {
        return Ok(false);
    }
    let wrote = conn.execute(
        "UPDATE accounts SET display_name = ?2 WHERE id = ?1 AND display_name IS NULL",
        params![id, name],
    )?;
    // Settled either way: a name that arrived while we scanned is still a name,
    // and re-guessing over it later would be the same mistake one sweep later.
    mark_identity_settled(conn, id)?;
    Ok(wrote > 0)
}

/// The two fields the user owns. Server settings stay insert-once — changing a
/// host mid-flight would strand the running sync engine and the stored secret.
///
/// Blank input is stored as NULL, so every caller downstream only has to ask
/// whether the value is there, never whether it is there but empty.
pub fn update_identity(
    conn: &Connection,
    id: &str,
    display_name: Option<&str>,
    signature: Option<&str>,
) -> rusqlite::Result<()> {
    fn clean(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    let name = clean(display_name);
    // Settings saves both fields on one blur, so a save is not by itself an
    // answer about the name: someone writing only a signature left the name box
    // empty because we hadn't guessed yet, not to refuse a name. Settle when the
    // name was actually decided — typed, or cleared after having been there.
    let had_name = get(conn, id)?.and_then(|a| a.display_name).is_some();
    conn.execute(
        "UPDATE accounts SET display_name = ?2, signature = ?3 WHERE id = ?1",
        params![id, name, clean(signature)],
    )?;
    if name.is_some() || had_name {
        mark_identity_settled(conn, id)?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use crate::db::Db;

    fn seed(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO accounts (id, email, provider, imap_host, smtp_host, created_at)
             VALUES ('acc1', 'me@example.com', 'custom', 'imap.example.com', 'smtp.example.com', 0)",
            [],
        )?;
        Ok(())
    }

    #[test]
    fn adopting_a_name_leaves_the_signature_alone() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            super::update_identity(conn, "acc1", None, Some("Jane, Acme"))?;
            super::adopt_display_name(conn, "acc1", "Jane Doe")?;

            let a = super::get(conn, "acc1")?.expect("account");
            assert_eq!(a.display_name.as_deref(), Some("Jane Doe"));
            // The sign-off is the user's; a guess about the name must not eat it.
            assert_eq!(a.signature.as_deref(), Some("Jane, Acme"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_cleared_name_is_never_guessed_again() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            assert!(!super::identity_settled(conn, "acc1")?);

            // Clearing a name that was there is a decision, not an absence of one.
            super::update_identity(conn, "acc1", Some("Jane Doe"), None)?;
            super::update_identity(conn, "acc1", Some("  "), None)?;
            let a = super::get(conn, "acc1")?.expect("account");
            assert_eq!(a.display_name, None);
            assert!(super::identity_settled(conn, "acc1")?);
            assert!(!super::adopt_display_name(conn, "acc1", "Jane Doe")?);
            Ok(())
        })
        .unwrap();
    }

    /// Settings saves name and signature on one blur. Writing only a signature
    /// before the mailbox has ever been scanned must not read as "I want no
    /// name" — that would be #43 all over again, silently.
    #[test]
    fn writing_only_a_signature_still_allows_the_guess() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            super::update_identity(conn, "acc1", Some(""), Some("Jane, Acme"))?;
            assert!(!super::identity_settled(conn, "acc1")?);
            assert!(super::adopt_display_name(conn, "acc1", "Jane Doe")?);
            let a = super::get(conn, "acc1")?.expect("account");
            assert_eq!(a.display_name.as_deref(), Some("Jane Doe"));
            assert_eq!(a.signature.as_deref(), Some("Jane, Acme"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn one_guess_per_mailbox_ever() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            assert!(super::adopt_display_name(conn, "acc1", "Jane Doe")?);
            assert!(super::identity_settled(conn, "acc1")?);
            // A second sweep must not re-guess over the first answer.
            assert!(!super::adopt_display_name(conn, "acc1", "Someone Else")?);
            assert_eq!(
                super::get(conn, "acc1")?
                    .expect("account")
                    .display_name
                    .as_deref(),
                Some("Jane Doe")
            );
            Ok(())
        })
        .unwrap();
    }

    /// Scanning Sent is slow enough that the user can answer first. Whatever
    /// they chose — a name of their own, or none at all — outranks the guess.
    #[test]
    fn a_name_chosen_while_the_scan_ran_wins() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            super::update_identity(conn, "acc1", Some("J. Doe"), None)?;
            assert!(!super::adopt_display_name(conn, "acc1", "Jane Doe")?);
            assert_eq!(
                super::get(conn, "acc1")?
                    .expect("account")
                    .display_name
                    .as_deref(),
                Some("J. Doe")
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn blank_identity_fields_are_stored_as_absent() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            seed(conn)?;
            super::update_identity(conn, "acc1", Some(""), Some("   "))?;
            let a = super::get(conn, "acc1")?.expect("account");
            assert_eq!(a.display_name, None);
            assert_eq!(a.signature, None);
            // …and surrounding whitespace never reaches the wire.
            super::update_identity(conn, "acc1", Some("  Jane  "), None)?;
            assert_eq!(
                super::get(conn, "acc1")?
                    .expect("account")
                    .display_name
                    .as_deref(),
                Some("Jane")
            );
            Ok(())
        })
        .unwrap();
    }
}
