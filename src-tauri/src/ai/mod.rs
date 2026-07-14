pub mod agent;
pub mod anthropic;
pub mod openrouter;
pub mod prompts;
pub mod retrieval;

/// One turn in a chat-style request, shared by both providers. Roles are only
/// ever "user" or "assistant"; the system prompt is passed separately.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// A tool the model asked to run, assembled from a provider's streamed
/// response. `input` is the parsed arguments object (`{}` when the model sent
/// none), regardless of provider wire format.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// One assistant round of a tool-calling exchange, normalized across
/// providers. `stop_reason` is `Some("tool_use")` when the model requested
/// tools (Anthropic's `tool_use` / OpenAI's `tool_calls`).
#[derive(Debug, Default)]
pub struct AssistantTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: Option<String>,
}
