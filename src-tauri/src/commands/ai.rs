use crate::ai::retrieval::Citation;
use crate::ai::{agent, anthropic, attachments, openrouter, prompts, ChatMessage, MediaBlock};
use crate::db::{queries, Db};
use crate::error::{Result, SkimError};
use crate::secrets;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AiEvent {
    Delta {
        text: String,
    },
    Progress {
        current: usize,
        total: usize,
    },
    /// The agent invoked a tool. `kind` is "search" or "read"; `arg` is a short
    /// human summary for the reasoning trace.
    ToolCall {
        id: String,
        kind: String,
        arg: String,
    },
    /// A tool finished. `count` is the number of emails a search returned.
    ToolDone {
        id: String,
        count: Option<u32>,
    },
    Done {
        citations: Vec<Citation>,
    },
    Error {
        code: String,
        message: String,
    },
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
    media: Vec<MediaBlock>,
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
                    media,
                    max_tokens,
                };
                anthropic::stream(&ctx.key, &request, &mut on_delta).await
            }
            Provider::OpenRouter => {
                // OpenRouter has no native attachment path; content was folded
                // into the prompt text as extracted text, so `media` is unused.
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
/// prompt block. Snapshots the engine map and delegates to the owned-handle
/// twin shared with the agent loop.
async fn email_block(state: &State<'_, AppState>, message_id: i64) -> Result<prompts::EmailBlock> {
    let engines = state.engines.lock().await.clone();
    agent::email_block_owned(&state.db, &engines, message_id).await
}

/// Enrich `blocks` (aligned 1:1 with `ids`, chronological) with attachment
/// context and gather native media blocks for the request. Processes the chain
/// anchor-first (the last id — the open message) so it wins the shared budget.
/// Bodies must already be built (that triggers the fetch that caches the files).
async fn collect_attachments(
    state: &State<'_, AppState>,
    ctx: &AiContext,
    ids: &[i64],
    blocks: &mut [prompts::EmailBlock],
) -> Vec<MediaBlock> {
    let native = ctx.provider == Provider::Anthropic;
    let mut budget = attachments::Budget::default();
    let mut media: Vec<MediaBlock> = Vec::new();
    for i in (0..ids.len()).rev() {
        let collected =
            attachments::collect_for_message(&state.db, ids[i], native, &mut budget).await;
        if !collected.notes.is_empty() {
            blocks[i].attachments = collected.notes;
        }
        media.extend(collected.media);
    }
    media
}

// ---- features ------------------------------------------------------------

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

/// The anchor message plus up to `limit - 1` earlier messages of its
/// thread, in chronological order (the anchor message is last).
async fn reply_chain(
    state: &State<'_, AppState>,
    message_id: i64,
    limit: usize,
    attach: Option<&AiContext>,
) -> Result<(Vec<prompts::EmailBlock>, Vec<MediaBlock>)> {
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
            // Earlier part of the thread, ending at the anchor message.
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
    for id in &ids {
        chain.push(email_block(state, *id).await?);
    }
    let media = match attach {
        Some(ctx) => collect_attachments(state, ctx, &ids, &mut chain).await,
        None => Vec::new(),
    };
    Ok((chain, media))
}

/// One turn of an AI conversation (composer drafting or ask sessions), as sent
/// by the frontend. `role` is "user" or "assistant".
#[derive(Debug, Deserialize)]
pub struct AiTurn {
    pub role: String,
    pub content: String,
}

/// Turns as sent by the frontend → provider messages, with `preamble` folded
/// into the first user turn so the whole session shares the context without
/// re-sending it every round.
fn session_messages(turns: &[AiTurn], preamble: &str) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = Vec::with_capacity(turns.len());
    let mut injected = false;
    for turn in turns {
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
    messages
}

/// Interactive email drafting. Carries the whole conversation so the user can
/// refine the draft turn by turn against a single shared context. The
/// assistant's reply IS the current email body; the newest turn must be a user
/// instruction.
#[tauri::command]
pub async fn ai_compose(
    state: State<'_, AppState>,
    request_id: String,
    turns: Vec<AiTurn>,
    reply_to_message_id: Option<i64>,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    // A reply sees the whole conversation, not just the last message.
    let (chain, media) = match reply_to_message_id {
        Some(id) => reply_chain(&state, id, 8, Some(&ctx)).await?,
        None => (Vec::new(), Vec::new()),
    };
    let profile = writer_profile(&state).await?;
    let (system, preamble) = prompts::compose_session(&chain, &profile, &ctx.now, &ctx.locale);

    let messages = session_messages(&turns, &preamble);
    if messages.is_empty() {
        return Err(SkimError::other("ai", "no instruction provided"));
    }
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        messages,
        media,
        2048,
        Vec::new(),
        channel,
    );
    Ok(())
}

