use crate::ai::retrieval::Citation;
use crate::ai::{
    agent, anthropic, attachments, ollama, openai_compat, openrouter, prompts, ChatMessage,
    MediaBlock,
};
use crate::db::{bodies, queries, Db};
use crate::error::{Result, SkimError};
use crate::mail::translate;
use crate::secrets;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AiEvent {
    Delta {
        text: String,
    },
    /// The model is reasoning before it answers. Nothing to render: it only
    /// says the model is alive, so the UI can stop guessing that a silent
    /// round means one still loading. Sent once per request, whatever the
    /// provider reported (see [`reasoning_once`]).
    Reasoning,
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

/// Output ceiling for the one-shot AI features. Like the agent's own budget,
/// this is shared with whatever reasoning the model does before it answers —
/// too tight and a thinking model spends it all and returns nothing.
const ONE_SHOT_MAX_TOKENS: u32 = 8192;
/// Style analysis reads a handful of sent messages and writes a short profile,
/// so it needs less room than a user-facing answer — but still more than the
/// reasoning alone will use.
const STYLE_MAX_TOKENS: u32 = 4096;

// ---- providers -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Anthropic,
    OpenRouter,
    /// A user-supplied OpenAI-compatible endpoint ("custom" in settings).
    Custom,
}

impl Provider {
    fn parse(s: &str) -> Self {
        match s {
            "openrouter" => Provider::OpenRouter,
            "custom" => Provider::Custom,
            _ => Provider::Anthropic,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenRouter => "openrouter",
            Provider::Custom => "custom",
        }
    }

    fn secret(self) -> &'static str {
        match self {
            Provider::Anthropic => secrets::ANTHROPIC_KEY,
            Provider::OpenRouter => secrets::OPENROUTER_KEY,
            Provider::Custom => secrets::CUSTOM_KEY,
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
        // The custom endpoint is configured via `ai_set_custom` — there is no
        // universal validation endpoint to call anyway.
        Provider::Custom => {}
    }
    secrets::set(provider.secret(), &key)?;
    // Configuring a provider's key makes it the active one.
    let name = provider.id();
    state
        .db
        .call(move |conn| queries::set_setting(conn, "ai_provider", name))
        .await
}

/// Configure the user-supplied OpenAI-compatible endpoint and make it the
/// active provider. The key is optional (local servers need none) — an empty
/// key clears any stored one. No connectivity check: there is no universal
/// probe endpoint, so errors surface on the first real request instead.
#[tauri::command]
pub async fn ai_set_custom(
    state: State<'_, AppState>,
    base_url: String,
    key: String,
    model: String,
) -> Result<()> {
    let base_url = openai_compat::normalize_base_url(&base_url)
        .ok_or_else(|| SkimError::other("ai", "enter a valid http(s) URL"))?;
    let model = model.trim().to_string();
    if model.is_empty() {
        return Err(SkimError::other("ai", "enter a model id"));
    }
    let key = key.trim();
    if key.is_empty() {
        secrets::delete(secrets::CUSTOM_KEY)?;
    } else {
        secrets::set(secrets::CUSTOM_KEY, key)?;
    }
    state
        .db
        .call(move |conn| {
            queries::set_setting(conn, "custom_base_url", &base_url)?;
            queries::set_setting(conn, "custom_model", &model)?;
            queries::set_setting(conn, "ai_provider", "custom")
        })
        .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub provider: String,
    pub anthropic: bool,
    pub openrouter: bool,
    /// The custom endpoint counts as configured once a base URL is set — its
    /// key is optional.
    pub custom: bool,
}

