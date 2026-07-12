//! Mailbox retrieval for the command-palette chat: FTS query from the
//! question, top-N messages by bm25, rendered as numbered context blocks.

use crate::ai::prompts::EmailBlock;
use rusqlite::Connection;
use serde::Serialize;

const TOP_N: usize = 12;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub index: usize,
    pub message_id: i64,
    pub thread_id: Option<i64>,
    pub folder_id: i64,
    pub subject: String,
    pub from: String,
}

/// Small multilingual stopword list — enough to keep FTS queries useful.
const STOPWORDS: &[&str] = &[
    "the",
    "a",
    "an",
    "is",
    "are",
    "was",
    "were",
    "do",
    "does",
    "did",
    "what",
    "when",
    "who",
    "whom",
    "which",
    "how",
    "why",
    "where",
    "me",
    "my",
    "i",
    "you",
    "your",
    "we",
    "our",
    "of",
    "to",
    "in",
    "on",
    "at",
    "for",
    "from",
    "with",
    "about",
    "and",
    "or",
    "any",
    "all",
    "have",
    "has",
    "had",
    "this",
    "that",
    "these",
    "those",
    "there",
    "still",
    "last",
    "next",
    "week",
    "month",
    "что",
    "как",
    "когда",
    "кто",
    "где",
    "почему",
    "мне",
    "мои",
    "мой",
    "моя",
    "я",
    "ты",
    "вы",
    "мы",
    "наш",
    "в",
    "на",
    "о",
    "об",
    "от",
    "с",
    "со",
    "и",
    "или",
    "все",
    "есть",
    "это",
    "этот",
    "эта",
    "эти",
    "ли",
    "не",
    "за",
    "по",
    "у",
    "к",
    "из",
];

pub fn question_to_fts(question: &str) -> Option<String> {
    let terms: Vec<String> = question
        .split(|c: char| !c.is_alphanumeric() && c != '@' && c != '.')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| t.len() >= 2 && !STOPWORDS.contains(&t.as_str()))
        .map(|t| format!("\"{t}\""))
        .collect();
    if terms.is_empty() {
        None
    } else {
        // OR — recall over precision; bm25 ranks the good hits up.
        Some(terms.join(" OR "))
    }
}

pub struct RetrievedEmail {
    pub block: EmailBlock,
    pub citation: Citation,
}

pub fn retrieve(
    conn: &Connection,
    question: &str,
    extra_message_id: Option<i64>,
) -> rusqlite::Result<Vec<RetrievedEmail>> {
    let mut out: Vec<RetrievedEmail> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    type MetaRow = (
        Option<i64>,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let push_message = |conn: &Connection,
                        message_id: i64,
                        out: &mut Vec<RetrievedEmail>,
                        seen: &mut std::collections::HashSet<i64>|
     -> rusqlite::Result<()> {
        if !seen.insert(message_id) {
            return Ok(());
        }
        use rusqlite::OptionalExtension;
        let row: Option<MetaRow> = conn
            .query_row(
                "SELECT thread_id, folder_id, subject, from_name, from_addr, date
                 FROM messages WHERE id = ?1",
                rusqlite::params![message_id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((thread_id, folder_id, subject, from_name, from_addr, date)) = row else {
            return Ok(());
        };
        let body: Option<String> = conn
            .query_row(
                "SELECT COALESCE(body_text, '') FROM message_bodies WHERE message_id = ?1",
                rusqlite::params![message_id],
                |r| r.get(0),
            )
            .optional()?;
        let snippet: Option<String> = conn
            .query_row(
                "SELECT snippet FROM messages WHERE id = ?1",
                rusqlite::params![message_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();

        let from = match (&from_name, &from_addr) {
            (Some(n), Some(a)) if !n.is_empty() => format!("{n} <{a}>"),
            (_, Some(a)) => a.clone(),
            _ => "unknown".into(),
        };
        let index = out.len() + 1;
        out.push(RetrievedEmail {
            block: EmailBlock {
                from: from.clone(),
                date: format_date(date),
                subject: subject.clone().unwrap_or_default(),
                // Headers-only messages still contribute their snippet.
                body: body
                    .filter(|b| !b.is_empty())
                    .or(snippet)
                    .unwrap_or_default(),
            },
            citation: Citation {
                index,
                message_id,
                thread_id,
                folder_id,
                subject: subject.unwrap_or_default(),
                from,
            },
        });
        Ok(())
    };

    if let Some(id) = extra_message_id {
        push_message(conn, id, &mut out, &mut seen)?;
    }

    if let Some(fts) = question_to_fts(question) {
        let ids: Vec<i64> = {
            let mut stmt = conn.prepare_cached(
                "SELECT messages_fts.rowid FROM messages_fts
                 WHERE messages_fts MATCH ?1
                 ORDER BY bm25(messages_fts) LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![fts, TOP_N as i64], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for id in ids {
            if out.len() >= TOP_N {
                break;
            }
            push_message(conn, id, &mut out, &mut seen)?;
        }
    }

    Ok(out)
}

pub fn format_date(unix: i64) -> String {
    let days = unix / 86400;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let (y, m) = if m <= 2 { (y + 1, m) } else { (y, m) };
    format!("{y}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::question_to_fts;

    #[test]
    fn builds_or_queries_without_stopwords() {
        let q = question_to_fts("What invoices are still unpaid this month?").unwrap();
        assert!(q.contains("\"invoices\""));
        assert!(q.contains("\"unpaid\""));
        assert!(!q.contains("\"what\""));
        assert!(q.contains(" OR "));
    }

    #[test]
    fn empty_for_stopwords_only() {
        assert_eq!(question_to_fts("what is this"), None);
    }
}
