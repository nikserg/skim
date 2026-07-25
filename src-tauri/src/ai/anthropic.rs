//! Minimal streaming client for the Anthropic Messages API. The user's own
//! key is used; requests go directly from this machine to api.anthropic.com.

use super::{AssistantTurn, ChatMessage, MediaBlock, MediaKind, ThinkingBlock, ToolCall};
use crate::error::{Result, SkimError};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const API_BASE: &str = "https://api.anthropic.com/v1";
const API_VERSION: &str = "2023-06-01";

pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

pub struct Request {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    /// Native attachments (PDFs/images) appended to the first user turn.
    pub media: Vec<MediaBlock>,
    pub max_tokens: u32,
}

/// One attachment as an Anthropic content block. Shared with the agent loop,
/// which folds media into the first user turn of a tool-calling request.
pub(crate) fn media_block_json(mb: &MediaBlock) -> Value {
    match mb.kind {
        MediaKind::Pdf => json!({
            "type": "document",
            "title": mb.filename,
            "source": {
                "type": "base64",
                "media_type": mb.media_type,
                "data": mb.data_base64,
            },
        }),
        MediaKind::Image => json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": mb.media_type,
                "data": mb.data_base64,
            },
        }),
    }
}

/// Serialize turns to Anthropic's wire format, folding `media` into the first
/// user turn as document/image blocks (each preceded by a label). Messages with
/// no media serialize as `{role, content: "…"}`; the API accepts both shapes.
fn build_messages(messages: &[ChatMessage], media: &[MediaBlock]) -> Vec<Value> {
    let mut media_placed = false;
    messages
        .iter()
        .map(|m| {
            if m.role == "user" && !media_placed && !media.is_empty() {
                media_placed = true;
                let mut content = vec![json!({ "type": "text", "text": m.content })];
                for mb in media {
                    content.push(json!({
                        "type": "text",
                        "text": format!("Attachment \"{}\":", mb.filename),
                    }));
                    content.push(media_block_json(mb));
                }
                json!({ "role": m.role, "content": content })
            } else {
                json!({ "role": m.role, "content": m.content })
            }
        })
        .collect()
}

/// Validate an API key with a free models-list call.
pub async fn validate_key(key: &str) -> Result<()> {
    let resp = super::http_client()
        .get(format!("{API_BASE}/models?limit=1"))
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;
    match resp.status().as_u16() {
        200 => Ok(()),
        401 | 403 => Err(SkimError::other("ai_key", "the API key was rejected")),
        code => Err(SkimError::other(
            "ai",
            format!("unexpected response: {code}"),
        )),
    }
}