#[tauri::command]
pub async fn ai_key_status(state: State<'_, AppState>) -> Result<KeyStatus> {
    let (provider, custom_base_url) = state
        .db
        .call(|conn| {
            Ok((
                queries::get_setting(conn, "ai_provider")?,
                queries::get_setting(conn, "custom_base_url")?,
            ))
        })
        .await?;
    Ok(KeyStatus {
        provider: provider.unwrap_or_else(|| "anthropic".into()),
        anthropic: secrets::get(secrets::ANTHROPIC_KEY)?.is_some(),
        openrouter: secrets::get(secrets::OPENROUTER_KEY)?.is_some(),
        custom: custom_base_url.is_some_and(|u| !u.trim().is_empty()),
    })
}

#[tauri::command]
pub async fn ai_clear_key(state: State<'_, AppState>, provider: String) -> Result<()> {
    let provider = Provider::parse(&provider);
    secrets::delete(provider.secret())?;
    // "Configured" for the custom endpoint means "base URL set" — clear it so
    // the provider reads as removed, not just keyless. The model goes with it:
    // a removed endpoint should leave nothing behind to resurface later.
    if provider == Provider::Custom {
        state
            .db
            .call(|conn| {
                queries::set_setting(conn, "custom_base_url", "")?;
                queries::set_setting(conn, "custom_model", "")
            })
            .await?;
    }
    Ok(())
}

/// OpenRouter's live catalog, so the model field can only ever be set to a
/// model that exists. Fetched on demand — Settings asks once per session.
#[tauri::command]
pub async fn openrouter_models() -> Result<Vec<openrouter::Model>> {
    openrouter::list_models().await
}

/// The models installed on an Ollama server reachable at the custom
/// endpoint's base URL, narrowed to tool-capable ones. Any error means
/// "not an Ollama server" to the caller.
#[tauri::command]
pub async fn ollama_models(url: String) -> Result<Vec<ollama::Model>> {
    ollama::list_models(&url).await
}

// ---- shared plumbing -----------------------------------------------------

struct AiContext {
    provider: Provider,
    /// Where OpenAI-compatible traffic goes; `None` for Anthropic.
    endpoint: Option<openai_compat::Endpoint>,
    key: String,
    model: String,
    locale: String,
    /// e.g. "Monday, 2026-07-13 14:32 (UTC+02:00)"
    now: String,
}

impl AiContext {
    fn agent_provider(&self) -> agent::Provider {
        match &self.endpoint {
            None => agent::Provider::Anthropic,
            Some(ep) => agent::Provider::OpenAiCompat(ep.clone()),
        }
    }
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
    let (provider, anthropic_model, openrouter_model, custom_base_url, custom_model, locale) = db
        .call(|conn| {
            Ok((
                queries::get_setting(conn, "ai_provider")?,
                queries::get_setting(conn, "ai_model")?,
                queries::get_setting(conn, "openrouter_model")?,
                queries::get_setting(conn, "custom_base_url")?,
                queries::get_setting(conn, "custom_model")?,
                queries::get_setting(conn, "locale")?,
            ))
        })
        .await?;
    let provider = Provider::parse(provider.as_deref().unwrap_or("anthropic"));
    // The custom endpoint may legitimately have no key (local servers).
    let key = match provider {
        Provider::Custom => secrets::get(secrets::CUSTOM_KEY)?.unwrap_or_default(),
        _ => secrets::get(provider.secret())?
            .ok_or_else(|| SkimError::other("ai_key", "no AI API key configured"))?,
    };
    let (model, endpoint) = match provider {
        Provider::Anthropic => (
            anthropic_model.unwrap_or_else(|| anthropic::DEFAULT_MODEL.to_string()),
            None,
        ),
        Provider::OpenRouter => (
            openrouter_model.unwrap_or_else(|| openrouter::DEFAULT_MODEL.to_string()),
            Some(openrouter::endpoint()),
        ),
        Provider::Custom => {
            let base_url = custom_base_url
                .as_deref()
                .and_then(openai_compat::normalize_base_url)
                .ok_or_else(|| SkimError::other("ai_key", "no AI endpoint configured"))?;
            let model = custom_model
                .filter(|m| !m.trim().is_empty())
                .ok_or_else(|| SkimError::other("ai", "no model configured"))?;
            (
                model,
                Some(openai_compat::Endpoint {
                    base_url,
                    attribution: false,
                }),
            )
        }
    };
    Ok(AiContext {
        provider,
        endpoint,
        key,
        model,
        locale: locale.unwrap_or_else(|| "en".into()),
        now: now_line(),
    })
}

