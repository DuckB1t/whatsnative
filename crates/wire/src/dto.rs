//! Data Transfer Objects (DTOs) for the WhatsNative wire protocol.

use serde::{Deserialize, Serialize};

/// Mirrors the `is_false` spelling used by core and IPC so a skipped boolean
/// stays absent from the frame rather than becoming `false`.
fn is_false(value: &bool) -> bool {
    !*value
}

/// Snapshot of the daemon's connection to WhatsApp servers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionStatusDto {
    pub state: String,
    pub phone: Option<String>,
    pub name: Option<String>,
    pub jid: Option<String>,
    pub lid: Option<String>,
    pub qr_ascii: Option<String>,
    pub pair_code: Option<String>,
    pub pair_expires_at_ms: Option<i64>,
}

/// One conversation/chat summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatDto {
    pub jid: String,
    pub name: String,
    pub unread_count: u32,
    pub manually_unread: bool,
    pub is_group: bool,
    pub is_pinned: bool,
    pub is_muted: bool,
    pub is_archived: bool,
    pub last_message_ts: Option<i64>,
    pub last_message_preview: Option<String>,
}

/// One message in a conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDto {
    pub id: String,
    pub chat_jid: String,
    pub sender_jid: String,
    pub from_me: bool,
    pub timestamp_ms: i64,
    pub text: Option<String>,
    /// "text", "image", "video", "audio", "document", "sticker", "poll", "reaction", "revoked", "call", "system"
    pub kind: String,
    /// "pending", "sent", "delivered", "read", "failed"
    pub status: String,
    pub is_starred: bool,
    pub reply_to_id: Option<String>,
    pub media: Option<MediaDto>,
    pub reactions: Vec<ReactionDto>,
    /// The poll this message opens, when `kind` is `"poll"`. Tallies are
    /// not carried — see [`PollDto` as built by `poll_view_to_dto`]: votes
    /// arrive as separate updates and this task counts none of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<MessagePollDto>,
}

/// Media attachment metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaDto {
    pub file_sha256: String,
    pub mime_type: String,
    pub filename: Option<String>,
    pub size_bytes: u64,
    pub is_downloaded: bool,
    pub local_path: Option<String>,
}

/// Emoji reaction to a message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionDto {
    pub sender_jid: String,
    pub emoji: String,
    pub timestamp_ms: i64,
}

/// Address book / WhatsApp contact entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactDto {
    pub jid: String,
    pub name: Option<String>,
    pub push_name: Option<String>,
    pub phone: Option<String>,
    pub is_business: bool,
    pub alias: Option<String>,
    pub tags: Vec<String>,
}

/// WhatsApp group summary and metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupDto {
    pub jid: String,
    pub subject: String,
    pub description: Option<String>,
    pub owner_jid: Option<String>,
    pub participant_count: usize,
    pub announce_only: bool,
    pub locked: bool,
    pub participants: Vec<GroupParticipantDto>,
}

/// One participant in a group.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupParticipantDto {
    pub jid: String,
    pub is_admin: bool,
    pub is_superadmin: bool,
}

/// Action to perform on a group participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupParticipantAction {
    Add,
    Remove,
    Promote,
    Demote,
}

/// A poll within a conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollDto {
    pub id: String,
    pub chat_jid: String,
    pub question: String,
    pub options: Vec<PollOptionDto>,
    pub selectable_count: u32,
}

/// One option within a poll.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollOptionDto {
    pub index: u32,
    pub name: String,
    pub vote_count: u32,
    pub voters: Vec<String>,
}

/// A poll carried on a message, as the creation describes it.
///
/// Separate from [`PollDto`] on purpose: that one names a stored poll by
/// id and chat for the poll endpoints, while this one travels on the
/// message that opens it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagePollDto {
    pub question: String,
    pub options: Vec<String>,
    pub selectable_count: u32,
}

/// User presence status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    Available,
    Unavailable,
    Composing,
    Paused,
    Recording,
}