/// Stream a completion, invoking `on_delta` for each text fragment.
/// Returns the stop reason. Honors one retry on 429/529.
pub async fn stream(
    key: &str,
    request: &Request,
    mut on_delta: impl FnMut(&str),
) -> Result<Option<String>> {
    let mut attempt = 0;
    loop {
        match stream_once(key, request, &mut on_delta).await {
            Err(e) if e.code() == "ai_overloaded" && attempt == 0 => {
                attempt = 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            other => return other,
        }
    }
}

async fn stream_once(
    key: &str,
    request: &Request,
    on_delta: &mut impl FnMut(&str),
) -> Result<Option<String>> {
    let body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "system": request.system,
        "messages": build_messages(&request.messages, &request.media),
        "stream": true,
    });

    let resp = super::http_client()
        .post(format!("{API_BASE}/messages"))
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;

    let status = resp.status().as_u16();
    if status == 429 || status == 529 {
        return Err(SkimError::other("ai_overloaded", "the API is overloaded"));
    }
    if status == 401 || status == 403 {
        return Err(SkimError::other("ai_key", "the API key was rejected"));
    }
    if status != 200 {
        let text = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or(text);
        return Err(SkimError::other("ai", message));
    }

    let mut stop_reason: Option<String> = None;
    let mut buffer = String::new();
    let mut bytes = resp.bytes_stream();
    while let Some(chunk) = bytes.next().await {
        let chunk = chunk.map_err(|e| SkimError::other("network", e.to_string()))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // SSE frames are separated by blank lines; keep the tail in buffer.
        while let Some(pos) = buffer.find("\n\n") {
            let frame = buffer[..pos].to_string();
            buffer.drain(..pos + 2);
            for line in frame.lines() {
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let Ok(event) = serde_json::from_str::<SseEvent>(data.trim()) else {
                    continue;
                };
                match event {
                    SseEvent::ContentBlockDelta { delta } => {
                        if let Some(text) = delta.text {
                            on_delta(&text);
                        }
                    }
                    SseEvent::MessageDelta { delta } => {
                        if let Some(reason) = delta.stop_reason {
                            stop_reason = Some(reason);
                        }
                    }
                    SseEvent::Error { error } => {
                        return Err(SkimError::other("ai", error.message));
                    }
                    SseEvent::Other => {}
                }
            }
        }
    }
    Ok(stop_reason)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SseEvent {
    ContentBlockDelta {
        delta: Delta,
    },
    MessageDelta {
        delta: MessageDeltaBody,
    },
    Error {
        error: ApiError,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct Delta {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct MessageDeltaBody {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

// ---- tool-calling ---------------------------------------------------------

/// A tool-enabled request. `messages` are raw content-block turns (not
/// `ChatMessage`) so assistant turns can carry `tool_use` blocks and user
/// turns `tool_result` blocks.
pub struct ToolRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub max_tokens: u32,
}

/// Stream one assistant round that may include tool calls. Text is streamed to
/// `on_delta`; `tool_use` blocks are accumulated and returned. One retry on
/// 429/529 (before any bytes stream, so no double-emit).
pub async fn stream_tools(
    key: &str,
    request: &ToolRequest,
    on_delta: &mut impl FnMut(&str),
) -> Result<AssistantTurn> {
    let mut attempt = 0;
    loop {
        match stream_tools_once(key, request, on_delta).await {
            Err(e) if e.code() == "ai_overloaded" && attempt == 0 => {
                attempt = 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            other => return other,
        }
    }
}

/// A content block being accumulated across `content_block_delta` frames.
enum BlockAccum {
    Text(String),
    Tool {
        id: String,
        name: String,
        json: String,
    },
    /// Reasoning. Accumulated but never streamed to the user — it only needs
    /// to survive intact so later rounds can replay it.
    Thinking {
        text: String,
        signature: String,
    },
    Redacted {
        data: String,
    },
}

async fn stream_tools_once(
    key: &str,
    request: &ToolRequest,
    on_delta: &mut impl FnMut(&str),
) -> Result<AssistantTurn> {
    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "system": request.system,
        "messages": request.messages,
        "stream": true,
    });
    // Omit `tools` when empty — the API rejects an empty array, and a
    // tool-less request is just a plain completion (the force-final round).
    if !request.tools.is_empty() {
        body["tools"] = json!(request.tools);
    }

    let resp = super::http_client()
        .post(format!("{API_BASE}/messages"))
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;

    let status = resp.status().as_u16();
    if status == 429 || status == 529 {
        return Err(SkimError::other("ai_overloaded", "the API is overloaded"));
    }
    if status == 401 || status == 403 {
        return Err(SkimError::other("ai_key", "the API key was rejected"));
    }
    if status != 200 {
        let text = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or(text);
        return Err(SkimError::other("ai", message));
    }

    // Blocks are indexed and interleaved: one message can carry a text block
    // plus one or more tool_use blocks. Accumulate each by its index.
    let mut blocks: BTreeMap<usize, BlockAccum> = BTreeMap::new();
    let mut stop_reason: Option<String> = None;
    let mut buffer = String::new();
    let mut bytes = resp.bytes_stream();
    while let Some(chunk) = bytes.next().await {
        let chunk = chunk.map_err(|e| SkimError::other("network", e.to_string()))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find("\n\n") {
            let frame = buffer[..pos].to_string();
            buffer.drain(..pos + 2);
            for line in frame.lines() {
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                apply_sse(data.trim(), &mut blocks, &mut stop_reason, on_delta)?;
            }
        }
    }

    Ok(assemble_turn(blocks, stop_reason))
}

/// Fold one `data:` payload into the accumulating blocks. Unparseable frames
/// are skipped: the stream stays useful across schema additions.
fn apply_sse(
    data: &str,
    blocks: &mut BTreeMap<usize, BlockAccum>,
    stop_reason: &mut Option<String>,
    on_delta: &mut impl FnMut(&str),
) -> Result<()> {
    let Ok(event) = serde_json::from_str::<ToolSse>(data) else {
        return Ok(());
    };
    match event {
        ToolSse::ContentBlockStart {
            index,
            content_block,
        } => match content_block {
            ToolBlockStart::Text { text } => {
                if !text.is_empty() {
                    on_delta(&text);
                }
                blocks.insert(index, BlockAccum::Text(text));
            }
            ToolBlockStart::ToolUse { id, name } => {
                blocks.insert(
                    index,
                    BlockAccum::Tool {
                        id,
                        name,
                        json: String::new(),
                    },
                );
            }
            ToolBlockStart::Thinking {
                thinking,
                signature,
            } => {
                blocks.insert(
                    index,
                    BlockAccum::Thinking {
                        text: thinking,
                        signature,
                    },
                );
            }
            ToolBlockStart::RedactedThinking { data } => {
                blocks.insert(index, BlockAccum::Redacted { data });
            }
            ToolBlockStart::Other => {}
        },
        ToolSse::ContentBlockDelta { index, delta } => match blocks.get_mut(&index) {
            Some(BlockAccum::Text(s)) => {
                if let Some(t) = delta.text {
                    on_delta(&t);
                    s.push_str(&t);
                }
            }
            Some(BlockAccum::Tool { json, .. }) => {
                if let Some(pj) = delta.partial_json {
                    json.push_str(&pj);
                }
            }
            // Reasoning is collected silently: `thinking_delta` must not reach
            // `on_delta` or it would stream into the answer body.
            Some(BlockAccum::Thinking { text, signature }) => {
                if let Some(t) = delta.thinking {
                    text.push_str(&t);
                }
                if let Some(s) = delta.signature {
                    signature.push_str(&s);
                }
            }
            // Redacted reasoning arrives whole in its start frame; there is
            // nothing to accumulate.
            Some(BlockAccum::Redacted { .. }) => {}
            None => {
                // Delta before its start frame — recover text.
                if let Some(t) = delta.text {
                    on_delta(&t);
                    blocks.insert(index, BlockAccum::Text(t));
                }
            }
        },
        ToolSse::MessageDelta { delta } => {
            if let Some(reason) = delta.stop_reason {
                *stop_reason = Some(reason);
            }
        }
        ToolSse::Error { error } => {
            return Err(SkimError::other("ai", error.message));
        }
        ToolSse::Other => {}
    }
    Ok(())
}

fn assemble_turn(
    blocks: BTreeMap<usize, BlockAccum>,
    stop_reason: Option<String>,
) -> AssistantTurn {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    let mut thinking = Vec::new();
    for (_, block) in blocks {
        match block {
            BlockAccum::Text(s) => text.push_str(&s),
            BlockAccum::Tool { id, name, json } => {
                // A no-arg tool emits zero input_json_delta frames → default {}.
                let input = if json.trim().is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&json).unwrap_or_else(|_| serde_json::json!({}))
                };
                tool_calls.push(ToolCall { id, name, input });
            }
            // An unsigned thinking block can't be replayed, so drop it rather
            // than have the API reject the whole next round.
            BlockAccum::Thinking { text, signature } if !signature.is_empty() => {
                thinking.push(ThinkingBlock::Thinking { text, signature });
            }
            BlockAccum::Thinking { .. } => {}
            BlockAccum::Redacted { data } => thinking.push(ThinkingBlock::Redacted { data }),
        }
    }
    AssistantTurn {
        text,
        tool_calls,
        stop_reason,
        thinking,
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToolSse {
    ContentBlockStart {
        index: usize,
        content_block: ToolBlockStart,
    },
    ContentBlockDelta {
        index: usize,
        delta: ToolDelta,
    },
    MessageDelta {
        delta: MessageDeltaBody,
    },
    Error {
        error: ApiError,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ToolBlockStart {
    Text {
        #[serde(default)]
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
        #[serde(default)]
        signature: String,
    },
    RedactedThinking {
        #[serde(default)]
        data: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct ToolDelta {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user",
            content: content.into(),
        }
    }

    #[test]
    fn no_media_keeps_string_content() {
        let msgs = build_messages(&[user("hi")], &[]);
        assert_eq!(msgs, vec![json!({ "role": "user", "content": "hi" })]);
    }

    /// Feed SSE payloads through the parser, returning the assembled turn plus
    /// everything that reached `on_delta`.
    fn run_sse(frames: &[Value]) -> (AssistantTurn, String) {
        let mut blocks = BTreeMap::new();
        let mut stop_reason = None;
        let mut streamed = String::new();
        {
            let mut on_delta = |s: &str| streamed.push_str(s);
            for f in frames {
                apply_sse(&f.to_string(), &mut blocks, &mut stop_reason, &mut on_delta).unwrap();
            }
        }
        (assemble_turn(blocks, stop_reason), streamed)
    }

    #[test]
    fn thinking_is_captured_but_never_streamed() {
        let (turn, streamed) = run_sse(&[
            json!({"type": "content_block_start", "index": 0,
                   "content_block": {"type": "thinking", "thinking": ""}}),
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "thinking_delta", "thinking": "let me check"}}),
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "signature_delta", "signature": "sig123"}}),
            json!({"type": "content_block_start", "index": 1,
                   "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 1,
                   "delta": {"type": "text_delta", "text": "Found it."}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}}),
        ]);
        // The answer is only the text block — reasoning must not leak into it.
        assert_eq!(turn.text, "Found it.");
        assert_eq!(streamed, "Found it.");
        match turn.thinking.as_slice() {
            [ThinkingBlock::Thinking { text, signature }] => {
                assert_eq!(text, "let me check");
                assert_eq!(signature, "sig123");
            }
            other => panic!("expected one signed thinking block, got {other:?}"),
        }
    }

    #[test]
    fn thinking_that_eats_the_budget_yields_no_text() {
        // The bug this guards: reasoning spends the whole max_tokens ceiling,
        // so the round ends with a stop reason and nothing to show.
        let (turn, streamed) = run_sse(&[
            json!({"type": "content_block_start", "index": 0,
                   "content_block": {"type": "thinking", "thinking": ""}}),
            json!({"type": "content_block_delta", "index": 0,
                   "delta": {"type": "thinking_delta", "thinking": "thinking and thinking"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "max_tokens"}}),
        ]);
        assert!(turn.text.is_empty());
        assert!(streamed.is_empty());
        assert_eq!(turn.stop_reason.as_deref(), Some("max_tokens"));
        // Unsigned, so it can't be replayed and is dropped.
        assert!(turn.thinking.is_empty());
    }

    #[test]
    fn redacted_thinking_survives_for_replay() {
        let (turn, _) = run_sse(&[
            json!({"type": "content_block_start", "index": 0,
                   "content_block": {"type": "redacted_thinking", "data": "encrypted"}}),
            json!({"type": "content_block_start", "index": 1,
                   "content_block": {"type": "text", "text": "ok"}}),
        ]);
        assert_eq!(turn.text, "ok");
        match turn.thinking.as_slice() {
            [ThinkingBlock::Redacted { data }] => assert_eq!(data, "encrypted"),
            other => panic!("expected one redacted block, got {other:?}"),
        }
    }

    #[test]
    fn thinking_does_not_disturb_tool_calls() {
        let (turn, _) = run_sse(&[
            json!({"type": "content_block_start", "index": 0,
                   "content_block": {"type": "thinking", "thinking": "x", "signature": "s"}}),
            json!({"type": "content_block_start", "index": 1,
                   "content_block": {"type": "tool_use", "id": "tu_1", "name": "search_emails"}}),
            json!({"type": "content_block_delta", "index": 1,
                   "delta": {"type": "input_json_delta", "partial_json": "{\"query\":\"alta\"}"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}}),
        ]);
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].input["query"], "alta");
        assert_eq!(turn.thinking.len(), 1);
    }

    #[test]
    fn pdf_folds_into_first_user_turn_as_document() {
        let media = vec![MediaBlock {
            kind: MediaKind::Pdf,
            media_type: "application/pdf".into(),
            data_base64: "QkFTRTY0".into(),
            filename: "report.pdf".into(),
        }];
        let msgs = build_messages(&[user("what's in the pdf?")], &media);
        let content = msgs[0]["content"].as_array().unwrap();
        // text prompt, then a label text block, then the document block.
        assert_eq!(
            content[0],
            json!({ "type": "text", "text": "what's in the pdf?" })
        );
        assert_eq!(content[2]["type"], "document");
        assert_eq!(content[2]["source"]["type"], "base64");
        assert_eq!(content[2]["source"]["media_type"], "application/pdf");
        assert_eq!(content[2]["source"]["data"], "QkFTRTY0");
    }

    #[test]
    fn image_uses_image_block() {
        let media = vec![MediaBlock {
            kind: MediaKind::Image,
            media_type: "image/png".into(),
            data_base64: "AAAA".into(),
            filename: "chart.png".into(),
        }];
        let msgs = build_messages(&[user("q")], &media);
        let content = msgs[0]["content"].as_array().unwrap();
        assert_eq!(content[2]["type"], "image");
        assert_eq!(content[2]["source"]["media_type"], "image/png");
    }

    #[test]
    fn media_attaches_only_to_first_user_turn() {
        let media = vec![MediaBlock {
            kind: MediaKind::Pdf,
            media_type: "application/pdf".into(),
            data_base64: "x".into(),
            filename: "a.pdf".into(),
        }];
        let msgs = build_messages(
            &[
                user("first"),
                ChatMessage {
                    role: "assistant",
                    content: "reply".into(),
                },
                user("second"),
            ],
            &media,
        );
        assert!(msgs[0]["content"].is_array());
        assert_eq!(msgs[1], json!({ "role": "assistant", "content": "reply" }));
        assert_eq!(msgs[2], json!({ "role": "user", "content": "second" }));
    }
}
