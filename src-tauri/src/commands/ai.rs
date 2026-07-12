use crate::ai::retrieval::Citation;
use crate::ai::{anthropic, prompts, retrieval};
use crate::db::{bodies, queries, Db};
use crate::error::{Result, SkimError};
use crate::secrets;
use crate::state::AppState;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AiEvent {
    Delta { text: String },
    Done { citations: Vec<Citation> },
    Error { code: String, message: String },
}

// ---- key management ------------------------------------------------------

#[tauri::command]
pub async fn ai_set_key(key: String) -> Result<()> {
    let key = key.trim().to_string();
    anthropic::validate_key(&key).await?;
    secrets::set(secrets::ANTHROPIC_KEY, &key)
}

#[tauri::command]
pub fn ai_key_status() -> Result<bool> {
    Ok(secrets::get(secrets::ANTHROPIC_KEY)?.is_some())
}

#[tauri::command]
pub fn ai_clear_key() -> Result<()> {
    secrets::delete(secrets::ANTHROPIC_KEY)
}

// ---- shared plumbing -----------------------------------------------------

struct AiContext {
    key: String,
    model: String,
    locale: String,
}

async fn ai_context(db: &Db) -> Result<AiContext> {
    let key = secrets::get(secrets::ANTHROPIC_KEY)?
        .ok_or_else(|| SkimError::other("ai_key", "no Anthropic API key configured"))?;
    let (model, locale) = db
        .call(|conn| {
            Ok((
                queries::get_setting(conn, "ai_model")?,
                queries::get_setting(conn, "locale")?,
            ))
        })
        .await?;
    Ok(AiContext {
        key,
        model: model.unwrap_or_else(|| anthropic::DEFAULT_MODEL.to_string()),
        locale: locale.unwrap_or_else(|| "en".into()),
    })
}

/// Spawn the streaming task and register it for cancellation.
#[allow(clippy::too_many_arguments)] // flat request parameters, one call path
fn spawn_stream(
    state: &AppState,
    request_id: String,
    ctx: AiContext,
    system: String,
    user: String,
    max_tokens: u32,
    citations: Vec<Citation>,
    channel: Channel<AiEvent>,
) {
    let task = tokio::spawn(async move {
        let request = anthropic::Request {
            model: ctx.model,
            system,
            messages: vec![anthropic::ChatMessage {
                role: "user",
                content: user,
            }],
            max_tokens,
        };
        let result = anthropic::stream(&ctx.key, &request, |delta| {
            let _ = channel.send(AiEvent::Delta {
                text: delta.to_string(),
            });
        })
        .await;
        match result {
            Ok(_) => {
                let _ = channel.send(AiEvent::Done { citations });
            }
            Err(e) => {
                let _ = channel.send(AiEvent::Error {
                    code: e.code().to_string(),
                    message: e.to_string(),
                });
            }
        }
    });
    if let Ok(mut tasks) = state.ai_tasks.lock() {
        tasks.retain(|_, h| !h.is_finished());
        tasks.insert(request_id, task.abort_handle());
    }
}

#[tauri::command]
pub fn ai_cancel(state: State<'_, AppState>, request_id: String) -> Result<()> {
    if let Ok(mut tasks) = state.ai_tasks.lock() {
        if let Some(handle) = tasks.remove(&request_id) {
            handle.abort();
        }
    }
    Ok(())
}

/// Make sure a message's body is cached (best effort), then return its
/// prompt block.
async fn email_block(state: &State<'_, AppState>, message_id: i64) -> Result<prompts::EmailBlock> {
    let needs_fetch = state
        .db
        .call(move |conn| bodies::body_state(conn, message_id))
        .await?
        == Some(0);
    if needs_fetch {
        let account_id: String = state
            .db
            .call(move |conn| {
                conn.query_row(
                    "SELECT account_id FROM messages WHERE id = ?1",
                    rusqlite::params![message_id],
                    |r| r.get(0),
                )
            })
            .await?;
        let handle = {
            let engines = state.engines.lock().await;
            engines.get(&account_id).cloned()
        };
        if let Some(handle) = handle {
            let _ = handle.fetch_body(message_id).await;
        }
    }

    state
        .db
        .call(move |conn| {
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
            let from = match (&from_name, &from_addr) {
                (Some(n), Some(a)) if !n.is_empty() => format!("{n} <{a}>"),
                (_, Some(a)) => a.clone(),
                _ => "unknown".into(),
            };
            Ok(prompts::EmailBlock {
                from,
                date: retrieval::format_date(date),
                subject: subject.unwrap_or_default(),
                body,
            })
        })
        .await
}

