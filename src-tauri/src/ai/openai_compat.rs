//! Minimal streaming client for OpenAI-compatible chat-completions APIs.
//! Serves both the OpenRouter provider and any user-supplied endpoint
//! (Ollama, LM Studio, vLLM, a gateway, OpenAI itself). The user's own key
//! is used; requests go directly from this machine to the endpoint.

use super::{AssistantTurn, ChatMessage, ToolCall};
use crate::error::{Result, SkimError};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

/// Where an OpenAI-compatible request goes.
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// Normalized base, e.g. "https://openrouter.ai/api/v1" — no trailing
    /// slash; `/chat/completions` is appended here.
    pub base_url: String,
    /// Send OpenRouter's attribution headers (HTTP-Referer / X-Title).
    pub attribution: bool,
}

/// Clean up a user-entered base URL. Trims whitespace and trailing slashes,
/// strips an accidentally pasted `/chat/completions`, and requires a parseable
/// http(s) URL with a host. Deliberately does NOT append `/v1` — endpoints
/// disagree on whether they use it, so the user's input is taken literally.
pub fn normalize_base_url(raw: &str) -> Option<String> {
    let mut url = raw.trim().trim_end_matches('/');
    if let Some(stripped) = url.strip_suffix("/chat/completions") {
        url = stripped.trim_end_matches('/');
    }
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(url.to_string())
}

pub struct Request {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
}

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

fn chat_body(request: &Request) -> serde_json::Value {
    // System prompt first, then the (possibly multi-turn) conversation.
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    for m in &request.messages {
        messages.push(json!({ "role": m.role, "content": m.content }));
    }
    json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": true,
    })
}

fn tool_body(request: &ToolRequest) -> serde_json::Value {
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
    body
}

/// POST to the endpoint's `/chat/completions`. No Authorization header at all
/// when `key` is empty — local servers need none, and some reject an empty
/// bearer.
fn post_chat(ep: &Endpoint, key: &str, body: &serde_json::Value) -> reqwest::RequestBuilder {
    let mut req = reqwest::Client::new().post(format!("{}/chat/completions", ep.base_url));
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    if ep.attribution {
        // Attribution headers recommended by OpenRouter.
        req = req
            .header("HTTP-Referer", "https://github.com/nikserg/skim")
            .header("X-Title", "Skim");
    }
    req.json(body)
}

/// Map a non-200 response to the shared error codes; `Ok` for 200.
async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
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
    Ok(resp)
}

/// Pop the `data:` payloads of every complete (`\n\n`-terminated) SSE frame
/// off the front of `buffer`. Comment lines (`: PROCESSING`) and other
/// non-data lines are skipped. With `flush`, the remaining tail is treated as
/// one final frame — some servers end the stream without a terminator or a
/// `[DONE]` marker.
fn drain_data(buffer: &mut String, flush: bool) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        let frame = match buffer.find("\n\n") {
            Some(pos) => {
                let frame = buffer[..pos].to_string();
                buffer.drain(..pos + 2);
                frame
            }
            None if flush && !buffer.is_empty() => std::mem::take(buffer),
            None => break,
        };
        for line in frame.lines() {
            if let Some(data) = line.strip_prefix("data:") {
                out.push(data.trim().to_string());
            }
        }
    }
    out
}

