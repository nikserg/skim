use crate::error::Result;
use crate::state::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub message_id: i64,
    pub thread_id: Option<i64>,
    pub folder_id: i64,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub date: i64,
    pub snippet: String,
}

/// Free text → quoted, prefix-starred FTS5 terms. Empty for no searchable terms.
fn fts_terms(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect()
}

/// Turn free text into an FTS5 prefix query: each term quoted + starred,
/// terms ANDed. Returns None for input with no searchable terms.
pub fn build_fts_query(input: &str) -> Option<String> {
    let terms = fts_terms(input);
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

/// Like [`build_fts_query`], but terms are OR-joined — matches ANY word.
/// Returns None when the input has fewer than two terms (OR ≡ AND there).
pub fn build_fts_query_any(input: &str) -> Option<String> {
    let terms = fts_terms(input);
    if terms.len() < 2 {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

#[tauri::command]
pub async fn search_messages(
    state: State<'_, AppState>,
    query: String,
    limit: i64,
    account_id: Option<String>,
) -> Result<Vec<SearchHit>> {
    let Some(fts_query) = build_fts_query(&query) else {
        return Ok(Vec::new());
    };
    let limit = limit.clamp(1, 50);
    state
        .db
        .read("search_messages", move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT m.id, m.thread_id, m.folder_id, m.subject, m.from_name, m.from_addr,
                        m.date, snippet(messages_fts, 3, '', '', '…', 12)
                 FROM messages_fts
                 JOIN messages m ON m.id = messages_fts.rowid
                 WHERE messages_fts MATCH ?1
                   AND (?3 IS NULL OR m.account_id = ?3)
                 ORDER BY bm25(messages_fts)
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![fts_query, limit, account_id], |r| {
                    let from_name: Option<String> = r.get(4)?;
                    let from_addr: Option<String> = r.get(5)?;
                    Ok(SearchHit {
                        message_id: r.get(0)?,
                        thread_id: r.get(1)?,
                        folder_id: r.get(2)?,
                        subject: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        from_name: from_name
                            .filter(|s| !s.is_empty())
                            .or_else(|| from_addr.clone())
                            .unwrap_or_default(),
                        from_addr: from_addr.unwrap_or_default(),
                        date: r.get(6)?,
                        snippet: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
}

/// Message ids of a thread — used by keyboard shortcuts that act on the
/// selected list row without loading the full thread detail.
#[tauri::command]
pub async fn thread_message_ids(state: State<'_, AppState>, thread_id: i64) -> Result<Vec<i64>> {
    state
        .db
        .read("thread_message_ids", move |conn| {
            let mut stmt = conn.prepare_cached("SELECT id FROM messages WHERE thread_id = ?1")?;
            let rows = stmt
                .query_map(rusqlite::params![thread_id], |r| r.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
}

/// Message ids of many threads at once — the bulk-selection path. Calling the
/// per-thread command in a loop would cost one IPC round trip per ticked row,
/// and a bulk selection is routinely dozens of them.
#[tauri::command]
pub async fn thread_message_ids_bulk(
    state: State<'_, AppState>,
    thread_ids: Vec<i64>,
) -> Result<Vec<i64>> {
    state
        .db
        .read("thread_message_ids_bulk", move |conn| {
            message_ids_for_threads(conn, &thread_ids)
        })
        .await
}

/// Split out from the command so it can be tested against a seeded database.
fn message_ids_for_threads(
    conn: &rusqlite::Connection,
    thread_ids: &[i64],
) -> rusqlite::Result<Vec<i64>> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }
    // The ids ride in as a JSON array so the SQL text — and with it the prepared
    // statement cache entry — stays the same whatever the count.
    let ids = serde_json::json!(thread_ids).to_string();
    let mut stmt = conn.prepare_cached(
        "SELECT m.id FROM messages m JOIN json_each(?1) j ON j.value = m.thread_id",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![ids], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod thread_id_tests {
    use super::message_ids_for_threads;
    use crate::db::models::NewMessage;
    use crate::db::queries::insert_message;
    use crate::db::Db;

    #[test]
    fn collects_every_message_of_the_named_threads() {
        let db = Db::open_in_memory().unwrap();
        db.with(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host, created_at)
                 VALUES ('a1', 'work@example.com', 'custom', 'i.example.com', 's.example.com', 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO folders (account_id, imap_name, role, display_name, unread_count, sort_order)
                 VALUES ('a1', 'INBOX', 'inbox', 'Inbox', 0, 0)",
                [],
            )?;
            let folder: i64 = conn.query_row("SELECT id FROM folders", [], |r| r.get(0))?;

            let mut ids = Vec::new();
            let mut threads = Vec::new();
            for uid in 1..=4u32 {
                let (id, thread) = insert_message(
                    conn,
                    &NewMessage {
                        account_id: "a1".into(),
                        folder_id: folder,
                        uid,
                        message_id: Some(format!("<{uid}@example.com>")),
                        date: uid as i64,
                        ..Default::default()
                    },
                )?
                .expect("message inserted");
                ids.push(id);
                threads.push(thread);
            }
            // Fold the second message into the first one's conversation, so the
            // set under test is: a two-message thread, a one-message thread, and
            // a third thread we never ask for.
            conn.execute(
                "UPDATE messages SET thread_id = ?1 WHERE id = ?2",
                rusqlite::params![threads[0], ids[1]],
            )?;

            let mut got = message_ids_for_threads(conn, &[threads[0], threads[2]])?;
            got.sort_unstable();
            // Both messages of the first thread, plus the single-message one —
            // and nothing from the thread that was never named.
            let mut want = vec![ids[0], ids[1], ids[2]];
            want.sort_unstable();
            assert_eq!(got, want);

            // No threads means no query and no ids — not "everything".
            assert!(message_ids_for_threads(conn, &[])?.is_empty());
            // An id that matches no thread contributes nothing.
            assert!(message_ids_for_threads(conn, &[999_999])?.is_empty());
            Ok(())
        })
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::{build_fts_query, build_fts_query_any};

    #[test]
    fn builds_prefix_queries() {
        assert_eq!(
            build_fts_query("hello world"),
            Some("\"hello\"* \"world\"*".into())
        );
        assert_eq!(build_fts_query("  "), None);
        // embedded quotes can't break out of the term
        assert_eq!(build_fts_query("a\"b"), Some("\"ab\"*".into()));
    }

    #[test]
    fn builds_any_queries() {
        assert_eq!(
            build_fts_query_any("hello world"),
            Some("\"hello\"* OR \"world\"*".into())
        );
        // fewer than two terms: OR would equal AND, so no fallback query
        assert_eq!(build_fts_query_any("hello"), None);
        assert_eq!(build_fts_query_any("  "), None);
    }
}
