use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub provider: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: String,
    pub auth_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: i64,
    pub account_id: String,
    pub imap_name: String,
    pub role: Option<String>,
    pub display_name: String,
    pub unread_count: i64,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub name: Option<String>,
    pub addr: String,
}

/// Projection for the message list pane: one row per thread in a folder,
/// shaped by the latest message.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRow {
    pub id: i64,
    /// In flat (ungrouped) mode this row represents a single message; carries
    /// its message id. `None` in grouped mode, where the row is a whole thread.
    pub message_id: Option<i64>,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub snippet: String,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMeta {
    pub id: i64,
    pub folder_id: i64,
    pub thread_id: Option<i64>,
    pub subject: String,
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub date: i64,
    pub snippet: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub body_state: i64,
    /// True when the message carries a usable `List-Unsubscribe` header, so the
    /// reading pane can offer an unsubscribe chip. The full target stays in the
    /// DB and is never sent to the frontend — the chip just triggers the action.
    pub can_unsubscribe: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    pub id: i64,
    pub message_id: i64,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
    pub is_inline: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetail {
    pub id: i64,
    pub subject: String,
    pub messages: Vec<MessageMeta>,
}

/// Calendar invitation extracted from a message's text/calendar part,
/// ready for the invite card in the reading pane.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteView {
    pub method: String, // "request" | "cancel" | "reply"
    pub uid: String,
    pub sequence: i64,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    /// Unix seconds (UTC) for timed events; None for all-day.
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub is_all_day: bool,
    /// "YYYY-MM-DD", inclusive range, for all-day events.
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub rrule: Option<String>,
    pub attendee_count: usize,
    /// The user's stored answer: "accepted" | "declined" | "tentative".
    pub my_response: Option<String>,
    /// For method == "reply": who answered and how.
    pub reply_attendee: Option<String>,
    pub reply_partstat: Option<String>,
    pub can_rsvp: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedBody {
    pub message_id: i64,
    /// Sanitized HTML (or plain text converted to safe HTML).
    pub html: String,
    pub blocked_images: usize,
    pub from_addr: Option<String>,
    pub attachments: Vec<AttachmentMeta>,
    pub invite: Option<InviteView>,
}

/// Headers of a message as parsed from the wire, ready for insertion.
#[derive(Debug, Clone, Default)]
pub struct NewMessage {
    pub account_id: String,
    pub folder_id: i64,
    pub uid: u32,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub subject: Option<String>,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub to_addrs: Vec<Address>,
    pub cc_addrs: Vec<Address>,
    pub date: i64,
    pub snippet: Option<String>,
    pub size: Option<i64>,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    /// Raw `List-Unsubscribe` header value (comma-separated `<uri>` list), if present.
    pub list_unsubscribe: Option<String>,
    /// `List-Unsubscribe-Post: List-Unsubscribe=One-Click` present (RFC 8058).
    pub list_unsubscribe_one_click: bool,
}
