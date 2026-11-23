//! One result of a full-text search over stored messages.
//!
//! Search is a local read, so a result carries only what a front end needs to
//! name the row, show a line about it, and, for a stored media row, say what
//! kind it is when the stored proto proves it. The full message and its bytes
//! stay in the store; a client that wants to render them asks for the message
//! or its page.

use serde::{Deserialize, Serialize};

/// A chat-store full-text hit, independent of any transport.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Stored media kind label when this row carries media, such as `image` or
    /// `document`. `None` for rows that are not media or were projected before
    /// this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_kind: Option<String>,
    /// File name from the stored media metadata when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// MIME type from the stored media metadata when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Pixel width from stored media metadata when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Pixel height from stored media metadata when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// The stored proto carries display bytes, such as a JPEG thumbnail or a
    /// video poster frame. This says those bytes exist, not that they decode;
    /// a front end that cannot draw them still draws an honest kind card.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_stored_preview: bool,
    /// Whether the stored media envelope has download metadata. A document
    /// can be worth listing and saving even with no local bytes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fetchable: bool,
}