/// WhatsApp profile details.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDto {
    pub jid: String,
    pub name: Option<String>,
    pub about: Option<String>,
    pub picture_url: Option<String>,
    /// Lookup outcome; absent for daemons predating this field.
    #[serde(default)]
    pub picture_status: Option<String>,
    #[serde(default)]
    pub pn_jid: Option<String>,
    #[serde(default)]
    pub lid: Option<String>,
    /// Where each identity was learned: request, session, protocol, or local_mapping.
    #[serde(default)]
    pub pn_source: Option<String>,
    #[serde(default)]
    pub lid_source: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub verified_name: Option<String>,
    #[serde(default)]
    pub is_business: Option<bool>,
}

/// One participant in a group call's authoritative roster.
///
/// The name is optional because the library's snapshot carries only a JID and
/// the session resolves labels against its address book; an unresolved label
/// is still a participant worth showing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallParticipantDto {
    pub jid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<u32>,
}

/// Call event metadata in chat history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEventDto {
    pub id: String,
    pub chat_jid: String,
    pub caller_jid: String,
    pub timestamp_ms: i64,
    pub is_video: bool,
    pub is_group: bool,
    /// The group this call belonged to, when it was a group call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_jid: Option<String>,
    /// The last roster the session saw, in transaction order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<CallParticipantDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_transaction_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_media: Option<String>,
    pub duration_seconds: Option<u32>,
    pub outcome: String, // "connected", "missed", "declined", "cancelled"
}

/// A broadcast channel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDto {
    pub jid: String,
    /// Viewer update mute setting; absent means unknown, not unmuted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_muted: Option<bool>,
    pub name: String,
    pub description: Option<String>,
    pub subscriber_count: u64,
    pub picture_url: Option<String>,
}

/// One pending group membership request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupJoinRequestDto {
    pub jid: String,
    pub request_time_secs: Option<u64>,
}

/// One locally known account profile: a daemon socket with a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDto {
    pub id: String,
    pub socket_path: String,
    pub active: bool,
}

/// Local storage usage diagnostics.
///
/// `database_bytes` is the *shared* store: every local account lives in one
/// file keyed by `device_id`, and SQLite has no per-account size without the
/// `dbstat` module this build trims. It is reported as what it is — the whole
/// store — rather than as one account's share. `media_bytes`/`media_files` are
/// account-scoped: the media directory is shared but every key carries its
/// account's prefix.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDto {
    pub database_bytes: u64,
    pub media_bytes: u64,
    pub media_files: u64,
}

/// Doctor / health-check diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorDto {
    pub daemon_running: bool,
    pub socket_path: String,
    pub connection_state: String,
    pub database_ok: bool,
    pub database_bytes: u64,
    pub media_cache_dir: String,
    pub media_cache_bytes: u64,
}

// --- Protocol 50 payloads ---
//
// These are wire-owned mirrors of the shapes core and IPC already put on the
// socket. This crate deliberately has no dependency on either: the contract is
// serde parity — the same field names, nesting and spellings — not Rust type
// identity. Keep the attributes in step with the definitions they mirror.

/// One address on the account's block list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedContact {
    /// The address as the server wrote it: a PN or a LID, never a group.
    pub jid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Unix seconds, when the server supplied a time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_at: Option<u64>,
}

/// The account a username resolved to.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResolvedUsername {
    pub jid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pn_jid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_name: Option<String>,
}

/// What a username lookup answered. `KeyRequired` is distinct from `NotFound`:
/// a second ask with the account's key can still resolve the handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsernameLookupResult {
    Found(ResolvedUsername),
    KeyRequired {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    },
    NotFound,
    InvalidUsername,
}

/// Where an event happens. Coordinates are protocol floats, so `PartialEq` is
/// derived and `Eq` is asserted explicitly, the same as core: protocol
/// coordinates are finite and a NaN is not a location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventLocation {
    pub degrees_latitude: f64,
    pub degrees_longitude: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Eq for EventLocation {}

