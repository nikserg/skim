use crate::ai::retrieval::Citation;
use crate::ai::{anthropic, openrouter, prompts, retrieval, ChatMessage};
use crate::db::{bodies, queries, Db};
use crate::error::{Result, SkimError};
use crate::secrets;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AiEvent {
    Delta { text: String },
    Progress { current: usize, total: usize },
    Done { citations: Vec<Citation> },
    Error { code: String, message: String },
}

// ---- providers -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Anthropic,
    OpenRouter,
}

impl Provider {
    fn parse(s: &str) -> Self {
        if s == "openrouter" {
            Provider::OpenRouter
        } else {
            Provider::Anthropic
        }
    }

    fn secret(self) -> &'static str {
        match self {
            Provider::Anthropic => secrets::ANTHROPIC_KEY,
            Provider::OpenRouter => secrets::OPENROUTER_KEY,
        }
    }
}

// ---- key management ------------------------------------------------------

#[tauri::command]
pub async fn ai_set_key(state: State<'_, AppState>, provider: String, key: String) -> Result<()> {
    let key = key.trim().to_string();
    let provider = Provider::parse(&provider);
    match provider {
        Provider::Anthropic => anthropic::validate_key(&key).await?,
        Provider::OpenRouter => openrouter::validate_key(&key).await?,
    }
    secrets::set(provider.secret(), &key)?;
    // Configuring a provider's key makes it the active one.
    let name = match provider {
        Provider::Anthropic => "anthropic",
        Provider::OpenRouter => "openrouter",
    };
    state
        .db
        .call(move |conn| queries::set_setting(conn, "ai_provider", name))
        .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub provider: String,
    pub anthropic: bool,
    pub openrouter: bool,
}

#[tauri::command]
pub async fn ai_key_status(state: State<'_, AppState>) -> Result<KeyStatus> {
    let provider = state
        .db
        .call(|conn| queries::get_setting(conn, "ai_provider"))
        .await?
        .unwrap_or_else(|| "anthropic".into());
    Ok(KeyStatus {
        provider,
        anthropic: secrets::get(secrets::ANTHROPIC_KEY)?.is_some(),
        openrouter: secrets::get(secrets::OPENROUTER_KEY)?.is_some(),
    })
}

#[tauri::command]
pub fn ai_clear_key(provider: String) -> Result<()> {
    secrets::delete(Provider::parse(&provider).secret())
}

// ---- shared plumbing -----------------------------------------------------

struct AiContext {
    provider: Provider,
    key: String,
    model: String,
    locale: String,
    /// e.g. "Monday, 2026-07-13 14:32 (UTC+02:00)"
    now: String,
}

fn now_line() -> String {
    let now = chrono::Local::now();
    format!(
        "{} (UTC{})",
        now.format("%A, %Y-%m-%d %H:%M"),
        now.format("%:z")
    )
}

async fn ai_context(db: &Db) -> Result<AiContext> {
    let (provider, anthropic_model, openrouter_model, locale) = db
        .call(|conn| {
            Ok((
                queries::get_setting(conn, "ai_provider")?,
                queries::get_setting(conn, "ai_model")?,
                queries::get_setting(conn, "openrouter_model")?,
                queries::get_setting(conn, "locale")?,
            ))
        })
        .await?;
    let provider = Provider::parse(provider.as_deref().unwrap_or("anthropic"));
    let key = secrets::get(provider.secret())?
        .ok_or_else(|| SkimError::other("ai_key", "no AI API key configured"))?;
    let model = match provider {
        Provider::Anthropic => {
            anthropic_model.unwrap_or_else(|| anthropic::DEFAULT_MODEL.to_string())
        }
        Provider::OpenRouter => {
            openrouter_model.unwrap_or_else(|| openrouter::DEFAULT_MODEL.to_string())
        }
    };
    Ok(AiContext {
        provider,
        key,
        model,
        locale: locale.unwrap_or_else(|| "en".into()),
        now: now_line(),
    })
}

