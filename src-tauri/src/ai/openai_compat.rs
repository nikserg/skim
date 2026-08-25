//! Minimal streaming client for OpenAI-compatible chat-completions APIs.
//! Serves both the OpenRouter provider and any user-supplied endpoint
//! (Ollama, LM Studio, vLLM, a gateway, OpenAI itself). The user's own key
//! is used; requests go directly from this machine to the endpoint.

use super::{AssistantTurn, ChatMessage, ToolCall};
use crate::error::{Result, SkimError};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
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

/// Which JSON key carries the output ceiling. OpenAI deprecated `max_tokens`
/// when o1 shipped, and its reasoning models now refuse it outright — but every
/// other OpenAI-compatible server still speaks it, and several (Ollama,
/// llama-cpp-python) drop `max_completion_tokens` on the floor without
/// complaining, which would leave the answer uncapped rather than merely
/// wrong. So we open with the name everyone understands and switch only for an
/// endpoint that asks us to, by name, in its own error. See
/// [`rejects_max_tokens`].
const MAX_TOKENS: &str = "max_tokens";
/// The name OpenAI's reasoning models want instead. See [`MAX_TOKENS`] for why
/// it is never the one we open with.
const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";

/// Sent only under [`EFFORT_NONE`], and only to an endpoint that named that
/// value in its own refusal. See [`refuses_tools_while_reasoning`].
const REASONING_EFFORT: &str = "reasoning_effort";
/// A downgrade, not a rename: under it the answer comes back from a model that
/// no longer thinks first.
const EFFORT_NONE: &str = "none";

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

fn chat_body(request: &Request, tokens_key: &str) -> serde_json::Value {
    // System prompt first, then the (possibly multi-turn) conversation.
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    for m in &request.messages {
        messages.push(json!({ "role": m.role, "content": m.content }));
    }
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
    });
    body[tokens_key] = json!(request.max_tokens);
    body
}

fn tool_body(request: &ToolRequest, tokens_key: &str, effort: Option<&str>) -> serde_json::Value {
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    messages.extend(request.messages.iter().cloned());
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
    });
    body[tokens_key] = json!(request.max_tokens);
    // Omit `tools`/`tool_choice` when empty — strict endpoints reject an empty
    // tools array or a tool_choice with no tools (the force-final round).
    if !request.tools.is_empty() {
        body["tools"] = json!(request.tools);
        body["tool_choice"] = json!("auto");
        // Inside this branch on purpose: the tools provoke the refusal, so the
        // round carrying none keeps its reasoning, and it writes the answer.
        if let Some(effort) = effort {
            body[REASONING_EFFORT] = json!(effort);
        }
    }
    body
}