// ---- features ------------------------------------------------------------

#[tauri::command]
pub async fn ai_summarize(
    state: State<'_, AppState>,
    request_id: String,
    thread_id: i64,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    // Latest messages of the thread (bounded), bodies fetched best-effort.
    let ids: Vec<i64> = state
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id FROM messages WHERE thread_id = ?1 ORDER BY date DESC LIMIT 6",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![thread_id], |r| r.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    if ids.is_empty() {
        return Err(SkimError::other("mail", "thread not found"));
    }
    let mut emails = Vec::new();
    for id in ids.into_iter().rev() {
        emails.push(email_block(&state, id).await?);
    }
    let (system, user) = prompts::summarize(&emails, &ctx.locale);
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user,
        1024,
        Vec::new(),
        channel,
    );
    Ok(())
}

#[tauri::command]
pub async fn ai_draft(
    state: State<'_, AppState>,
    request_id: String,
    instruction: String,
    reply_to_message_id: Option<i64>,
    tone: Option<String>,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    let reply_context = match reply_to_message_id {
        Some(id) => Some(email_block(&state, id).await?),
        None => None,
    };
    let user_name: String = state
        .db
        .call(|conn| {
            let row: std::result::Result<(Option<String>, String), _> = conn.query_row(
                "SELECT display_name, email FROM accounts LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            );
            Ok(match row {
                Ok((Some(name), _)) if !name.is_empty() => name,
                Ok((_, email)) => email,
                Err(_) => "the user".into(),
            })
        })
        .await?;
    let (system, user) = prompts::draft(
        &instruction,
        reply_context.as_ref(),
        tone.as_deref(),
        &user_name,
        &ctx.locale,
    );
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user,
        2048,
        Vec::new(),
        channel,
    );
    Ok(())
}

#[tauri::command]
pub async fn ai_adjust_draft(
    state: State<'_, AppState>,
    request_id: String,
    current_text: String,
    adjustment: String,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    let (system, user) = prompts::adjust(&current_text, &adjustment, &ctx.locale);
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user,
        2048,
        Vec::new(),
        channel,
    );
    Ok(())
}

#[tauri::command]
pub async fn ai_ask(
    state: State<'_, AppState>,
    request_id: String,
    message_id: i64,
    question: String,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    let email = email_block(&state, message_id).await?;
    let (system, user) = prompts::ask(&email, &question, &ctx.locale);
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user,
        2048,
        Vec::new(),
        channel,
    );
    Ok(())
}

#[tauri::command]
pub async fn ai_chat(
    state: State<'_, AppState>,
    request_id: String,
    question: String,
    context_message_id: Option<i64>,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    let q = question.clone();
    let retrieved = state
        .db
        .call(move |conn| retrieval::retrieve(conn, &q, context_message_id))
        .await?;

    if retrieved.is_empty() {
        // Nothing matched — don't burn an API call.
        let _ = channel.send(AiEvent::Error {
            code: "ai_no_context".into(),
            message: String::new(),
        });
        return Ok(());
    }

    let citations: Vec<Citation> = retrieved.iter().map(|r| r.citation.clone()).collect();
    let context: Vec<(usize, prompts::EmailBlock)> = retrieved
        .into_iter()
        .map(|r| (r.citation.index, r.block))
        .collect();
    let today = retrieval::format_date(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    );
    let (system, user) = prompts::chat(&question, &context, &today, &ctx.locale);
    spawn_stream(
        &state, request_id, ctx, system, user, 4096, citations, channel,
    );
    Ok(())
}
