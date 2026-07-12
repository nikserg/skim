//! Minimal streaming client for the Anthropic Messages API. The user's own
//! key is used; requests go directly from this machine to api.anthropic.com.

use crate::error::{Result, SkimError};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;

const API_BASE: &str = "https://api.anthropic.com/v1";
const API_VERSION: &str = "2023-06-01";

pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: &'static str, // "user" | "assistant"
    pub content: String,
}

pub struct Request {
    pub model: String,
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
}

/// Validate an API key with a free models-list call.
pub async fn validate_key(key: &str) -> Result<()> {
    let resp = reqwest::Client::new()
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
        "messages": request.messages,
        "stream": true,
    });

    let resp = reqwest::Client::new()
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