/// Stream a completion, invoking `on_delta` for each text fragment.
/// Returns the finish reason. Honors one retry on rate-limit/upstream errors.
pub async fn stream(
    ep: &Endpoint,
    key: &str,
    request: &Request,
    mut on_delta: impl FnMut(&str),
) -> Result<Option<String>> {
    let mut attempt = 0;
    loop {
        match stream_once(ep, key, request, &mut on_delta).await {
            Err(e) if e.code() == "ai_overloaded" && attempt == 0 => {
                attempt = 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            other => return other,
        }
    }
}

async fn stream_once(
    ep: &Endpoint,
    key: &str,
    request: &Request,
    on_delta: &mut impl FnMut(&str),
) -> Result<Option<String>> {
    let resp = post_chat(ep, key, &chat_body(request))
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;
    let resp = check_status(resp).await?;

    let mut finish_reason: Option<String> = None;
    let mut buffer = String::new();
    let mut bytes = resp.bytes_stream();
    let mut ended = false;
    while !ended {
        let data = match bytes.next().await {
            Some(chunk) => {
                let chunk = chunk.map_err(|e| SkimError::other("network", e.to_string()))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                drain_data(&mut buffer, false)
            }
            None => {
                ended = true;
                drain_data(&mut buffer, true)
            }
        };
        for data in data {
            if data == "[DONE]" {
                return Ok(finish_reason);
            }
            let Ok(event) = serde_json::from_str::<Chunk>(&data) else {
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

/// Stream one assistant round that may include tool calls. Text streams to
/// `on_delta`; `tool_calls` are accumulated and returned. One retry on
/// rate-limit/upstream errors (before any bytes stream).
pub async fn stream_tools(
    ep: &Endpoint,
    key: &str,
    request: &ToolRequest,
    on_delta: &mut impl FnMut(&str),
) -> Result<AssistantTurn> {
    let mut attempt = 0;
    loop {
        match stream_tools_once(ep, key, request, on_delta).await {
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
    ep: &Endpoint,
    key: &str,
    request: &ToolRequest,
    on_delta: &mut impl FnMut(&str),
) -> Result<AssistantTurn> {
    let resp = post_chat(ep, key, &tool_body(request))
        .send()
        .await
        .map_err(|e| SkimError::other("network", e.to_string()))?;
    let resp = check_status(resp).await?;

    let mut text = String::new();
    let mut tcs: BTreeMap<usize, TcAccum> = BTreeMap::new();
    let mut finish_reason: Option<String> = None;
    let mut buffer = String::new();
    let mut bytes = resp.bytes_stream();
    let mut ended = false;
    while !ended {
        let data = match bytes.next().await {
            Some(chunk) => {
                let chunk = chunk.map_err(|e| SkimError::other("network", e.to_string()))?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                drain_data(&mut buffer, false)
            }
            None => {
                ended = true;
                drain_data(&mut buffer, true)
            }
        };
        for data in data {
            if data == "[DONE]" {
                return Ok(assemble_turn(text, tcs, finish_reason));
            }
            let Ok(event) = serde_json::from_str::<ToolChunk>(&data) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_clean_url_unchanged() {
        assert_eq!(
            normalize_base_url("https://openrouter.ai/api/v1"),
            Some("https://openrouter.ai/api/v1".into())
        );
        assert_eq!(
            normalize_base_url("http://localhost:11434/v1"),
            Some("http://localhost:11434/v1".into())
        );
    }

    #[test]
    fn normalize_trims_whitespace_and_trailing_slashes() {
        assert_eq!(
            normalize_base_url("  http://localhost:11434/v1//  "),
            Some("http://localhost:11434/v1".into())
        );
    }

    #[test]
    fn normalize_strips_pasted_completions_path() {
        assert_eq!(
            normalize_base_url("http://localhost:11434/v1/chat/completions"),
            Some("http://localhost:11434/v1".into())
        );
    }

    #[test]
    fn normalize_rejects_garbage() {
        assert_eq!(normalize_base_url(""), None);
        assert_eq!(normalize_base_url("localhost:11434"), None);
        assert_eq!(normalize_base_url("ftp://host/v1"), None);
        assert_eq!(normalize_base_url("not a url"), None);
    }

    #[test]
    fn chat_body_puts_system_first_and_streams() {
        let body = chat_body(&Request {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![ChatMessage {
                role: "user",
                content: "hi".into(),
            }],
            max_tokens: 42,
        });
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "sys");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 42);
    }

    #[test]
    fn tool_body_omits_tools_when_empty() {
        let base = ToolRequest {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![json!({ "role": "user", "content": "hi" })],
            tools: Vec::new(),
            max_tokens: 42,
        };
        let body = tool_body(&base);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());

        let with_tools = ToolRequest {
            tools: vec![json!({ "type": "function" })],
            ..base
        };
        let body = tool_body(&with_tools);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn drain_data_skips_comments_and_keeps_partial_tail() {
        let mut buffer = ": PROCESSING\n\ndata: {\"a\":1}\n\ndata: {\"partial".to_string();
        assert_eq!(drain_data(&mut buffer, false), vec!["{\"a\":1}"]);
        assert_eq!(buffer, "data: {\"partial");
    }

    #[test]
    fn drain_data_joins_frame_split_across_chunks() {
        let mut buffer = "data: {\"a\"".to_string();
        assert!(drain_data(&mut buffer, false).is_empty());
        buffer.push_str(":1}\n\n");
        assert_eq!(drain_data(&mut buffer, false), vec!["{\"a\":1}"]);
    }

    #[test]
    fn drain_data_flushes_unterminated_final_frame() {
        let mut buffer = "data: {\"a\":1}".to_string();
        assert_eq!(drain_data(&mut buffer, true), vec!["{\"a\":1}"]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn assemble_turn_passes_finish_reason_without_tools() {
        let turn = assemble_turn("hi".into(), BTreeMap::new(), Some("stop".into()));
        assert_eq!(turn.stop_reason.as_deref(), Some("stop"));
        assert!(turn.tool_calls.is_empty());
    }

    #[test]
    fn assemble_turn_normalizes_tool_finish_and_bad_args() {
        let mut tcs = BTreeMap::new();
        tcs.insert(
            0,
            TcAccum {
                id: "c1".into(),
                name: "search".into(),
                args: "{broken".into(),
            },
        );
        let turn = assemble_turn(String::new(), tcs, Some("stop".into()));
        assert_eq!(turn.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(turn.tool_calls[0].input, json!({}));
    }
}