/// Spawn the streaming task and register it for cancellation.
#[allow(clippy::too_many_arguments)] // flat request parameters, one call path
fn spawn_stream(
    state: &AppState,
    request_id: String,
    ctx: AiContext,
    system: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    citations: Vec<Citation>,
    channel: Channel<AiEvent>,
) {
    let task = tokio::spawn(async move {
        let mut on_delta = |delta: &str| {
            let _ = channel.send(AiEvent::Delta {
                text: delta.to_string(),
            });
        };
        let result = match ctx.provider {
            Provider::Anthropic => {
                let request = anthropic::Request {
                    model: ctx.model,
                    system,
                    messages,
                    max_tokens,
                };
                anthropic::stream(&ctx.key, &request, &mut on_delta).await
            }
            Provider::OpenRouter => {
                let request = openrouter::Request {
                    model: ctx.model,
                    system,
                    messages,
                    max_tokens,
                };
                openrouter::stream(&ctx.key, &request, &mut on_delta).await
            }
        };
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

/// A single user turn — the shape every one-shot feature sends.
fn user_turn(content: String) -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: "user",
        content,
    }]
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
    let (system, user) = prompts::summarize(&emails, &ctx.now, &ctx.locale);
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user_turn(user),
        1024,
        Vec::new(),
        channel,
    );
    Ok(())
}

