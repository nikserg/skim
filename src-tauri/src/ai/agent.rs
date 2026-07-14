//! Agentic tool-calling loop for the mailbox-wide chat (`ai_chat`).
//!
//! Instead of a fixed retrieve-then-answer step, the model drives its own
//! retrieval through two tools — `search_emails` and `read_email` — and we
//! stream its reasoning trace (which tool it called, with what) plus the
//! answer. Works for both providers: a provider-neutral transcript is
//! serialized to each provider's wire format before every round.

use crate::ai::retrieval::{format_date, Citation};
use crate::ai::{anthropic, openrouter, prompts, AssistantTurn, ToolCall};
use crate::commands::search::build_fts_query;
use crate::db::{bodies, Db};
use crate::error::Result;
use crate::mail::sync::SyncHandle;
use chrono::TimeZone;
use rusqlite::types::Value as SqlValue;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MAX_ROUNDS: usize = 6;
const MAX_TOOL_CALLS: usize = 12;
const READ_EMAIL_BUDGET: usize = 6_000;
const CONTEXT_EMAIL_BUDGET: usize = 2_000;
const SNIPPET_MAX: usize = 160;
const SEARCH_LIMIT_DEFAULT: i64 = 10;
const SEARCH_LIMIT_MAX: i64 = 25;

#[derive(Clone, Copy)]
pub enum Provider {
    Anthropic,
    OpenRouter,
}

/// Owned handles the loop needs; snapshot before spawning so it never has to
/// hold the non-`'static` Tauri `State`.
pub struct AgentDeps {
    pub db: Db,
    pub engines: HashMap<String, SyncHandle>,
}

// ---- citation registry ----------------------------------------------------

/// Assigns a stable 1-based `[N]` to every email the model sees, deduped by
/// message id so a re-found email keeps its number.
struct Registry {
    citations: Vec<Citation>,
    by_message: HashMap<i64, usize>,
}

impl Registry {
    fn new() -> Self {
        Self {
            citations: Vec::new(),
            by_message: HashMap::new(),
        }
    }

    fn assign(&mut self, message_id: i64, make: impl FnOnce(usize) -> Citation) -> usize {
        if let Some(&idx) = self.by_message.get(&message_id) {
            return idx;
        }
        let idx = self.citations.len() + 1;
        self.citations.push(make(idx));
        self.by_message.insert(message_id, idx);
        idx
    }

    fn citation(&self, index: usize) -> Option<&Citation> {
        self.citations.get(index.checked_sub(1)?)
    }
}

// ---- provider-neutral transcript ------------------------------------------

enum Turn {
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    ToolResults(Vec<ToolResult>),
}

struct ToolResult {
    id: String,
    content: String,
    is_error: bool,
}

fn anthropic_messages(turns: &[Turn]) -> Vec<Value> {
    turns
        .iter()
        .map(|t| match t {
            Turn::User(text) => json!({ "role": "user", "content": text }),
            Turn::Assistant { text, tool_calls } => {
                let mut content = Vec::new();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                for tc in tool_calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.input,
                    }));
                }
                json!({ "role": "assistant", "content": content })
            }
            Turn::ToolResults(results) => {
                let content: Vec<Value> = results
                    .iter()
                    .map(|r| {
                        let mut block = json!({
                            "type": "tool_result",
                            "tool_use_id": r.id,
                            "content": r.content,
                        });
                        if r.is_error {
                            block["is_error"] = json!(true);
                        }
                        block
                    })
                    .collect();
                json!({ "role": "user", "content": content })
            }
        })
        .collect()
}

fn openai_messages(turns: &[Turn]) -> Vec<Value> {
    let mut out = Vec::new();
    for t in turns {
        match t {
            Turn::User(text) => out.push(json!({ "role": "user", "content": text })),
            Turn::Assistant { text, tool_calls } => {
                let mut msg = json!({ "role": "assistant" });
                msg["content"] = if text.is_empty() {
                    Value::Null
                } else {
                    json!(text)
                };
                if !tool_calls.is_empty() {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.input.to_string(),
                                },
                            })
                        })
                        .collect();
                    msg["tool_calls"] = json!(calls);
                }
                out.push(msg);
            }
            Turn::ToolResults(results) => {
                for r in results {
                    out.push(json!({
                        "role": "tool",
                        "tool_call_id": r.id,
                        "content": r.content,
                    }));
                }
            }
        }
    }
    out
}

