//! Minimal streaming client for the OpenRouter chat-completions API
//! (OpenAI-compatible). The user's own key is used; requests go directly
//! from this machine to openrouter.ai.

use crate::error::{Result, SkimError};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;

const API_BASE: &str = "https://openrouter.ai/api/v1";

pub const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-5";

pub struct Request {
    pub model: String,
    pub system: String,
    pub user: String,
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
    let body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": [
            { "role": "system", "content": request.system },
            { "role": "user", "content": request.user },
        ],
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