/// Q&A about the open message's conversation. Carries the whole dialog so the
/// user can ask follow-ups against a single shared context; `message_id` is
/// the message open in the reading pane, the newest turn is a user question.
#[tauri::command]
pub async fn ai_ask(
    state: State<'_, AppState>,
    request_id: String,
    message_id: i64,
    turns: Vec<AiTurn>,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    let (chain, media) = reply_chain(&state, message_id, 25, Some(&ctx)).await?;
    let (system, preamble) = prompts::ask_session(&chain, &ctx.now, &ctx.locale);
    let messages = session_messages(&turns, &preamble);
    if messages.is_empty() {
        return Err(SkimError::other("ai", "no question provided"));
    }
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        messages,
        media,
        2048,
        Vec::new(),
        channel,
    );
    Ok(())
}

/// Mailbox-wide assistant. The model drives retrieval through the
/// `search_emails` / `read_email` tools (see [`crate::ai::agent`]); we stream
/// its reasoning trace and answer, then return the cited emails. Carries the
/// whole conversation so the user can ask follow-ups against a shared context;
/// the newest turn is the current user question.
#[tauri::command]
pub async fn ai_chat(
    state: State<'_, AppState>,
    request_id: String,
    turns: Vec<AiTurn>,
    prior_citations: Vec<Citation>,
    context_message_id: Option<i64>,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let history: Vec<(String, String)> = turns
        .into_iter()
        .map(|t| (t.role, t.content))
        .filter(|(_, content)| !content.trim().is_empty())
        .collect();
    if history.is_empty() {
        return Err(SkimError::other("ai", "no question provided"));
    }
    let ctx = ai_context(&state.db).await?;
    let provider = match ctx.provider {
        Provider::Anthropic => agent::Provider::Anthropic,
        Provider::OpenRouter => agent::Provider::OpenRouter,
    };
    let system = prompts::chat_agent(&ctx.now, &ctx.locale, context_message_id.is_some());
    let deps = agent::AgentDeps {
        db: state.db.clone(),
        engines: state.engines.lock().await.clone(),
    };

    // The channel is shared by four closures across the spawned task.
    let channel = std::sync::Arc::new(channel);
    let ch_delta = channel.clone();
    let ch_call = channel.clone();
    let ch_done_tool = channel.clone();

    let task = tokio::spawn(async move {
        let mut on_delta = move |d: &str| {
            let _ = ch_delta.send(AiEvent::Delta {
                text: d.to_string(),
            });
        };
        let on_tool_call = move |id: &str, kind: &str, arg: &str| {
            let _ = ch_call.send(AiEvent::ToolCall {
                id: id.to_string(),
                kind: kind.to_string(),
                arg: arg.to_string(),
            });
        };
        let on_tool_done = move |id: &str, count: Option<u32>| {
            let _ = ch_done_tool.send(AiEvent::ToolDone {
                id: id.to_string(),
                count,
            });
        };
        let result = agent::run(
            provider,
            ctx.key,
            ctx.model,
            system,
            history,
            prior_citations,
            context_message_id,
            deps,
            &mut on_delta,
            &on_tool_call,
            &on_tool_done,
        )
        .await;
        match result {
            Ok(citations) => {
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
        Vec::new(),
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
            media: Vec::new(),
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