// ---- tool definitions -----------------------------------------------------

const SEARCH_DESC: &str = "Search the user's mailbox. Combine any of: keyword (matches \
    subject/sender/body), sender substring, subject substring, folder, and a date range. \
    Leave `query` empty to list the most recent emails by date (use this for \"last month\" / \
    summary questions, together with `after`). Returns compact rows, each tagged [N].";
const READ_DESC: &str = "Read the full body of a search result, identified by its [N] ref number.";

fn search_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query":   { "type": "string", "description": "keywords; leave empty to list by date only" },
            "from":    { "type": "string", "description": "sender name or address substring" },
            "subject": { "type": "string", "description": "subject substring" },
            "after":   { "type": "string", "description": "only emails on/after this date, YYYY-MM-DD" },
            "before":  { "type": "string", "description": "only emails on/before this date, YYYY-MM-DD" },
            "folder":  { "type": "string", "description": "restrict to a folder role: inbox, sent, archive, trash, junk, drafts, starred" },
            "limit":   { "type": "integer", "description": "max results, 1-25 (default 10)" }
        }
    })
}

fn read_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "ref": { "type": "integer", "description": "the [N] number of a search result" }
        },
        "required": ["ref"]
    })
}

fn tools_for(provider: Provider) -> Vec<Value> {
    match provider {
        Provider::Anthropic => vec![
            json!({ "name": "search_emails", "description": SEARCH_DESC, "input_schema": search_schema() }),
            json!({ "name": "read_email", "description": READ_DESC, "input_schema": read_schema() }),
        ],
        Provider::OpenRouter => vec![
            json!({ "type": "function", "function": { "name": "search_emails", "description": SEARCH_DESC, "parameters": search_schema() } }),
            json!({ "type": "function", "function": { "name": "read_email", "description": READ_DESC, "parameters": read_schema() } }),
        ],
    }
}

// ---- the loop -------------------------------------------------------------