/// The oldest date every still-filling inbox is known to cover, so the agent
/// can tell "not in your mail" apart from "not downloaded yet". `None` once the
/// backfill has walked each inbox to its first message — then the cache is the
/// whole mailbox and there is nothing to qualify.
///
/// Takes the newest of the per-inbox oldest dates: that is the boundary above
/// which *every* inbox has coverage, so the claim holds for all of them.
async fn sync_horizon(db: &Db) -> Option<String> {
    let oldest: i64 = db
        .call(|conn| {
            conn.query_row(
                "SELECT MAX(oldest) FROM (
                   SELECT MIN(m.date) AS oldest
                     FROM messages m JOIN folders f ON f.id = m.folder_id
                    WHERE f.role = 'inbox' AND f.backfill_done = 0
                    GROUP BY m.folder_id
                 )",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
        })
        .await
        .ok()
        .flatten()?;
    Some(crate::ai::retrieval::format_date(oldest))
}

/// What becomes of a one-shot stream's text once it finishes.
enum Persist {
    /// Nothing: the UI already received it delta by delta.
    Nothing,
    /// Splice the numbered segment translations back into the message body and
    /// cache the result. Boxed because it carries the whole source body.
    Translation(Box<TranslationJob>),
}

/// Everything needed to turn a numbered reply back into a translated body.
struct TranslationJob {
    db: Db,
    message_id: i64,
    /// Target language: the key the cached row is stored under.
    lang: String,
    /// The original body — whichever of the two forms the message had.
    html: Option<String>,
    text: Option<String>,
    extracted: translate::Extracted,
    truncated: bool,
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
    persist: Persist,
    channel: Channel<AiEvent>,
) {
    let task = tokio::spawn(async move {
        // A translation's raw text is not for reading — the pane redraws from the
        // cached body once it lands — so it accumulates here and the segments
        // that have come back are reported as progress instead.
        let total = match &persist {
            Persist::Translation(job) => job.extracted.len(),
            Persist::Nothing => 0,
        };
        let mut answer = String::new();
        let mut reported = 0;
        let mut on_delta = |delta: &str| {
            if total == 0 {
                let _ = channel.send(AiEvent::Delta {
                    text: delta.to_string(),
                });
                return;
            }
            answer.push_str(delta);
            let done = answer.matches("[[").count().min(total);
            if done != reported {
                reported = done;
                let _ = channel.send(AiEvent::Progress {
                    current: done,
                    total,
                });
            }
        };
        let mut on_reasoning = reasoning_once(&channel);
        let result = match &ctx.endpoint {
            None => {
                let request = anthropic::Request {
                    model: ctx.model,
                    system,
                    messages,
                    media,
                    max_tokens,
                };
                anthropic::stream(&ctx.key, &request, &mut on_delta, &mut on_reasoning).await
            }
            Some(ep) => {
                // OpenAI-compatible endpoints have no native attachment path;
                // content was folded into the prompt text as extracted text,
                // so `media` is unused.
                let request = openai_compat::Request {
                    model: ctx.model,
                    system,
                    messages,
                    max_tokens,
                };
                openai_compat::stream(ep, &ctx.key, &request, &mut on_delta, &mut on_reasoning)
                    .await
            }
        };
        match result {
            Ok(_) => {
                if let Persist::Translation(job) = persist {
                    if let Err(e) = store_translation(*job, &answer).await {
                        let _ = channel.send(AiEvent::Error {
                            code: e.code().to_string(),
                            message: e.to_string(),
                        });
                        return;
                    }
                }
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

/// Rebuild the body with the translated segments in place and cache it.
async fn store_translation(job: TranslationJob, answer: &str) -> Result<()> {
    // A marker with nothing after it is not an answer: it is what a stream cut
    // right after `[[n]]` leaves behind. Dropped here rather than downstream,
    // because kept it would count as a translated segment and `apply` would
    // splice the empty string over the block, taking the paragraph out of the
    // mail for good.
    let mut segments = translate::parse_reply(answer);
    segments.retain(|_, text| !text.trim().is_empty());
    if segments.is_empty() {
        // Nothing usable came back. Caching the original as its own translation
        // would leave the pane claiming a translated message it never got.
        return Err(SkimError::other(
            "ai",
            "no translated segments in the reply",
        ));
    }
    let TranslationJob {
        db,
        message_id,
        lang,
        html,
        text,
        extracted,
        truncated,
    } = job;
    // A reply can also come back short of the list it was given: the output
    // ceiling is shared with whatever reasoning came first, and a model can drop
    // a segment on its own. Whatever it never answered keeps its original
    // language, which is the very thing `truncated` reports, so an input that
    // outran the budget and an answer that did land on the same note instead of
    // the second passing for a whole translation.
    //
    // Segment numbers are the index plus one, with no gaps, so asking for each
    // one by name is exact where comparing counts is not: a reply that skips a
    // number while inventing one out of range keeps the total intact.
    let answered_all = (1..=extracted.len() as u32).all(|id| segments.contains_key(&id));
    let truncated = truncated || !answered_all;
    let (html, text) = match (&html, &text) {
        (Some(html), _) => (Some(translate::apply(html, &extracted, &segments)), None),
        (None, Some(text)) => (
            None,
            Some(translate::apply_text(text, &extracted, &segments)),
        ),
        (None, None) => (None, None),
    };
    // The subject's segment is never spliced into the body — it is stored on its
    // own, for the pane's heading.
    let subject = extracted
        .subject_id()
        .and_then(|id| segments.get(&id))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let translation = bodies::Translation {
        html,
        text,
        subject,
        truncated,
    };
    db.call(move |conn| bodies::set_translation(conn, message_id, &lang, &translation))
        .await
}

/// One `Reasoning` event per request: the latch is built once per command, so
/// it spans every round of an agent loop too. The streaming clients call
/// `on_reasoning` per frame, because "is this the first one" is not their
/// business: a round can open several reasoning blocks, and the frames carry no
/// text to count anyway. Collapsing that to a single event belongs here, once,
/// for every provider and every command.
fn reasoning_once(channel: &Channel<AiEvent>) -> impl FnMut() + '_ {
    let mut reported = false;
    move || {
        if !reported {
            reported = true;
            let _ = channel.send(AiEvent::Reasoning);
        }
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

// ---- pop-out chat windows -------------------------------------------------

/// How far each further pop-out is nudged off the centred one, in logical px,
/// so a second window doesn't land exactly on top of the first.
const WINDOW_CASCADE: f64 = 28.0;

/// Give a chat its own window. The session stays where it is — the main window
/// owns the conversation and the request, and this window is a view onto it,
/// addressed by `session_id`. Closing it is how the chat comes back inline, so
/// `lib.rs` reports the window's destruction to the frontend.
///
/// Async on purpose: building a webview from a synchronous command runs it on
/// the main thread, where it waits for an event loop that is waiting for the
/// command — the window comes up stuck on about:blank. `open_compose_window`
/// has the same shape for the same reason.
#[tauri::command]
pub async fn open_ai_window(app: AppHandle, session_id: u64, title: String) -> Result<()> {
    let label = format!("ai-chat-{session_id}");
    // Asking again for a chat that already has a window means "show me that
    // window", not "open a second one onto the same conversation".
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }
    let title = match window_title(title.trim()) {
        t if t.is_empty() => "Skim AI".to_string(),
        t => t,
    };
    let open = app
        .webview_windows()
        .keys()
        .filter(|label| label.starts_with("ai-chat-"))
        .count();
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::App(format!("index.html#/chat/{session_id}").into()),
    )
    .title(title)
    .inner_size(640.0, 760.0)
    .min_inner_size(420.0, 420.0)
    .decorations(false)
    .center()
    .build()
    .map_err(|e| SkimError::other("window", e.to_string()))?;

    if open > 0 {
        // Cascade off the centre; wrap after a few so windows stay on screen.
        let scale = window.scale_factor().unwrap_or(1.0);
        let step = (WINDOW_CASCADE * scale) as i32 * (open % 6) as i32;
        if step > 0 {
            if let Ok(pos) = window.outer_position() {
                let _ =
                    window.set_position(tauri::PhysicalPosition::new(pos.x + step, pos.y + step));
            }
        }
    }
    Ok(())
}

/// Trim a subject/question down to something a taskbar button can show.
fn window_title(raw: &str) -> String {
    let single_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= 60 {
        return single_line;
    }
    let head: String = single_line.chars().take(59).collect();
    format!("{}…", head.trim_end())
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
        ONE_SHOT_MAX_TOKENS,
        Vec::new(),
        Persist::Nothing,
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
    let history: Vec<(String, String)> = turns
        .into_iter()
        .map(|t| (t.role, t.content))
        .filter(|(_, content)| !content.trim().is_empty())
        .collect();
    if history.is_empty() {
        return Err(SkimError::other("ai", "no question provided"));
    }
    let ctx = ai_context(&state.db).await?;
    // The whole thread (plus native attachments) is folded into the first turn;
    // the model can't search the mailbox — it only answers about this
    // conversation, optionally opening links the thread contains via `fetch_url`.
    let (chain, media) = reply_chain(&state, message_id, 25, Some(&ctx)).await?;
    // Local phishing heuristics over the open message (None when clean), so a
    // "is this legit?" question — or the phishing quick chip — gets grounded
    // signals instead of the model guessing from body text alone.
    let security = state
        .db
        .call(move |conn| {
            let html = match crate::db::bodies::get_body(conn, message_id)? {
                Some((Some(html), _)) => {
                    crate::mail::sanitize::sanitize_email_html(&html, message_id, true).html
                }
                Some((None, Some(text))) => crate::mail::sanitize::text_to_html(&text),
                _ => String::new(),
            };
            crate::mail::suspicion::prompt_summary(conn, message_id, &html)
        })
        .await?;
    let (system, preamble) =
        prompts::ask_session(&chain, security.as_deref(), &ctx.now, &ctx.locale);
    let provider = ctx.agent_provider();
    let deps = agent::AgentDeps {
        db: state.db.clone(),
        engines: state.engines.lock().await.clone(),
    };

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
        let mut on_reasoning = reasoning_once(&channel);
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
            Vec::new(),
            agent::Context::Thread { preamble, media },
            agent::ToolSet::FETCH_ONLY,
            deps,
            &mut on_delta,
            &mut on_reasoning,
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
    let provider = ctx.agent_provider();
    let horizon = sync_horizon(&state.db).await;
    let system = prompts::chat_agent(
        &ctx.now,
        &ctx.locale,
        context_message_id.is_some(),
        horizon.as_deref(),
    );
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
        let mut on_reasoning = reasoning_once(&channel);
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
            agent::Context::OpenMessage(context_message_id),
            agent::ToolSet::MAILBOX,
            deps,
            &mut on_delta,
            &mut on_reasoning,
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
    /// (message id, thread id, real folder id, subject, from)
    type RecapRow = (i64, Option<i64>, i64, String, String);

    let ctx = ai_context(&state.db).await?;
    // A negative folder id is the virtual "All inboxes" folder — recap every
    // account's inbox. Citations always carry the message's real folder id so
    // clicking one can navigate in either scope.
    const RECAP_FILTER: &str = "((?1 >= 0 AND folder_id = ?1)
          OR (?1 < 0 AND folder_id IN (SELECT id FROM folders WHERE role = 'inbox')))";
    let (rows, unread_total): (Vec<RecapRow>, usize) = state
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT id, thread_id, folder_id, COALESCE(subject, ''),
                        COALESCE(NULLIF(from_name, ''), COALESCE(from_addr, ''))
                 FROM messages
                 WHERE {RECAP_FILTER} AND is_read = 0
                 ORDER BY date DESC LIMIT ?2"
            ))?;
            let rows = stmt
                .query_map(rusqlite::params![folder_id, RECAP_LIMIT as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let total: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM messages WHERE {RECAP_FILTER} AND is_read = 0"),
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
    for (i, (id, thread_id, real_folder_id, subject, from)) in rows.into_iter().enumerate() {
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
            folder_id: real_folder_id,
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
        ONE_SHOT_MAX_TOKENS,
        citations,
        Persist::Nothing,
        channel,
    );
    Ok(())
}

// ---- inline translation ----------------------------------------------------

/// Source characters one translation request may carry, counted over the
/// deduplicated segment texts.
///
/// Derived from what the answer actually costs rather than guessed: a Cyrillic
/// target runs ~2.2 characters per token, the `[[n]]` markers add ~7 characters
/// per segment, and [`ONE_SHOT_MAX_TOKENS`] is shared with whatever reasoning
/// the model does first. 10k source characters come back as roughly 5–6k tokens,
/// which fits with margin — and covers all but the longest few percent of real
/// mail. Anything past this keeps its original language rather than being
/// chunked into a second request, which would double the latency and the price.
const TRANSLATE_MAX_CHARS: usize = 10_000;
/// And a cap on the count, so a pathological newsletter can't become a thousand
/// numbered lines.
const TRANSLATE_MAX_SEGMENTS: usize = 250;

/// Translate one message's body into the user's language, in place.
///
/// The reply is not streamed to the UI: it is a numbered segment list, useful
/// only once spliced back into the body. What the pane gets is progress, and
/// then a cached translation to re-render from.
#[tauri::command]
pub async fn ai_translate(
    state: State<'_, AppState>,
    request_id: String,
    message_id: i64,
    channel: Channel<AiEvent>,
) -> Result<()> {
    let ctx = ai_context(&state.db).await?;
    // Also caches the body over IMAP if this message has never been opened.
    let block = email_block(&state, message_id).await?;
    let (html, text) = state
        .db
        .call(move |conn| bodies::get_body(conn, message_id))
        .await?
        .unwrap_or((None, None));

    let mut extracted = match (&html, &text) {
        (Some(html), _) => translate::extract(html),
        (None, Some(text)) => translate::extract_text(text),
        (None, None) => translate::Extracted::default(),
    };
    if extracted.is_empty() {
        return Err(SkimError::other("ai", "nothing to translate"));
    }
    let truncated = extracted.truncate_to(TRANSLATE_MAX_CHARS, TRANSLATE_MAX_SEGMENTS);
    // After the budget: the subject is one short line the pane shows above the
    // body, and it must never be what gets dropped.
    extracted.add_subject(&block.subject);

    let (system, user) = prompts::translate(&block, &extracted.prompt_list(), &ctx.locale);
    let job = TranslationJob {
        db: state.db.clone(),
        message_id,
        lang: ctx.locale.clone(),
        html,
        text,
        extracted,
        truncated,
    };
    spawn_stream(
        &state,
        request_id,
        ctx,
        system,
        user_turn(user),
        Vec::new(),
        ONE_SHOT_MAX_TOKENS,
        Vec::new(),
        Persist::Translation(Box::new(job)),
        channel,
    );
    Ok(())
}

// ---- personal style analysis ----------------------------------------------

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
        let own_words = crate::mail::parse::strip_quoted(&block.body);
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
            max_tokens: STYLE_MAX_TOKENS,
        };
        let mut profile_text = String::new();
        let mut on_delta = |delta: &str| {
            profile_text.push_str(delta);
            let _ = channel.send(AiEvent::Delta {
                text: delta.to_string(),
            });
        };
        let mut on_reasoning = reasoning_once(&channel);
        let result = match &ctx.endpoint {
            None => anthropic::stream(&ctx.key, &request, &mut on_delta, &mut on_reasoning).await,
            Some(ep) => {
                let request = openai_compat::Request {
                    model: ctx.model,
                    system,
                    messages: user_turn(user),
                    max_tokens: STYLE_MAX_TOKENS,
                };
                openai_compat::stream(ep, &ctx.key, &request, &mut on_delta, &mut on_reasoning)
                    .await
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

#[cfg(test)]
mod tests {
    use super::{bodies, store_translation, translate, window_title, Provider, TranslationJob};
    use crate::db::Db;

    /// A mailbox with one message whose body is already cached.
    async fn seed(db: &Db) -> i64 {
        db.call(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, provider, imap_host, smtp_host, created_at)
                 VALUES ('a1', 'me@example.com', 'custom', 'imap.example.com',
                         'smtp.example.com', 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO folders (account_id, imap_name, role, display_name, unread_count,
                                      sort_order)
                 VALUES ('a1', 'INBOX', 'inbox', 'Inbox', 0, 0)",
                [],
            )?;
            let folder = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO messages (account_id, folder_id, uid, date, is_read, is_starred,
                                       has_attachments, body_state)
                 VALUES ('a1', ?1, 1, 0, 0, 0, 0, 1)",
                rusqlite::params![folder],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .unwrap()
    }

    /// The whole backend half of a translation, minus the network: a numbered
    /// reply goes in, a translated body comes out of the cache with its markup
    /// untouched.
    #[tokio::test]
    async fn a_numbered_reply_becomes_a_cached_translated_body() {
        let db = Db::open_in_memory().unwrap();
        let id = seed(&db).await;
        let html = "<p>Hello <a href=\"https://x.test\">friend</a></p><p>Bye for now</p>";
        let job = TranslationJob {
            db: db.clone(),
            message_id: id,
            lang: "ru".into(),
            html: Some(html.to_string()),
            text: None,
            extracted: translate::extract(html),
            truncated: false,
        };
        store_translation(job, "[[1]] Привет, <1>друг</1>\n[[2]] Пока\n")
            .await
            .unwrap();

        let got = db
            .call(move |conn| bodies::get_translation(conn, id, "ru"))
            .await
            .unwrap()
            .expect("cached");
        assert_eq!(
            got.html.unwrap(),
            "<p>Привет, <a href=\"https://x.test\">друг</a></p><p>Пока</p>"
        );
        assert!(!got.truncated);
    }

    /// A model that stops mid-list leaves every segment after it in the source
    /// language, the same half-translated message a too-long mail produces, so
    /// it carries the same note. Caching it as complete would be the worst of
    /// both: a body still in its original language, and a cache row that stops
    /// a second press from ever trying again.
    #[tokio::test]
    async fn a_reply_that_stops_mid_list_is_cached_as_truncated() {
        let db = Db::open_in_memory().unwrap();
        let id = seed(&db).await;
        let html = "<p>Hello there</p><p>Bye for now</p>";
        let job = TranslationJob {
            db: db.clone(),
            message_id: id,
            lang: "ru".into(),
            html: Some(html.to_string()),
            text: None,
            extracted: translate::extract(html),
            truncated: false,
        };
        store_translation(job, "[[1]] Привет\n").await.unwrap();

        let got = db
            .call(move |conn| bodies::get_translation(conn, id, "ru"))
            .await
            .unwrap()
            .expect("cached");
        assert_eq!(got.html.unwrap(), "<p>Привет</p><p>Bye for now</p>");
        assert!(got.truncated, "a partial reply must be flagged");
    }

    /// The same, for a reply that keeps the total intact by inventing a number:
    /// two segments asked for, two answered, but one of them is an id nobody
    /// requested. Counting would call that whole; asking for each number by name
    /// catches the one that never came back.
    #[tokio::test]
    async fn a_reply_that_answers_an_unasked_number_is_cached_as_truncated() {
        let db = Db::open_in_memory().unwrap();
        let id = seed(&db).await;
        let html = "<p>Hello there</p><p>Bye for now</p>";
        let job = TranslationJob {
            db: db.clone(),
            message_id: id,
            lang: "ru".into(),
            html: Some(html.to_string()),
            text: None,
            extracted: translate::extract(html),
            truncated: false,
        };
        store_translation(job, "[[1]] Привет\n[[3]] Лишнее\n")
            .await
            .unwrap();

        let got = db
            .call(move |conn| bodies::get_translation(conn, id, "ru"))
            .await
            .unwrap()
            .expect("cached");
        assert_eq!(got.html.unwrap(), "<p>Привет</p><p>Bye for now</p>");
        assert!(got.truncated, "a missing requested id must be flagged");
    }

    /// A marker with nothing after it is what a stream cut mid-list leaves
    /// behind. Counted as an answer, it would splice an empty string over the
    /// block and take the paragraph out of the mail: worse than not translating
    /// it, and permanent once cached.
    #[tokio::test]
    async fn a_marker_with_no_text_after_it_never_erases_its_block() {
        let db = Db::open_in_memory().unwrap();
        let id = seed(&db).await;
        let html = "<p>Hello there</p><p>Bye for now</p>";
        let job = TranslationJob {
            db: db.clone(),
            message_id: id,
            lang: "ru".into(),
            html: Some(html.to_string()),
            text: None,
            extracted: translate::extract(html),
            truncated: false,
        };
        store_translation(job, "[[1]] Привет\n[[2]]").await.unwrap();

        let got = db
            .call(move |conn| bodies::get_translation(conn, id, "ru"))
            .await
            .unwrap()
            .expect("cached");
        assert_eq!(got.html.unwrap(), "<p>Привет</p><p>Bye for now</p>");
        assert!(got.truncated, "an empty answer is not an answer");
    }

    /// A reply with nothing usable in it must not be cached: the pane would then
    /// show the original while claiming it was translated.
    #[tokio::test]
    async fn an_unusable_reply_is_not_cached() {
        let db = Db::open_in_memory().unwrap();
        let id = seed(&db).await;
        let html = "<p>Hello there</p>";
        let job = TranslationJob {
            db: db.clone(),
            message_id: id,
            lang: "ru".into(),
            html: Some(html.to_string()),
            text: None,
            extracted: translate::extract(html),
            truncated: false,
        };
        assert!(store_translation(job, "I'm sorry, I can't help with that.")
            .await
            .is_err());
        assert!(db
            .call(move |conn| bodies::get_translation(conn, id, "ru"))
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn window_title_collapses_whitespace_and_truncates() {
        assert_eq!(
            window_title("Re:  the  budget\nthread"),
            "Re: the budget thread"
        );
        let long = "word ".repeat(40);
        let title = window_title(&long);
        assert_eq!(title.chars().count(), 60);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn provider_parse_round_trips_and_defaults_to_anthropic() {
        for p in [Provider::Anthropic, Provider::OpenRouter, Provider::Custom] {
            assert_eq!(Provider::parse(p.id()), p);
        }
        assert_eq!(Provider::parse("unknown"), Provider::Anthropic);
        assert_eq!(Provider::parse(""), Provider::Anthropic);
    }
}
