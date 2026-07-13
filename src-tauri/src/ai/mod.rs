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