/// Run the agent. Streams answer text via `on_delta`, and a per-tool trace via
/// `on_tool_call` (id, kind `"search"`/`"read"`, human arg) and `on_tool_done`
/// (id, email count for searches). Returns the emails actually cited in the
/// answer.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    provider: Provider,
    key: String,
    model: String,
    system: String,
    question: String,
    context_message_id: Option<i64>,
    deps: AgentDeps,
    on_delta: &mut impl FnMut(&str),
    on_tool_call: &impl Fn(&str, &str, &str),
    on_tool_done: &impl Fn(&str, Option<u32>),
) -> Result<Vec<Citation>> {
    let mut reg = Registry::new();
    let mut turns: Vec<Turn> = Vec::new();

    // Fold the currently-open email (if any) into the first user turn as context.
    let first = match context_message_id {
        Some(id) => match email_block_owned(&deps.db, &deps.engines, id).await {
            Ok(b) => format!(
                "The user is looking at this email:\n--- From: {} | Date: {} | Subject: {} ---\n{}\n\nTheir question: {}",
                b.from,
                b.date,
                b.subject,
                prompts::truncate(&b.body, CONTEXT_EMAIL_BUDGET),
                question
            ),
            Err(_) => question.clone(),
        },
        None => question.clone(),
    };
    turns.push(Turn::User(first));

    let mut full_text = String::new();
    let mut tool_calls_used = 0usize;

    for round in 0..MAX_ROUNDS {
        // On the last allowed round, or once the tool budget is spent, drop the
        // tools so the model must answer from what it already gathered.
        let force_final = tool_calls_used >= MAX_TOOL_CALLS || round == MAX_ROUNDS - 1;
        let tools = if force_final {
            Vec::new()
        } else {
            tools_for(provider)
        };

        let turn = call_provider(
            provider,
            &key,
            &model,
            &system,
            &turns,
            tools,
            on_delta,
            &mut full_text,
        )
        .await?;

        let wants_tools =
            turn.stop_reason.as_deref() == Some("tool_use") && !turn.tool_calls.is_empty();
        if !wants_tools {
            break;
        }

        turns.push(Turn::Assistant {
            text: turn.text.clone(),
            tool_calls: turn.tool_calls.clone(),
        });

        let mut results = Vec::with_capacity(turn.tool_calls.len());
        for tc in &turn.tool_calls {
            tool_calls_used += 1;
            let (kind, arg) = describe(&reg, tc);
            on_tool_call(&tc.id, kind, &arg);
            let (content, count, is_error) = exec_tool(&deps, &mut reg, tc).await;
            on_tool_done(&tc.id, count);
            results.push(ToolResult {
                id: tc.id.clone(),
                content,
                is_error,
            });
        }
        turns.push(Turn::ToolResults(results));
    }

    let cited = cited_indices(&full_text);
    let out = reg
        .citations
        .into_iter()
        .filter(|c| cited.contains(&c.index))
        .collect();
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
async fn call_provider(
    provider: Provider,
    key: &str,
    model: &str,
    system: &str,
    turns: &[Turn],
    tools: Vec<Value>,
    on_delta: &mut impl FnMut(&str),
    full_text: &mut String,
) -> Result<AssistantTurn> {
    let mut sink = |d: &str| {
        full_text.push_str(d);
        on_delta(d);
    };
    match provider {
        Provider::Anthropic => {
            let req = anthropic::ToolRequest {
                model: model.to_string(),
                system: system.to_string(),
                messages: anthropic_messages(turns),
                tools,
                max_tokens: 4096,
            };
            anthropic::stream_tools(key, &req, &mut sink).await
        }
        Provider::OpenRouter => {
            let req = openrouter::ToolRequest {
                model: model.to_string(),
                system: system.to_string(),
                messages: openai_messages(turns),
                tools,
                max_tokens: 4096,
            };
            openrouter::stream_tools(key, &req, &mut sink).await
        }
    }
}

/// A short human label for the reasoning trace.
fn describe(reg: &Registry, tc: &ToolCall) -> (&'static str, String) {
    match tc.name.as_str() {
        "read_email" => {
            let arg = tc
                .input
                .get("ref")
                .and_then(Value::as_i64)
                .and_then(|r| reg.citation(r as usize))
                .map(|c| {
                    if c.subject.is_empty() {
                        c.from.clone()
                    } else {
                        c.subject.clone()
                    }
                })
                .unwrap_or_default();
            ("read", arg)
        }
        _ => {
            let mut parts = Vec::new();
            if let Some(q) = str_opt(&tc.input, "query") {
                parts.push(q);
            }
            if let Some(f) = str_opt(&tc.input, "from") {
                parts.push(format!("from {f}"));
            }
            if let Some(s) = str_opt(&tc.input, "subject") {
                parts.push(format!("subject {s}"));
            }
            let after = str_opt(&tc.input, "after");
            let before = str_opt(&tc.input, "before");
            match (after, before) {
                (Some(a), Some(b)) => parts.push(format!("{a}…{b}")),
                (Some(a), None) => parts.push(format!("since {a}")),
                (None, Some(b)) => parts.push(format!("until {b}")),
                (None, None) => {}
            }
            let arg = if parts.is_empty() {
                "recent".to_string()
            } else {
                parts.join(", ")
            };
            ("search", arg)
        }
    }
}

async fn exec_tool(
    deps: &AgentDeps,
    reg: &mut Registry,
    tc: &ToolCall,
) -> (String, Option<u32>, bool) {
    match tc.name.as_str() {
        "search_emails" => match search_emails(deps, reg, &tc.input).await {
            Ok((text, count)) => (text, Some(count), false),
            Err(e) => (format!("search failed: {e}"), None, true),
        },
        "read_email" => match read_email(deps, reg, &tc.input).await {
            Ok(text) => (text, None, false),
            Err(e) => (format!("read failed: {e}"), None, true),
        },
        other => (format!("unknown tool: {other}"), None, true),
    }
}

// ---- tools ----------------------------------------------------------------

struct RowData {
    id: i64,
    thread_id: Option<i64>,
    folder_id: i64,
    subject: String,
    from_name: Option<String>,
    from_addr: Option<String>,
    date: i64,
    snippet: String,
}

async fn search_emails(
    deps: &AgentDeps,
    reg: &mut Registry,
    input: &Value,
) -> Result<(String, u32)> {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let from = str_opt(input, "from");
    let subject = str_opt(input, "subject");
    let folder = str_opt(input, "folder");
    let after = str_opt(input, "after").and_then(|s| parse_day_start(&s));
    let before = str_opt(input, "before").and_then(|s| parse_day_end(&s));
    let limit = input
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(SEARCH_LIMIT_DEFAULT)
        .clamp(1, SEARCH_LIMIT_MAX);

    let fts = if query.is_empty() {
        None
    } else {
        build_fts_query(&query)
    };

    // Shared filter clauses + params, in `?`-order.
    let mut clauses: Vec<&str> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();
    if let Some(a) = after {
        clauses.push("m.date >= ?");
        params.push(SqlValue::Integer(a));
    }
    if let Some(b) = before {
        clauses.push("m.date < ?");
        params.push(SqlValue::Integer(b));
    }
    if let Some(f) = &from {
        clauses.push("(m.from_name LIKE ? OR m.from_addr LIKE ?)");
        let like = format!("%{f}%");
        params.push(SqlValue::Text(like.clone()));
        params.push(SqlValue::Text(like));
    }
    if let Some(s) = &subject {
        clauses.push("m.subject LIKE ?");
        params.push(SqlValue::Text(format!("%{s}%")));
    }
    if let Some(fr) = &folder {
        clauses.push("m.folder_id IN (SELECT id FROM folders WHERE role = ?)");
        params.push(SqlValue::Text(fr.clone()));
    }
    let filter_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" AND {}", clauses.join(" AND "))
    };

    const COLS: &str = "m.id, m.thread_id, m.folder_id, COALESCE(m.subject,''), \
        m.from_name, m.from_addr, m.date, COALESCE(m.snippet,'')";

    let (sql, ordered): (String, Vec<SqlValue>) = if let Some(fts) = fts {
        let sql = format!(
            "SELECT {COLS} FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid \
             WHERE messages_fts MATCH ?{filter_sql} ORDER BY bm25(messages_fts) LIMIT ?"
        );
        let mut p = vec![SqlValue::Text(fts)];
        p.extend(params);
        p.push(SqlValue::Integer(limit));
        (sql, p)
    } else {
        let sql = format!(
            "SELECT {COLS} FROM messages m WHERE 1=1{filter_sql} ORDER BY m.date DESC LIMIT ?"
        );
        let mut p = params;
        p.push(SqlValue::Integer(limit));
        (sql, p)
    };

    let rows: Vec<RowData> = deps
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(ordered.iter()), |r| {
                    Ok(RowData {
                        id: r.get(0)?,
                        thread_id: r.get(1)?,
                        folder_id: r.get(2)?,
                        subject: r.get(3)?,
                        from_name: r.get(4)?,
                        from_addr: r.get(5)?,
                        date: r.get(6)?,
                        snippet: r.get(7)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await?;

    let count = rows.len() as u32;
    if rows.is_empty() {
        return Ok(("No emails matched.".to_string(), 0));
    }

    let mut lines = Vec::with_capacity(rows.len());
    for row in &rows {
        let from = display_from(&row.from_name, &row.from_addr);
        let subject = row.subject.clone();
        let idx = reg.assign(row.id, |index| Citation {
            index,
            message_id: row.id,
            thread_id: row.thread_id,
            folder_id: row.folder_id,
            subject: subject.clone(),
            from: from.clone(),
        });
        let snippet = prompts::truncate(&row.snippet, SNIPPET_MAX).replace('\n', " ");
        lines.push(format!(
            "[{idx}] {} | {} | {} — {}",
            format_date(row.date),
            from,
            row.subject,
            snippet
        ));
    }
    Ok((lines.join("\n"), count))
}

async fn read_email(deps: &AgentDeps, reg: &Registry, input: &Value) -> Result<String> {
    let Some(r) = input.get("ref").and_then(Value::as_i64) else {
        return Ok("Provide a numeric `ref` from a search result.".to_string());
    };
    let Some(message_id) = reg.citation(r as usize).map(|c| c.message_id) else {
        return Ok(format!(
            "No search result with ref [{r}]. Run search_emails first."
        ));
    };
    let block = email_block_owned(&deps.db, &deps.engines, message_id).await?;
    Ok(format!(
        "[{r}] From: {} | Date: {} | Subject: {}\n\n{}",
        block.from,
        block.date,
        block.subject,
        prompts::truncate(&block.body, READ_EMAIL_BUDGET)
    ))
}

// ---- shared helpers -------------------------------------------------------

/// Ensure a message's body is cached (best effort over IMAP), then return its
/// prompt block. Free-function twin of `commands::ai::email_block` that takes
/// owned handles so it can run inside the agent's spawned task.
pub async fn email_block_owned(
    db: &Db,
    engines: &HashMap<String, SyncHandle>,
    message_id: i64,
) -> Result<prompts::EmailBlock> {
    let needs_fetch = db
        .call(move |conn| bodies::body_state(conn, message_id))
        .await?
        == Some(0);
    if needs_fetch {
        let account_id: String = db
            .call(move |conn| {
                conn.query_row(
                    "SELECT account_id FROM messages WHERE id = ?1",
                    rusqlite::params![message_id],
                    |r| r.get(0),
                )
            })
            .await?;
        if let Some(handle) = engines.get(&account_id).cloned() {
            let _ = handle.fetch_body(message_id).await;
        }
    }

    db.call(move |conn| {
        let (subject, from_name, from_addr, date): (
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        ) = conn.query_row(
            "SELECT subject, from_name, from_addr, date FROM messages WHERE id = ?1",
            rusqlite::params![message_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
        let body = bodies::get_body(conn, message_id)?
            .and_then(|(_, text)| text)
            .unwrap_or_default();
        Ok(prompts::EmailBlock {
            from: display_from(&from_name, &from_addr),
            date: format_date(date),
            subject: subject.unwrap_or_default(),
            body,
            attachments: String::new(),
        })
    })
    .await
}

fn display_from(name: &Option<String>, addr: &Option<String>) -> String {
    match (name, addr) {
        (Some(n), Some(a)) if !n.is_empty() => format!("{n} <{a}>"),
        (_, Some(a)) => a.clone(),
        _ => "unknown".into(),
    }
}

fn str_opt(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Local-midnight unix seconds for the start of the given day.
fn parse_day_start(s: &str) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    local_unix(d.and_hms_opt(0, 0, 0)?)
}

/// Local-midnight unix seconds for the start of the day *after* the given day,
/// so a `before` filter includes the whole named day.
fn parse_day_end(s: &str) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    local_unix(d.succ_opt()?.and_hms_opt(0, 0, 0)?)
}

fn local_unix(naive: chrono::NaiveDateTime) -> Option<i64> {
    Some(
        chrono::Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp()),
    )
}

/// Distinct `[N]` markers the model emitted, so we surface only cited emails.
fn cited_indices(text: &str) -> HashSet<usize> {
    let bytes = text.as_bytes();
    let mut out = HashSet::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b']' {
                if let Ok(n) = text[i + 1..j].parse::<usize>() {
                    out.insert(n);
                }
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_distinct_citations() {
        let c = cited_indices("Paid via [2] and [5], also [2] again. Not [x] or [].");
        assert!(c.contains(&2));
        assert!(c.contains(&5));
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn day_bounds_are_ordered() {
        let start = parse_day_start("2026-07-14").unwrap();
        let end = parse_day_end("2026-07-14").unwrap();
        assert_eq!(end - start, 86_400);
    }
}