/// POST to the endpoint's `/chat/completions`. No Authorization header at all
/// when `key` is empty — local servers need none, and some reject an empty
/// bearer.
fn post_chat(ep: &Endpoint, key: &str, body: &serde_json::Value) -> reqwest::RequestBuilder {
    let mut req = super::http_client().post(format!("{}/chat/completions", ep.base_url));
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

/// Is this error body the endpoint telling us it wants the ceiling under
/// `max_completion_tokens` instead? OpenAI answers 400 with
/// `param: "max_tokens"` and `code: "unsupported_parameter"`, but neither field
/// is required here: a gateway that flattens the envelope keeps only the
/// message, and spells `code` as the HTTP number rather than a string.
///
/// The test is that the endpoint names *both* keys — that is what distinguishes
/// "use the other name" from "that number is too large", which some endpoints
/// also report against `param: "max_tokens"`. Retrying the latter under the new
/// name would not fix it and, on a server that ignores the new name, would
/// quietly remove the ceiling instead.
///
/// The `param` guard covers the mirror case: some Azure deployments reject
/// `max_completion_tokens`, and reading that as this would send back the very
/// key that was just refused.
fn rejects_max_tokens(body: &Value) -> bool {
    if body["error"]["param"] == json!(MAX_COMPLETION_TOKENS) {
        return false;
    }
    let message = body["error"]["message"].as_str().unwrap_or_default();
    message.contains(MAX_TOKENS) && message.contains(MAX_COMPLETION_TOKENS)
}

/// The one sentence every version of this refusal opens with. What follows
/// diverges by model, so the opening is the only stable part to match on.
const TOOLS_REFUSED: &str = "Function tools with reasoning_effort are not supported";

/// Is the endpoint refusing our tools over reasoning it turned on itself? The
/// request never mentioned reasoning: the model defaults to it, then rejects
/// the combination.
///
/// Must open with [`TOOLS_REFUSED`] *and* name `'none'`. The same opening
/// continues "Please use /v1/responses instead" on models offering no remedy we
/// can apply, and `error.param` reads `reasoning_effort` on both, so only the
/// named value separates the refusal we can fix from the one we cannot.
fn refuses_tools_while_reasoning(body: &Value) -> bool {
    let message = body["error"]["message"].as_str().unwrap_or_default();
    message.contains(TOOLS_REFUSED) && message.contains("'none'")
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
        let parsed = serde_json::from_str::<serde_json::Value>(&text).ok();
        let message = parsed
            .as_ref()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or(text);
        // Carry the provider's own message either way: the retry loops consume
        // these codes, so one only reaches the UI if the second attempt failed
        // too.
        if parsed.as_ref().is_some_and(refuses_tools_while_reasoning) {
            return Err(SkimError::other("ai_reasoning_param", message));
        }
        if parsed.as_ref().is_some_and(rejects_max_tokens) {
            return Err(SkimError::other("ai_token_param", message));
        }
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

/// Stream a completion, invoking `on_delta` for each text fragment and
/// `on_reasoning` while the model thinks before it answers.
/// Returns the finish reason. Honors one retry on rate-limit/upstream errors,
/// and one on an endpoint that wants the output ceiling under
/// `max_completion_tokens` (see [`MAX_TOKENS`]). Both happen before any bytes
/// stream, so a retry can never replay text the caller has already seen.
/// A gateway that reports the same refusal inside a 200 stream instead is not
/// covered — by then `on_delta` may have fired.
///
/// A thinking model sends its reasoning first, in frames whose `content` is
/// empty, which from the outside looks exactly like a model still loading; a
/// local one can reason for a minute. The reasoning text itself is dropped, it
/// must never reach the answer body, but its arrival is worth reporting, and
/// that is all `on_reasoning` says. It fires per frame, not once per round;
/// the caller decides what to make of that. [`super::anthropic`] reports
/// thinking through the same pair of callbacks.
pub async fn stream(
    ep: &Endpoint,
    key: &str,
    request: &Request,
    mut on_delta: impl FnMut(&str),
    on_reasoning: &mut impl FnMut(),
) -> Result<Option<String>> {
    let mut overloaded = false;
    let mut switched = false;
    let mut tokens_key = MAX_TOKENS;
    loop {
        match stream_once(ep, key, request, tokens_key, &mut on_delta, on_reasoning).await {
            Err(e) if e.code() == "ai_overloaded" && !overloaded => {
                overloaded = true;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            // No backoff: this is a decision the endpoint just handed us, not
            // backpressure. The latch is separate from the overload one so that
            // a rate limit early on cannot spend the switch, and `tokens_key`
            // outlives the loop body so a later overload retry keeps the new
            // name rather than reverting to the one already refused.
            Err(e) if e.code() == "ai_token_param" && !switched => {
                switched = true;
                tokens_key = MAX_COMPLETION_TOKENS;
            }
            other => return other,
        }
    }
}

async fn stream_once(
    ep: &Endpoint,
    key: &str,
    request: &Request,
    tokens_key: &str,
    on_delta: &mut impl FnMut(&str),
    on_reasoning: &mut impl FnMut(),
) -> Result<Option<String>> {
    let resp = post_chat(ep, key, &chat_body(request, tokens_key))
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
                if choice.delta.thinking() {
                    on_reasoning();
                }
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

/// One `choices[].delta`, shared by both wire shapes: `tool_calls` is simply
/// absent from a plain completion, and serde defaults it to `None`.
#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<Value>,
    #[serde(default)]
    reasoning_content: Option<Value>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

impl Delta {
    fn thinking(&self) -> bool {
        carries_reasoning(&self.reasoning) || carries_reasoning(&self.reasoning_content)
    }
}

/// Did the endpoint put actual reasoning under this key? Deliberately untyped:
/// the text is never read, and gateways spell it as a string, an object, or a
/// list of blocks. Typing it as a string would make one of those shapes fail
/// the whole frame, taking its `content` and `tool_calls` down with it.
///
/// A bare `true` or a number is not reasoning: some endpoints carry a flag
/// under this name on every frame, and reading that as "the model is thinking"
/// would fire on the very first one, before the model produced anything.
fn carries_reasoning(field: &Option<Value>) -> bool {
    match field {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        _ => false,
    }
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

// ---- tool-calling ---------------------------------------------------------

/// Stream one assistant round that may include tool calls. Text streams to
/// `on_delta` and reasoning is reported through `on_reasoning` (see [`stream`]);
/// `tool_calls` are accumulated and returned. One retry on rate-limit/upstream
/// errors, one on the token-parameter refusal, and one on an endpoint that
/// refuses tools while the model reasons, all before any bytes stream. See
/// [`stream`]. Only this path carries the last: [`stream`] sends no tools.
pub async fn stream_tools(
    ep: &Endpoint,
    key: &str,
    request: &ToolRequest,
    on_delta: &mut impl FnMut(&str),
    on_reasoning: &mut impl FnMut(),
) -> Result<AssistantTurn> {
    let mut overloaded = false;
    let mut switched = false;
    let mut downgraded = false;
    let mut tokens_key = MAX_TOKENS;
    let mut effort = None;
    loop {
        match stream_tools_once(ep, key, request, tokens_key, effort, on_delta, on_reasoning).await
        {
            Err(e) if e.code() == "ai_overloaded" && !overloaded => {
                overloaded = true;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            // See [`stream`] for why the latches are separate.
            Err(e) if e.code() == "ai_token_param" && !switched => {
                switched = true;
                tokens_key = MAX_COMPLETION_TOKENS;
            }
            // Unlike the key switch above, this one costs the user an answer
            // from a model that no longer reasons.
            Err(e) if e.code() == "ai_reasoning_param" && !downgraded => {
                downgraded = true;
                effort = Some(EFFORT_NONE);
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
        // This wire format carries no replayable reasoning blocks.
        thinking: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_tools_once(
    ep: &Endpoint,
    key: &str,
    request: &ToolRequest,
    tokens_key: &str,
    effort: Option<&str>,
    on_delta: &mut impl FnMut(&str),
    on_reasoning: &mut impl FnMut(),
) -> Result<AssistantTurn> {
    let resp = post_chat(ep, key, &tool_body(request, tokens_key, effort))
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
                if choice.delta.thinking() {
                    on_reasoning();
                }
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
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
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

    fn a_request() -> Request {
        Request {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![ChatMessage {
                role: "user",
                content: "hi".into(),
            }],
            max_tokens: 42,
        }
    }

    #[test]
    fn chat_body_puts_system_first_and_streams() {
        let body = chat_body(&a_request(), MAX_TOKENS);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "sys");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 42);
    }

    /// The ceiling must move to the requested key, not be duplicated under
    /// both: OpenAI refuses a body still carrying `max_tokens`, and endpoints
    /// with a strict schema refuse one carrying `max_completion_tokens`.
    #[test]
    fn the_ceiling_moves_to_the_requested_key_and_leaves_no_twin() {
        let body = chat_body(&a_request(), MAX_COMPLETION_TOKENS);
        assert_eq!(body["max_completion_tokens"], 42);
        assert!(body.get("max_tokens").is_none());

        let tool_request = ToolRequest {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![json!({ "role": "user", "content": "hi" })],
            tools: Vec::new(),
            max_tokens: 42,
        };
        let body = tool_body(&tool_request, MAX_COMPLETION_TOKENS, None);
        assert_eq!(body["max_completion_tokens"], 42);
        assert!(body.get("max_tokens").is_none());
        let body = tool_body(&tool_request, MAX_TOKENS, None);
        assert_eq!(body["max_tokens"], 42);
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn only_an_endpoint_naming_both_keys_flips_the_parameter() {
        let body = |json: &str| serde_json::from_str::<Value>(json).unwrap();
        // OpenAI's rejection, verbatim — every reasoning model answers this.
        assert!(rejects_max_tokens(&body(
            r#"{"error":{"message":"Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.","type":"invalid_request_error","param":"max_tokens","code":"unsupported_parameter"}}"#
        )));
        // A gateway that flattens the envelope keeps only the message, and
        // spells `code` as the HTTP number; the message alone must carry it.
        assert!(rejects_max_tokens(&body(
            r#"{"error":{"message":"Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.","code":400}}"#
        )));
        // The mirror refusal — some Azure deployments reject the new name.
        // Flipping here would send back the key just refused.
        assert!(!rejects_max_tokens(&body(
            r#"{"error":{"message":"Unrecognized request argument supplied: max_completion_tokens","param":"max_completion_tokens"}}"#
        )));
        // A ceiling that is merely too large names one key, not two. Retrying
        // it under the other name would not fix it, and on a server that
        // ignores that name it would silently remove the ceiling.
        assert!(!rejects_max_tokens(&body(
            r#"{"error":{"message":"max_tokens must be less than or equal to 8192","param":"max_tokens"}}"#
        )));
        assert!(!rejects_max_tokens(&body(
            r#"{"error":{"message":"model not found"}}"#
        )));
        assert!(!rejects_max_tokens(&body(r#"{}"#)));
    }

    #[test]
    fn only_a_refusal_naming_its_own_remedy_turns_reasoning_off() {
        let body = |json: &str| serde_json::from_str::<Value>(json).unwrap();
        // Captured from api.openai.com, on a request that never mentioned
        // reasoning at all.
        assert!(refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"Function tools with reasoning_effort are not supported for gpt-5.6-terra in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.","type":"invalid_request_error","param":"reasoning_effort","code":null}}"#
        )));
        // An SDK that prefixes the status onto the message must still match.
        assert!(refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"400 Function tools with reasoning_effort are not supported for gpt-5.6-luna in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'."}}"#
        )));
        // The same opening, from a model that offers no remedy but an API this
        // client does not speak. Retrying under a value it never named would
        // spend a round-trip to be told the same thing — and `param` reads
        // `reasoning_effort` here too, so only the remedy separates them.
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"Function tools with reasoning_effort are not supported for gpt-5.4 in /v1/chat/completions. Please use /v1/responses instead.","param":"reasoning_effort"}}"#
        )));
        // Prose that happens to contain the word is not the endpoint naming the
        // value: the quotes are what make it a remedy rather than a mention.
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"Function tools with reasoning_effort are not supported for gpt-5.4, and none of the chat/completions models support this. Please use /v1/responses instead.","param":"reasoning_effort"}}"#
        )));
    }

    #[test]
    fn a_server_that_never_knew_the_parameter_is_left_alone() {
        let body = |json: &str| serde_json::from_str::<Value>(json).unwrap();
        // Groq does not know the key at all and suggests another one. Sending
        // it back is strictly worse than never having sent it.
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"property 'reasoning_effort' is unsupported, did you mean 'reasoning_format'?","type":"invalid_request_error"}}"#
        )));
        // vLLM knows the key and constrains its values; `none` is not among
        // them, so this endpoint would refuse the very retry we would send.
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"1 validation error: {'type': 'literal_error', 'loc': ('body', 'reasoning_effort'), 'msg': \"Input should be 'low', 'medium' or 'high'\"}","type":"Bad Request","code":400}}"#
        )));
        // Ollama maps the key onto its own and rejects off-list values.
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"invalid think value: \"minimal\" (must be \"high\", \"medium\", \"low\", true, or false)"}}"#
        )));
        // The Responses API moved the parameter rather than refusing tools.
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"Unsupported parameter: 'reasoning_effort'. In the Responses API, this parameter has moved to 'reasoning.effort'.","code":"unsupported_parameter"}}"#
        )));
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"Unrecognized request argument supplied: reasoning_effort"}}"#
        )));
        // Mistral answers with an object where the message should be; reading
        // it as a string must yield no match rather than panicking.
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":{"detail":[{"msg":"Input should be 'none' or 'high'","loc":["body","reasoning_effort"]}]}}}"#
        )));
        // The same refusal flattened to a string still names the key and the
        // word we key the remedy on, but it is a rejected *value*, not a
        // refusal to run tools. Only the opening sentence tells them apart.
        assert!(!refuses_tools_while_reasoning(&body(
            r#"{"error":{"message":"Input should be 'none' or 'high' [type=enum, loc=('body','reasoning_effort'), input='medium']","type":"invalid_request_error"}}"#
        )));
        assert!(!refuses_tools_while_reasoning(&body(r#"{}"#)));
    }

    /// The two parameter refusals travel the same `check_status` chain, so
    /// neither may answer for the other: one moves a key, the other takes the
    /// model's reasoning away.
    #[test]
    fn the_two_parameter_refusals_never_read_as_each_other() {
        let body = |json: &str| serde_json::from_str::<Value>(json).unwrap();
        let ceiling = body(
            r#"{"error":{"message":"Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.","param":"max_tokens"}}"#,
        );
        let reasoning = body(
            r#"{"error":{"message":"Function tools with reasoning_effort are not supported for gpt-5.6-sol in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.","param":"reasoning_effort"}}"#,
        );
        assert!(rejects_max_tokens(&ceiling) && !refuses_tools_while_reasoning(&ceiling));
        assert!(refuses_tools_while_reasoning(&reasoning) && !rejects_max_tokens(&reasoning));
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
        let body = tool_body(&base, MAX_TOKENS, None);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());

        let with_tools = ToolRequest {
            tools: vec![json!({ "type": "function" })],
            ..base
        };
        let body = tool_body(&with_tools, MAX_TOKENS, None);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tool_choice"], "auto");
    }

    /// The agent drops the tools for its last round, and that round writes the
    /// answer — so a downgrade negotiated earlier must not follow it there and
    /// spend the budget the reasoning was meant to use.
    #[test]
    fn reasoning_only_goes_off_where_the_tools_are() {
        let base = ToolRequest {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![json!({ "role": "user", "content": "hi" })],
            tools: Vec::new(),
            max_tokens: 42,
        };
        let body = tool_body(&base, MAX_TOKENS, Some(EFFORT_NONE));
        assert!(body.get("reasoning_effort").is_none());

        let with_tools = ToolRequest {
            tools: vec![json!({ "type": "function" })],
            ..base
        };
        let body = tool_body(&with_tools, MAX_TOKENS, Some(EFFORT_NONE));
        assert_eq!(body["reasoning_effort"], "none");
        // The tools are still there: the point is to keep them, not trade them.
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tool_choice"], "auto");
        // Never sent unasked: the default request carries no reasoning key at
        // all, whatever the endpoint might have defaulted to on its own.
        let body = tool_body(&with_tools, MAX_TOKENS, None);
        assert!(body.get("reasoning_effort").is_none());
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
    fn reasoning_is_spotted_under_either_spelling() {
        let delta = |json: &str| serde_json::from_str::<Delta>(json).unwrap();
        // `reasoning` is what OpenRouter and Ollama's /v1 send; DeepSeek and
        // vLLM spell it `reasoning_content`.
        assert!(delta(r#"{"content":"","reasoning":"let me think"}"#).thinking());
        assert!(delta(r#"{"content":"","reasoning_content":"let me think"}"#).thinking());
        // Answer text is not reasoning, and an empty field is not a sign of life.
        assert!(!delta(r#"{"content":"hello"}"#).thinking());
        assert!(!delta(r#"{"reasoning":""}"#).thinking());
        assert!(!delta(r#"{"reasoning":null}"#).thinking());
    }

    #[test]
    fn only_reasoning_content_counts_as_reasoning() {
        let delta = |json: &str| serde_json::from_str::<Delta>(json).unwrap();
        // A gateway may carry reasoning as an object or a list of blocks. The
        // frame must still parse: dropping it would drop its `content` too.
        let chunk: Chunk = serde_json::from_str(
            r#"{"choices":[{"delta":{"content":"hi","reasoning":{"text":"hmm"}}}]}"#,
        )
        .unwrap();
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
        assert!(chunk.choices[0].delta.thinking());
        assert!(delta(r#"{"reasoning_content":[{"type":"thinking"}]}"#).thinking());

        // An empty container is a placeholder, and a bare flag is not content:
        // an endpoint that sends either on every frame must not read as a model
        // reasoning from the very first one.
        assert!(!delta(r#"{"reasoning":{}}"#).thinking());
        assert!(!delta(r#"{"reasoning":[]}"#).thinking());
        assert!(!delta(r#"{"reasoning":false}"#).thinking());
        assert!(!delta(r#"{"reasoning":true}"#).thinking());
        assert!(!delta(r#"{"reasoning":0}"#).thinking());
    }

    #[test]
    fn one_delta_shape_serves_both_wire_formats() {
        // `tool_calls` is simply absent from a plain completion.
        let plain: Chunk =
            serde_json::from_str(r#"{"choices":[{"delta":{"content":"hi"}}]}"#).unwrap();
        assert!(plain.choices[0].delta.tool_calls.is_none());

        let tools: ToolChunk = serde_json::from_str(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1"}]}}]}"#,
        )
        .unwrap();
        assert_eq!(tools.choices[0].delta.tool_calls.as_ref().unwrap().len(), 1);
    }

    /// A server that answers `replies` in order, one per connection, and
    /// records every request body it was sent. Enough of an endpoint to drive
    /// the retry loop, which is otherwise reachable only over the wire.
    async fn scripted(
        replies: Vec<(u16, String)>,
    ) -> (String, tokio::task::JoinHandle<Vec<serde_json::Value>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut seen = Vec::new();
            for (status, body) in replies {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                // Read until the headers end, then the declared body length.
                loop {
                    let n = sock.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let len: usize = text
                            .to_ascii_lowercase()
                            .split("content-length:")
                            .nth(1)
                            .and_then(|t| t.split("\r\n").next())
                            .and_then(|t| t.trim().parse().ok())
                            .unwrap_or(0);
                        if raw.len() >= head_end + 4 + len {
                            seen.push(
                                serde_json::from_slice(&raw[head_end + 4..])
                                    .unwrap_or(serde_json::Value::Null),
                            );
                            break;
                        }
                    }
                }
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
                let _ = sock.shutdown().await;
            }
            seen
        });
        (format!("http://127.0.0.1:{port}"), server)
    }

    fn a_tool_request() -> ToolRequest {
        ToolRequest {
            model: "m".into(),
            system: "sys".into(),
            messages: vec![json!({ "role": "user", "content": "hi" })],
            tools: vec![json!({ "type": "function" })],
            max_tokens: 42,
        }
    }

    const REFUSAL: &str = r#"{"error":{"message":"Function tools with reasoning_effort are not supported for gpt-5.6-terra in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.","param":"reasoning_effort"}}"#;

    /// The refusal must actually produce a second request carrying the value
    /// the endpoint named, with the tools it was refused over.
    #[tokio::test]
    async fn the_retry_goes_out_with_reasoning_off() {
        let done = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let (base_url, server) =
            scripted(vec![(400, REFUSAL.to_string()), (200, done.to_string())]).await;
        let ep = Endpoint {
            base_url,
            attribution: false,
        };
        let turn = stream_tools(&ep, "", &a_tool_request(), &mut |_: &str| {}, &mut || {})
            .await
            .unwrap();

        assert_eq!(turn.text, "hi");
        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 2, "the refusal is retried exactly once");
        assert!(seen[0].get("reasoning_effort").is_none());
        assert_eq!(seen[1]["reasoning_effort"], "none");
        // The retry keeps the tools it was refused over. Dropping them would
        // answer the round without the search results it was asked for.
        assert_eq!(seen[1]["tools"], seen[0]["tools"]);
        assert_eq!(seen[1]["tool_choice"], "auto");
    }

    /// An endpoint that refuses even under `none` must give up, not spin: the
    /// latch is the only thing bounding this loop against a metered API.
    #[tokio::test]
    async fn a_refusal_that_never_lifts_gives_up_after_one_retry() {
        let (base_url, server) = scripted(vec![
            (400, REFUSAL.to_string()),
            (400, REFUSAL.to_string()),
            (400, REFUSAL.to_string()),
        ])
        .await;
        let ep = Endpoint {
            base_url,
            attribution: false,
        };
        let err = stream_tools(&ep, "", &a_tool_request(), &mut |_: &str| {}, &mut || {})
            .await
            .unwrap_err();

        assert_eq!(err.code(), "ai_reasoning_param");
        server.abort();
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