/// The writer profile from Settings, with the account name as fallback.
async fn writer_profile(state: &State<'_, AppState>) -> Result<prompts::WriterProfile> {
    state
        .db
        .call(|conn| {
            use rusqlite::OptionalExtension;
            let custom_name =
                queries::get_setting(conn, "ai_user_name")?.filter(|s| !s.trim().is_empty());
            let name = match custom_name {
                Some(name) => name,
                None => conn
                    .query_row(
                        "SELECT COALESCE(NULLIF(display_name, ''), email) FROM accounts LIMIT 1",
                        [],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?
                    .unwrap_or_else(|| "the user".into()),
            };
            Ok(prompts::WriterProfile {
                name,
                style: queries::get_setting(conn, "ai_style")?
                    .filter(|s| !s.is_empty() && s != "auto"),
                instructions: queries::get_setting(conn, "ai_instructions")?,
                style_profile: queries::get_setting(conn, "ai_style_profile")?,
            })
        })
        .await
}

/// The reply-to message plus up to `limit - 1` earlier messages of its
/// thread, in chronological order (the replied-to message is last).
async fn reply_chain(
    state: &State<'_, AppState>,
    message_id: i64,
    limit: usize,
) -> Result<Vec<prompts::EmailBlock>> {
    let ids: Vec<i64> = state
        .db
        .call(move |conn| {
            let (thread_id, date): (Option<i64>, i64) = conn.query_row(
                "SELECT thread_id, date FROM messages WHERE id = ?1",
                rusqlite::params![message_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let Some(thread_id) = thread_id else {
                return Ok(vec![message_id]);
            };
            // Earlier part of the thread, ending at the replied-to message.
            let mut stmt = conn.prepare_cached(
                "SELECT id FROM messages
                 WHERE thread_id = ?1 AND (date < ?2 OR id = ?3)
                 ORDER BY date DESC LIMIT ?4",
            )?;
            let mut ids = stmt
                .query_map(
                    rusqlite::params![thread_id, date, message_id, limit as i64],
                    |r| r.get::<_, i64>(0),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ids.reverse(); // chronological
            if ids.last() != Some(&message_id) {
                ids.retain(|id| *id != message_id);
                ids.push(message_id);
            }
            Ok(ids)
        })
        .await?;
    let mut chain = Vec::with_capacity(ids.len());
    for id in ids {
        chain.push(email_block(state, id).await?);
    }
    Ok(chain)
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
    // A reply sees the whole conversation, not just the last message.
    let chain = match reply_to_message_id {
        Some(id) => reply_chain(&state, id, 8).await?,
        None => Vec::new(),
    };
    let profile = writer_profile(&state).await?;
    let (system, user) = prompts::draft(
        &instruction,
        &chain,
        tone.as_deref(),
        &profile,
        &ctx.now,
        &ctx.locale,
    );
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user_turn(user),
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
    let profile = writer_profile(&state).await?;
    let (system, user) =
        prompts::adjust(&current_text, &adjustment, &profile, &ctx.now, &ctx.locale);
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user_turn(user),
        2048,
        Vec::new(),
        channel,
    );
    Ok(())
}

/// One turn of the composer's drafting conversation, as sent by the frontend.
/// `role` is "user" (an instruction) or "assistant" (a draft the AI produced).
#[derive(Debug, Deserialize)]
pub struct ComposeTurn {
    pub role: String,
    pub content: String,
}

/// Interactive email drafting. Unlike `ai_draft`/`ai_adjust_draft` (one-shot),
/// this carries the whole conversation so the user can refine the draft turn by
/// turn against a single shared context. The assistant's reply IS the current
/// email body; the newest turn must be a user instruction.
#[tauri::command]
pub async fn ai_compose(
    state: State<'_, AppState>,
    request_id: String,
    turns: Vec<ComposeTurn>,
    reply_to_message_id: Option<i64>,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    // A reply sees the whole conversation, not just the last message.
    let chain = match reply_to_message_id {
        Some(id) => reply_chain(&state, id, 8).await?,
        None => Vec::new(),
    };
    let profile = writer_profile(&state).await?;
    let (system, preamble) = prompts::compose_session(&chain, &profile, &ctx.now, &ctx.locale);

    // The reply/quote context is folded into the first user turn so the whole
    // session shares it without re-sending it every round.
    let mut messages: Vec<ChatMessage> = Vec::with_capacity(turns.len());
    let mut injected = false;
    for turn in &turns {
        let role: &'static str = if turn.role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        let content = if !injected && role == "user" && !preamble.is_empty() {
            injected = true;
            format!("{preamble}\n\n{}", turn.content)
        } else {
            turn.content.clone()
        };
        messages.push(ChatMessage { role, content });
    }
    if messages.is_empty() {
        return Err(SkimError::other("ai", "no instruction provided"));
    }
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        messages,
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
    let (system, user) = prompts::ask(&email, &question, &ctx.now, &ctx.locale);
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user_turn(user),
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
    let (system, user) = prompts::chat(&question, &context, &ctx.now, &ctx.locale);
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user_turn(user),
        4096,
        citations,
        channel,
    );
    Ok(())
}

/// AI catch-up over the folder's unread mail. Streams the digest and returns
/// the covered messages as citations — the frontend marks those read.
#[tauri::command]
pub async fn ai_recap(
    state: State<'_, AppState>,
    request_id: String,
    folder_id: i64,
    channel: Channel<AiEvent>,
) -> Result<()> {
    const RECAP_LIMIT: usize = 20;
    /// (message id, thread id, subject, from)
    type RecapRow = (i64, Option<i64>, String, String);

    let ctx = ai_context(&state.db).await?;
    let (rows, unread_total): (Vec<RecapRow>, usize) = state
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT id, thread_id, COALESCE(subject, ''),
                        COALESCE(NULLIF(from_name, ''), COALESCE(from_addr, ''))
                 FROM messages
                 WHERE folder_id = ?1 AND is_read = 0
                 ORDER BY date DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![folder_id, RECAP_LIMIT as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let total: i64 = conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND is_read = 0",
                rusqlite::params![folder_id],
                |r| r.get(0),
            )?;
            Ok((rows, total as usize))
        })
        .await?;
    if rows.is_empty() {
        return Err(SkimError::other("mail", "no unread messages"));
    }

    let total = rows.len();
    let mut context: Vec<(usize, prompts::EmailBlock)> = Vec::with_capacity(total);
    let mut citations: Vec<Citation> = Vec::with_capacity(total);
    for (i, (id, thread_id, subject, from)) in rows.into_iter().enumerate() {
        let _ = channel.send(AiEvent::Progress {
            current: i + 1,
            total,
        });
        let Ok(block) = email_block(&state, id).await else {
            continue;
        };
        let index = citations.len() + 1;
        citations.push(Citation {
            index,
            message_id: id,
            thread_id,
            folder_id,
            subject,
            from,
        });
        context.push((index, block));
    }
    if context.is_empty() {
        return Err(SkimError::other("mail", "no unread messages"));
    }

    let more = unread_total.saturating_sub(context.len());
    let (system, user) = prompts::recap(&context, more, &ctx.now, &ctx.locale);
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user_turn(user),
        2048,
        citations,
        channel,
    );
    Ok(())
}

