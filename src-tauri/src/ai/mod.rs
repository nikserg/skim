pub mod agent;
pub mod anthropic;
pub mod attachments;
pub mod ollama;
pub mod openai_compat;
pub mod openrouter;
pub mod prompts;
pub mod retrieval;

use std::time::Duration;

/// How long we wait to reach a provider before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a stream may go *silent* before we call it dead. This is a gap
/// between reads, not a cap on the whole request: an answer can legitimately
/// take minutes, and a total timeout would cut long rounds short. Generous
/// enough that a local model loading itself into memory isn't mistaken for a
/// stall. Without it a stuck connection leaves the UI on "Thinking…" forever.
const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// The HTTP client for every call to a model provider.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        // Only fails if the TLS backend won't initialize, which is fatal
        // anyway — and is exactly what the plain constructor does.
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// One turn in a chat-style request, shared by both providers. Roles are only
/// ever "user" or "assistant"; the system prompt is passed separately.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// A binary attachment handed to the model natively (Anthropic `document` /
/// `image` content blocks). Only produced for the Anthropic provider; the
/// OpenRouter path falls back to local text extraction instead.
#[derive(Debug, Clone)]
pub enum MediaKind {
    Pdf,
    Image,
}

#[derive(Debug, Clone)]
pub struct MediaBlock {
    pub kind: MediaKind,
    /// MIME type, e.g. "application/pdf" or "image/png".
    pub media_type: String,
    pub data_base64: String,
    pub filename: String,
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

/// A reasoning block from a model that thinks before answering. Anthropic
/// requires these to be replayed untouched on later rounds of a tool-calling
/// exchange, so they are carried through the loop rather than dropped — we
/// never show them to the user.
#[derive(Debug, Clone)]
pub enum ThinkingBlock {
    /// Signed reasoning. `signature` must survive the round trip verbatim.
    Thinking { text: String, signature: String },
    /// Encrypted reasoning the client can't read, only hand back.
    Redacted { data: String },
}

/// One assistant round of a tool-calling exchange, normalized across
/// providers. `stop_reason` is `Some("tool_use")` when the model requested
/// tools (Anthropic's `tool_use` / OpenAI's `tool_calls`).
#[derive(Debug, Default)]
pub struct AssistantTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: Option<String>,
    /// Reasoning blocks, in the order the model emitted them. Empty for
    /// providers that don't return any.
    pub thinking: Vec<ThinkingBlock>,
}
