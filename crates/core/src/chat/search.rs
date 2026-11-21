//! One result of a full-text search over stored messages.
//!
//! Search is a local read, so a result carries only what a front end needs to
//! name the row and show a line about it. The full message stays in the store;
//! a client that wants to render it asks for the message or its page.

use serde::{Deserialize, Serialize};

/// A chat-store full-text hit, independent of any transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSearchHit {
    /// The chat the hit belongs to, including a chat filter the caller passed.
    pub chat_jid: String,
    /// The stored message id, unique within `chat_jid`.
    pub message_id: String,
    /// A bounded plain-text projection of the matched row.
    pub snippet: String,
    /// Who wrote it, when the store knows; `None` for rows with no sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_jid: Option<String>,
    /// When the message was sent or received, in Unix milliseconds.
    pub timestamp_ms: i64,
    /// Whether the matched row's content class carries an attachment.
    pub has_media: bool,
}