// ---- personal style analysis ----------------------------------------------

/// The user's own words: quoted tails, quote lines, and the signature
/// delimiter are stripped.
fn strip_quoted(body: &str) -> String {
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        // Attribution line that introduces a quoted reply.
        let attribution = (trimmed.starts_with("On ") && trimmed.ends_with("wrote:"))
            || trimmed.ends_with("пишет:")
            || trimmed.ends_with("schrieb:")
            || trimmed.ends_with("a écrit :");
        if attribution || trimmed.starts_with("-----Original Message-----") || trimmed == "-- " {
            break;
        }
        if trimmed.starts_with('>') {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

/// Scan the user's sent mail and distill a personal writing-style profile.
/// Progress events cover the scan; the profile itself streams as deltas and
/// is persisted (`ai_style_profile`) when generation completes.
#[tauri::command]
pub async fn ai_analyze_style(
    state: State<'_, AppState>,
    request_id: String,
    channel: Channel<AiEvent>,
) -> Result<()> {
    const SCAN_LIMIT: usize = 100;
    const SAMPLE_TARGET: usize = 40;

    let ctx = ai_context(&state.db).await?;
    let ids: Vec<i64> = state
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT m.id FROM messages m
                 JOIN folders f ON m.folder_id = f.id
                 WHERE f.role = 'sent'
                 ORDER BY m.date DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![SCAN_LIMIT as i64], |r| r.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    if ids.is_empty() {
        return Err(SkimError::other(
            "ai_no_sent",
            "no sent messages to analyze",
        ));
    }

    let total = ids.len();
    let mut samples: Vec<String> = Vec::new();
    for (i, id) in ids.into_iter().enumerate() {
        let _ = channel.send(AiEvent::Progress {
            current: i + 1,
            total,
        });
        if samples.len() >= SAMPLE_TARGET {
            break;
        }
        let Ok(block) = email_block(&state, id).await else {
            continue;
        };
        let own_words = strip_quoted(&block.body);
        // Too short to carry style signal (acks, "thanks!", …) still counts
        // a little — keep a lower bar but skip empties.
        if own_words.chars().count() >= 25 {
            samples.push(prompts::truncate(&own_words, 1_200));
        }
    }
    if samples.is_empty() {
        return Err(SkimError::other(
            "ai_no_sent",
            "no sent messages with text to analyze",
        ));
    }

    let (system, user) = prompts::style_analysis(&samples, &ctx.locale);
    let db = state.db.clone();
    let task = tokio::spawn(async move {
        let request = anthropic::Request {
            model: ctx.model.clone(),
            system: system.clone(),
            messages: vec![ChatMessage {
                role: "user",
                content: user.clone(),
            }],
            max_tokens: 1024,
        };
        let mut profile_text = String::new();
        let mut on_delta = |delta: &str| {
            profile_text.push_str(delta);
            let _ = channel.send(AiEvent::Delta {
                text: delta.to_string(),
            });
        };
        let result = match ctx.provider {
            Provider::Anthropic => anthropic::stream(&ctx.key, &request, &mut on_delta).await,
            Provider::OpenRouter => {
                let request = openrouter::Request {
                    model: ctx.model,
                    system,
                    messages: user_turn(user),
                    max_tokens: 1024,
                };
                openrouter::stream(&ctx.key, &request, &mut on_delta).await
            }
        };
        match result {
            Ok(_) => {
                let text = profile_text.trim().to_string();
                let _ = db
                    .call(move |conn| {
                        queries::set_setting(conn, "ai_style_profile", &text)?;
                        queries::set_setting(conn, "ai_style", "mine")
                    })
                    .await;
                let _ = channel.send(AiEvent::Done {
                    citations: Vec::new(),
                });
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
    Ok(())
}