/// What somebody answered on an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRsvp {
    Going,
    NotGoing,
    Maybe,
}

/// The message an operation created, addressed so a front end can select it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedMessageRef {
    pub chat_jid: String,
    pub message_id: String,
}

/// Whether a community or subgroup is listed for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityVisibility {
    Visible,
    Hidden,
}

/// What creating a community was asked to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityDraft {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub closed: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_non_admin_sub_group_creation: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub create_general_chat: bool,
}

/// A subgroup to create under an existing community.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubgroupDraft {
    pub parent_jid: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<String>,
    pub visibility: CommunityVisibility,
}

/// One group to attach to a community, with the listing it should get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSubgroupTarget {
    pub jid: String,
    pub visibility: CommunityVisibility,
}

/// Attach existing groups to a community in one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSubgroupsRequest {
    pub community_jid: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<LinkSubgroupTarget>,
}

/// Ask to join a community's subgroup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinSubgroupRequest {
    pub community_jid: String,
    pub subgroup_jid: String,
}

/// A group as the community list knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityGroupRef {
    pub jid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One group an operation could not handle, so the rest can still be reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunitySubgroupFailure {
    pub jid: String,
    pub code: u32,
}

/// Which step a non-fatal warning came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityWarningStep {
    SetDescription,
    LinkSubgroup,
    JoinApprovalPending,
}

/// Something that did not complete but did not fail the whole operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityWarning {
    pub step: CommunityWarningStep,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The whole answer to a community operation, including its partial failures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CommunityOperationResult {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<CommunityGroupRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_jids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed_groups: Vec<CommunitySubgroupFailure>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<CommunityWarning>,
}

/// A poll to open in a channel. An empty `options` still defaults/additively
/// decodes, exactly as it does in core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPollDraft {
    pub channel_jid: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    pub selectable_count: u32,
}

/// A reaction to a channel message. An empty `emoji` removes the sender's
/// reaction rather than adding one with no text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelReact {
    pub channel_jid: String,
    pub message_id: String,
    pub emoji: String,
}

/// A vote on a channel poll, by the indices of the chosen options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelVotePoll {
    pub channel_jid: String,
    pub poll_message_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub option_indices: Vec<u32>,
}

/// The message a channel operation created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPollCreated {
    pub channel_jid: String,
    pub message_id: String,
}

// IPC-only payloads: these are protocol structs rather than domain types, so
// they are mirrored here too rather than imported from either crate.

/// The address a block or unblock names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockTarget {
    pub jid: String,
}

/// An event creation. Every field but the destination and the name is optional
/// on the wire, so an older front end sending only those two still asks for a
/// valid card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateEvent {
    pub jid: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<i64>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EventLocation>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_scheduled_call: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub extra_guests_allowed: bool,
}

/// An answer to an event invitation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RespondEvent {
    pub chat_jid: String,
    pub event_message_id: String,
    pub response: EventRsvp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_guest_count: Option<i32>,
}

/// A plain-text post to a channel. The optimistic bubble's id travels with it
/// so the drawn bubble and the durable row name the same message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostToChannel {
    pub channel_jid: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::CallEventDto;

    #[test]
    fn a_call_from_an_older_daemon_needs_no_group_fields() {
        let json = r#"{
            "id":"c1","chat_jid":"a@s.whatsapp.net","caller_jid":"a@s.whatsapp.net",
            "timestamp_ms":1,"is_video":false,"is_group":false,
            "duration_seconds":null,"outcome":"missed"
        }"#;
        let call: CallEventDto = serde_json::from_str(json).expect("old frame still reads");
        assert_eq!(call.group_jid, None);
        assert!(call.participants.is_empty());
        assert_eq!(call.group_transaction_id, None);
        assert_eq!(call.group_media, None);
    }
}
