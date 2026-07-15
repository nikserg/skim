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
) -> Result<Vec<SearchHit>> {
    let Some(fts_query) = build_fts_query(&query) else {
        return Ok(Vec::new());
    };
    let limit = limit.clamp(1, 50);
    state
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT m.id, m.thread_id, m.folder_id, m.subject, m.from_name, m.from_addr,
                        m.date, snippet(messages_fts, 3, '', '', '…', 12)
                 FROM messages_fts
                 JOIN messages m ON m.id = messages_fts.rowid
                 WHERE messages_fts MATCH ?1
                 ORDER BY bm25(messages_fts)
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![fts_query, limit], |r| {
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
        .call(move |conn| {
            let mut stmt = conn.prepare_cached("SELECT id FROM messages WHERE thread_id = ?1")?;
            let rows = stmt
                .query_map(rusqlite::params![thread_id], |r| r.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
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
