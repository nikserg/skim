//! Minimal streaming client for the OpenRouter chat-completions API
//! (OpenAI-compatible). The user's own key is used; requests go directly
//! from this machine to openrouter.ai.

use super::{AssistantTurn, ChatMessage, ToolCall};
use crate::error::{Result, SkimError};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

const API_BASE: &str = "https://openrouter.ai/api/v1";

pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-5";

pub struct Request {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
}

/// Validate an API key with a free key-info call.
pub async fn validate_key(key: &str) -> Result<()> {
    let resp = reqwest::Client::new()
        .get(format!("{API_BASE}/key"))
        .bearer_auth(key)
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

// ---- catalog --------------------------------------------------------------

/// One model the user may pick.
#[derive(Debug, Clone, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
struct CatalogResponse {
    data: Vec<CatalogModel>,
}

#[derive(Deserialize)]
struct CatalogModel {
    id: String,
    name: String,
    #[serde(default)]
    supported_parameters: Vec<String>,
}

/// The live model catalog, narrowed to what Skim can actually drive: the chat
/// agent calls tools, so a model without tool support would silently fail on
/// the mailbox-wide assistant. Needs no API key. Ordered as OpenRouter returns
/// it — newest first.
pub async fn list_models() -> Result<Vec<Model>> {
    let resp = reqwest::Client::new()
        .get(format!("{API_BASE}/models"))
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;
    if resp.status() != 200 {
        return Err(SkimError::other(
            "ai",
            format!("could not load the model list: {}", resp.status()),
        ));
    }
    let catalog: CatalogResponse = resp
        .json()
        .await
        .map_err(|e| SkimError::other("ai", e.to_string()))?;
    Ok(catalog
        .data
        .into_iter()
        .filter(|m| m.supported_parameters.iter().any(|p| p == "tools"))
        .map(|m| Model {
            id: m.id,
            name: m.name,
        })
        .collect())
}

/// Stream a completion, invoking `on_delta` for each text fragment.
/// Returns the finish reason. Honors one retry on rate-limit/upstream errors.
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
    // System prompt first, then the (possibly multi-turn) conversation.
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    for m in &request.messages {
        messages.push(json!({ "role": m.role, "content": m.content }));
    }
    let body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": true,
    });

    let resp = reqwest::Client::new()
        .post(format!("{API_BASE}/chat/completions"))
        .bearer_auth(key)
        // Attribution headers recommended by OpenRouter.
        .header("HTTP-Referer", "https://github.com/nikserg/skim")
        .header("X-Title", "Skim")
        .json(&body)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;

    let status = resp.status().as_u16();
    if status == 429 || status == 502 || status == 503 {
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

    let mut finish_reason: Option<String> = None;
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
                // OpenRouter interleaves ": PROCESSING" comment lines.
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    return Ok(finish_reason);
                }
                let Ok(event) = serde_json::from_str::<Chunk>(data) else {
                    continue;
                };
                if let Some(error) = event.error {
                    return Err(SkimError::other("ai", error.message));
                }
                for choice in event.choices {
                    if let Some(text) = choice.delta.content {
                        if !text.is_empty() {
                            on_delta(&text);
                        }
                    }
                    if let Some(reason) = choice.finish_reason {
                        finish_reason = Some(reason);
                    }
                }
            }
        }
    }
    Ok(finish_reason)
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

// ---- tool-calling ---------------------------------------------------------

/// A tool-enabled request. `messages` are raw OpenAI-shaped turns (assistant
/// turns may carry `tool_calls`, results are `role:"tool"` messages); the
/// system prompt is prepended here.
pub struct ToolRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub max_tokens: u32,
}

/// Stream one assistant round that may include tool calls. Text streams to
/// `on_delta`; `tool_calls` are accumulated and returned. One retry on
/// rate-limit/upstream errors (before any bytes stream).
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

struct TcAccum {
    id: String,
    name: String,
    args: String,
}

fn assemble_turn(
    text: String,
    tcs: BTreeMap<usize, TcAccum>,
    finish_reason: Option<String>,
) -> AssistantTurn {
    let tool_calls: Vec<ToolCall> = tcs
        .into_values()
        .map(|t| {
            let input = if t.args.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&t.args).unwrap_or_else(|_| json!({}))
            };
            ToolCall {
                id: t.id,
                name: t.name,
                input,
            }
        })
        .collect();
    // Normalize to the Anthropic-style marker the agent loop checks for.
    let stop_reason = if tool_calls.is_empty() {
        finish_reason
    } else {
        Some("tool_use".to_string())
    };
    AssistantTurn {
        text,
        tool_calls,
        stop_reason,
    }
}

async fn stream_tools_once(
    key: &str,
    request: &ToolRequest,
    on_delta: &mut impl FnMut(&str),
) -> Result<AssistantTurn> {
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    messages.extend(request.messages.iter().cloned());
    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": true,
    });
    // Omit `tools`/`tool_choice` when empty — strict endpoints reject an empty
    // tools array or a tool_choice with no tools (the force-final round).
    if !request.tools.is_empty() {
        body["tools"] = json!(request.tools);
        body["tool_choice"] = json!("auto");
    }

    let resp = reqwest::Client::new()
        .post(format!("{API_BASE}/chat/completions"))
        .bearer_auth(key)
        .header("HTTP-Referer", "https://github.com/nikserg/skim")
        .header("X-Title", "Skim")
        .json(&body)
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;

    let status = resp.status().as_u16();
    if status == 429 || status == 502 || status == 503 {
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

    let mut text = String::new();
    let mut tcs: BTreeMap<usize, TcAccum> = BTreeMap::new();
    let mut finish_reason: Option<String> = None;
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
                let data = data.trim();
                if data == "[DONE]" {
                    return Ok(assemble_turn(text, tcs, finish_reason));
                }
                let Ok(event) = serde_json::from_str::<ToolChunk>(data) else {
                    continue;
                };
                if let Some(error) = event.error {
                    return Err(SkimError::other("ai", error.message));
                }
                for choice in event.choices {
                    if let Some(t) = choice.delta.content {
                        if !t.is_empty() {
                            on_delta(&t);
                            text.push_str(&t);
                        }
                    }
                    if let Some(calls) = choice.delta.tool_calls {
                        for call in calls {
                            let e = tcs.entry(call.index).or_insert_with(|| TcAccum {
                                id: String::new(),
                                name: String::new(),
                                args: String::new(),
                            });
                            if let Some(id) = call.id {
                                if !id.is_empty() {
                                    e.id = id;
                                }
                            }
                            if let Some(f) = call.function {
                                if let Some(n) = f.name {
                                    if !n.is_empty() {
                                        e.name = n;
                                    }
                                }
                                if let Some(a) = f.arguments {
                                    e.args.push_str(&a);
                                }
                            }
                        }
                    }
                    if let Some(reason) = choice.finish_reason {
                        finish_reason = Some(reason);
                    }
                }
            }
        }
    }
    Ok(assemble_turn(text, tcs, finish_reason))
}

#[derive(Deserialize)]
struct ToolChunk {
    #[serde(default)]
    choices: Vec<ToolChoice>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ToolChoice {
    #[serde(default)]
    delta: ToolDeltaBody,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct ToolDeltaBody {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Deserialize)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}
