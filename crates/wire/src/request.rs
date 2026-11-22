//! Client requests for the OxideZap wire protocol.

use serde::{Deserialize, Serialize};

use crate::dto::{
    BlockTarget, ChannelPollDraft, ChannelReact, ChannelVotePoll, CommunityDraft, CreateEvent,
    GroupParticipantAction, JoinSubgroupRequest, LinkSubgroupsRequest, PostToChannel,
    PresenceState, RespondEvent, SubgroupDraft,
};

/// What a request does to the account or to local state.
///
/// Declared per variant rather than inferred from a list of mutating ones,
/// because an inferred list is permissive by default: a new request that
/// forgets to register itself is silently allowed through a read-only
/// connection. The exhaustive `match` in [`ClientRequest::access`] makes the
/// compiler refuse to build until the author answers the question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reads state and changes nothing.
    Read,
    /// Writes account or local state; refused on a read-only connection.
    Write,
}

/// Domain-oriented requests sent by frontends (CLI, GUI, scripts) to the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", content = "params", rename_all = "snake_case")]
pub enum ClientRequest {
    /// Handshake initiating connection, declaring client capabilities and read-only mode.
    Hello {
        protocol: u32,
        client_name: String,
        /// Local SQLite account within this daemon, independent of its profile/socket.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_account: Option<i32>,
        /// Require account-selection acknowledgement before any business request.
        #[serde(default)]
        account_selection: bool,
        read_only: bool,
        session_events: bool,
    },
    /// Query current connection state and credentials.
    GetStatus,

    // --- Messages ---
    ListMessages {
        chat_jid: String,
        #[serde(default = "default_limit")]
        limit: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
    },
    GetMessage {
        chat_jid: String,
        message_id: String,
    },
    GetMessageContext {
        chat_jid: String,
        message_id: String,
        #[serde(default = "default_context_limit")]
        limit: usize,
    },
    SearchMessages {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chat_jid: Option<String>,
        #[serde(default)]
        has_media: bool,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    ListStarredMessages {
        #[serde(default = "default_limit")]
        limit: usize,
    },
    EditMessage {
        chat_jid: String,
        message_id: String,
        new_text: String,
    },
    RevokeMessage {
        chat_jid: String,
        message_id: String,
        for_everyone: bool,
    },
    ForwardMessage {
        source_chat_jid: String,
        message_id: String,
        target_chat_jid: String,
    },
    /// Set or clear a message's starred flag.
    ///
    /// Mirrors the legacy `ClientRequest::SetStarred`; the daemon answers with
    /// an ack on success and a correlated error when the app-state mutation or
    /// its local persistence fails.
    SetStarred {
        chat_jid: String,
        message_id: String,
        starred: bool,
    },
    /// Pin or unpin a stored message.
    ///
    /// Mirrors the legacy `ClientRequest::SetMessagePinned`; the daemon answers
    /// with an ack on success and a correlated error when the mutation or its
    /// local persistence fails. The library has no "forever" pin, so the
    /// longest available duration is used.
    SetMessagePinned {
        chat_jid: String,
        message_id: String,
        pinned: bool,
    },

    // --- Sends ---
    SendText {
        to: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mentions: Vec<String>,
        #[serde(default)]
        enqueue_only: bool,
    },
    SendMedia {
        to: String,
        file_path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default)]
        as_document: bool,
    },
    SendAudio {
        to: String,
        file_path: String,
        #[serde(default)]
        ptt: bool,
    },
    SendReaction {
        chat_jid: String,
        message_id: String,
        emoji: String,
    },
    SendPoll {
        to: String,
        question: String,
        options: Vec<String>,
        #[serde(default = "default_poll_selectable")]
        selectable_count: u32,
    },
    VotePoll {
        chat_jid: String,
        poll_id: String,
        selected_option_indices: Vec<u32>,
    },
    SendLocation {
        to: String,
        latitude: f64,
        longitude: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    SendStatus {
        text: String,
    },
    SendSticker {
        to: String,
        file_path: String,
    },
    /// Answer an interactive list message by selecting one of its rows.
    SendListResponse {
        to: String,
        title: String,
        row_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
    },

    // --- Chats ---
    ListChats {
        #[serde(default = "default_limit")]
        limit: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        offset: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default)]
        archived: bool,
    },
    GetChat {
        jid: String,
    },
    MarkRead {
        chat_jid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        through_message_id: Option<String>,
    },
    MarkUnread {
        chat_jid: String,
    },
    PinChat {
        chat_jid: String,
        pin: bool,
    },
    MuteChat {
        chat_jid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mute_duration_seconds: Option<u64>,
    },
    ArchiveChat {
        chat_jid: String,
        archive: bool,
    },

    // --- Media ---
    DownloadMedia {
        chat_jid: String,
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        destination: Option<String>,
    },
    RetryMedia {
        chat_jid: String,
        message_id: String,
    },

    // --- Contacts ---
    ListContacts {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    CheckContact {
        phone: String,
    },
    GetContact {
        jid: String,
    },
    RefreshContacts {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        jid: Option<String>,
    },
    SetContactAlias {
        jid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
    },
    TagContact {
        jid: String,
        tag: String,
    },
    UntagContact {
        jid: String,
        tag: String,
    },

    // --- Groups ---
    ListGroups,
    GetGroupInfo {
        group_jid: String,
    },
    CreateGroup {
        subject: String,
        participants: Vec<String>,
    },
    SetGroupTopic {
        group_jid: String,
        topic: String,
    },
    SetGroupDescription {
        group_jid: String,
        description: String,
    },
    ManageGroupParticipant {
        group_jid: String,
        participant_jid: String,
        action: GroupParticipantAction,
    },
    GetGroupInviteLink {
        group_jid: String,
        #[serde(default)]
        reset: bool,
    },
    JoinGroup {
        invite_code: String,
    },
    LeaveGroup {
        group_jid: String,
    },

    // --- Profile & Presence ---
    GetProfile {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        jid: Option<String>,
    },
    SetProfileAbout {
        about: String,
    },
    SetProfileName {
        name: String,
    },
    SetProfilePicture {
        file_path: String,
    },
    RemoveProfilePicture,
    SetPresence {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chat_jid: Option<String>,
        state: PresenceState,
    },

    // --- Polls ---
    ListPolls {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chat_jid: Option<String>,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    GetPoll {
        chat_jid: String,
        poll_id: String,
    },

    // --- Channels ---
    ListChannels,
    GetChannelInfo {
        channel_jid: String,
    },
    JoinChannel {
        channel_jid: String,
    },
    LeaveChannel {
        channel_jid: String,
    },

    // --- Blocklist & usernames (protocol 50) ---
    /// Read the account's block list.
    Blocklist,
    /// Put one address on the account's block list.
    Block(BlockTarget),
    /// Take one address off the account's block list.
    Unblock(BlockTarget),
    /// Resolve a Meta username through the account held by the daemon.
    LookupUsername {
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username_key: Option<String>,
    },

    // --- Events (protocol 50) ---
    /// Create a calendar event in a chat.
    CreateEvent(Box<CreateEvent>),
    /// Answer a calendar invitation.
    RespondEvent(RespondEvent),

    // --- Communities (protocol 50) ---
    /// Create a community and, when asked, its general chat.
    CreateCommunity(CommunityDraft),
    /// Create a subgroup under an existing community.
    CreateCommunitySubgroup(SubgroupDraft),
    /// Attach existing groups to a community, with their visibility.
    LinkCommunitySubgroups(LinkSubgroupsRequest),
    /// Join one community's subgroup.
    JoinCommunitySubgroup(JoinSubgroupRequest),

    // --- Native channels (protocol 50) ---
    /// Post plain text to a channel.
    PostToChannel(PostToChannel),
    /// Open a poll in a channel.
    CreateChannelPoll(ChannelPollDraft),
    /// React to a channel message, or remove our reaction with an empty emoji.
    ReactToChannel(ChannelReact),
    /// Replace a channel poll ballot by option index.
    VoteChannelPoll(ChannelVotePoll),

    // --- Calls & History ---
    ListCalls {
        #[serde(default = "default_limit")]
        limit: usize,
    },
    HistoryCoverage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chat_jid: Option<String>,
    },
    HistoryBackfill {
        chat_jid: String,
        #[serde(default = "default_backfill_count")]
        count: u32,
    },

    // --- Media & Store maintenance ---
    BackfillMedia {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chat_jid: Option<String>,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    CleanupChats,
    PurgeMessages {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chat_jid: Option<String>,
    },

    // --- Profile extra ---
    GetBusinessProfile {
        jid: String,
    },

    // --- Groups extra ---
    SetGroupPermissions {
        group_jid: String,
        announce_only: bool,
        locked: bool,
    },
    ListGroupJoinRequests {
        group_jid: String,
    },
    ManageGroupJoinRequest {
        group_jid: String,
        participant_jid: String,
        approve: bool,
    },

    // --- Auth ---
    /// Ask the primary device for a phone-number pairing code.
    RequestPairCode {
        phone: String,
    },

    // --- Accounts ---
    ListAccounts,

    // --- Diagnostics & Storage ---
    GetStorageUsage,
    ClearMediaCache,
    DoctorCheck,
    ForgetSession,
    Shutdown,
}

impl ClientRequest {
    /// What this request does, per variant and without a catch-all.
    ///
    /// The counterpart to the old `matches!` list, which was permissive by
    /// default and had quietly left `RefreshContacts`, `BackfillMedia`,
    /// `HistoryBackfill`, `DownloadMedia` and `GetGroupInviteLink { reset: true }`
    /// out. A variant added here fails to compile until it declares itself.
    pub fn access(&self) -> Access {
        match self {
            // Reads: no account or local state is changed. `RequestPairCode`
            // mints credentials on the server, so it is a write.
            Self::Hello { .. }
            | Self::GetStatus
            | Self::ListMessages { .. }
            | Self::GetMessage { .. }
            | Self::GetMessageContext { .. }
            | Self::SearchMessages { .. }
            | Self::ListStarredMessages { .. }
            | Self::ListChats { .. }
            | Self::GetChat { .. }
            | Self::ListContacts { .. }
            | Self::CheckContact { .. }
            | Self::GetContact { .. }
            | Self::GetGroupInfo { .. }
            | Self::ListGroupJoinRequests { .. }
            | Self::GetProfile { .. }
            | Self::GetBusinessProfile { .. }
            | Self::ListPolls { .. }
            | Self::GetPoll { .. }
            | Self::ListChannels
            | Self::GetChannelInfo { .. }
            | Self::ListCalls { .. }
            | Self::HistoryCoverage { .. }
            | Self::GetStorageUsage
            | Self::DoctorCheck
            | Self::ListAccounts => Access::Read,

            // Both answer with a typed payload under the request id and change
            // nothing: asking for a list or resolving a handle stays a read.
            Self::Blocklist | Self::LookupUsername { .. } => Access::Read,

            // Listing groups reaches the server for the participating query,
            // but writes nothing: a read-only connection may still list them.
            Self::ListGroups => Access::Read,

            // `reset: true` mints a new link on the server; showing one only
            // reads it.
            Self::GetGroupInviteLink { reset: false, .. } => Access::Read,

            Self::GetGroupInviteLink { reset: true, .. }
            | Self::RequestPairCode { .. }
            | Self::EditMessage { .. }
            | Self::RevokeMessage { .. }
            | Self::ForwardMessage { .. }
            | Self::SetStarred { .. }
            | Self::SetMessagePinned { .. }
            | Self::SendText { .. }
            | Self::SendMedia { .. }
            | Self::SendAudio { .. }
            | Self::SendReaction { .. }
            | Self::SendPoll { .. }
            | Self::VotePoll { .. }
            | Self::SendLocation { .. }
            | Self::SendStatus { .. }
            | Self::SendSticker { .. }
            | Self::SendListResponse { .. }
            | Self::SetContactAlias { .. }
            | Self::TagContact { .. }
            | Self::UntagContact { .. }
            | Self::RefreshContacts { .. }
            | Self::SetGroupPermissions { .. }
            | Self::ManageGroupJoinRequest { .. }
            | Self::JoinChannel { .. }
            | Self::LeaveChannel { .. }
            | Self::CleanupChats
            | Self::PurgeMessages { .. }
            | Self::MarkRead { .. }
            | Self::MarkUnread { .. }
            | Self::PinChat { .. }
            | Self::MuteChat { .. }
            | Self::ArchiveChat { .. }
            | Self::DownloadMedia { .. }
            | Self::RetryMedia { .. }
            | Self::BackfillMedia { .. }
            | Self::HistoryBackfill { .. }
            | Self::CreateGroup { .. }
            | Self::SetGroupTopic { .. }
            | Self::SetGroupDescription { .. }
            | Self::ManageGroupParticipant { .. }
            | Self::JoinGroup { .. }
            | Self::LeaveGroup { .. }
            | Self::SetProfileAbout { .. }
            | Self::SetProfileName { .. }
            | Self::SetProfilePicture { .. }
            | Self::RemoveProfilePicture
            | Self::SetPresence { .. }
            | Self::ClearMediaCache
            | Self::ForgetSession
            // Protocol 50 account and channel operations. A block, event,
            // community or channel write reaches the session, not just local
            // state, so a read-only connection must refuse all of them.
            | Self::Block(..)
            | Self::Unblock(..)
            | Self::CreateEvent(..)
            | Self::RespondEvent(..)
            | Self::CreateCommunity(..)
            | Self::CreateCommunitySubgroup(..)
            | Self::LinkCommunitySubgroups(..)
            | Self::JoinCommunitySubgroup(..)
            | Self::PostToChannel(..)
            | Self::CreateChannelPoll(..)
            | Self::ReactToChannel(..)
            | Self::VoteChannelPoll(..)
            | Self::Shutdown => Access::Write,
        }
    }

    /// Whether this request mutates account or local state.
    pub fn is_mutation(&self) -> bool {
        self.access() == Access::Write
    }
}

fn default_limit() -> usize {
    50
}

fn default_context_limit() -> usize {
    5
}

fn default_poll_selectable() -> u32 {
    1
}

fn default_backfill_count() -> u32 {
    50
}

#[cfg(test)]
mod tests {
    use super::{Access, ClientRequest as R};
    use crate::dto::{
        BlockTarget, ChannelPollDraft, ChannelReact, ChannelVotePoll, CommunityDraft,
        CommunityVisibility, CreateEvent, EventRsvp, JoinSubgroupRequest, LinkSubgroupsRequest,
        PostToChannel, RespondEvent, SubgroupDraft,
    };

    /// The requests the old `matches!` list left out. Each one changes account
    /// or local state, so a read-only connection must refuse it.
    #[test]
    fn previously_unlisted_mutations_are_writes() {
        for request in [
            R::RefreshContacts { jid: None },
            R::BackfillMedia {
                chat_jid: None,
                limit: 50,
            },
            R::HistoryBackfill {
                chat_jid: "x@s.whatsapp.net".into(),
                count: 50,
            },
            R::DownloadMedia {
                chat_jid: "x@s.whatsapp.net".into(),
                message_id: "m".into(),
                destination: None,
            },
            R::GetGroupInviteLink {
                group_jid: "g@g.us".into(),
                reset: true,
            },
            R::RequestPairCode {
                phone: "5511999999999".into(),
            },
            R::SendListResponse {
                to: "x@s.whatsapp.net".into(),
                title: "menu".into(),
                row_id: "row-1".into(),
                reply_to: None,
            },
            R::SetStarred {
                chat_jid: "x@s.whatsapp.net".into(),
                message_id: "SYNTHETIC-1".into(),
                starred: true,
            },
            R::SetMessagePinned {
                chat_jid: "x@s.whatsapp.net".into(),
                message_id: "SYNTHETIC-2".into(),
                pinned: true,
            },
        ] {
            assert_eq!(request.access(), Access::Write, "{request:?}");
        }
    }

    /// Showing an invite link reads it; only a reset writes one.
    #[test]
    fn showing_an_invite_link_is_a_read() {
        assert_eq!(
            R::GetGroupInviteLink {
                group_jid: "g@g.us".into(),
                reset: false,
            }
            .access(),
            Access::Read
        );
        assert!(
            !R::GetGroupInviteLink {
                group_jid: "g@g.us".into(),
                reset: false,
            }
            .is_mutation()
        );
    }

    /// Reads stay reads, so a read-only agent keeps working.
    #[test]
    fn listing_and_looking_up_do_not_mutate() {
        for request in [
            R::GetStatus,
            R::ListMessages {
                chat_jid: "x@s.whatsapp.net".into(),
                limit: 50,
                before: None,
                after: None,
            },
            R::SearchMessages {
                query: "oi".into(),
                chat_jid: None,
                has_media: false,
                limit: 50,
            },
            R::ListGroups,
            R::ListChats {
                limit: 50,
                offset: None,
                query: None,
                archived: false,
            },
            R::HistoryCoverage { chat_jid: None },
            R::ListCalls { limit: 50 },
            R::ListAccounts,
        ] {
            assert_eq!(request.access(), Access::Read, "{request:?}");
        }
    }

    /// Every new protocol 50 mutation reaches the account, so a read-only
    /// connection must refuse it.
    #[test]
    fn new_mutations_are_writes() {
        for request in [
            R::Block(BlockTarget {
                jid: "15555550100@s.whatsapp.net".into(),
            }),
            R::Unblock(BlockTarget {
                jid: "100000000000001@lid".into(),
            }),
            R::CreateEvent(Box::new(CreateEvent {
                jid: "155555500001@g.us".into(),
                name: "Synthetic standup".into(),
                description: None,
                start_time: None,
                end_time: None,
                join_link: None,
                location: None,
                is_scheduled_call: false,
                extra_guests_allowed: false,
            })),
            R::RespondEvent(RespondEvent {
                chat_jid: "155555500001@g.us".into(),
                event_message_id: "SYNTHETIC-1".into(),
                response: EventRsvp::Going,
                extra_guest_count: None,
            }),
            R::CreateCommunity(CommunityDraft {
                name: "Synthetic community".into(),
                description: None,
                closed: false,
                allow_non_admin_sub_group_creation: false,
                create_general_chat: false,
            }),
            R::CreateCommunitySubgroup(SubgroupDraft {
                parent_jid: "155555500001@g.us".into(),
                name: "Synthetic subgroup".into(),
                participants: vec![],
                visibility: CommunityVisibility::Visible,
            }),
            R::LinkCommunitySubgroups(LinkSubgroupsRequest {
                community_jid: "155555500001@g.us".into(),
                groups: vec![],
            }),
            R::JoinCommunitySubgroup(JoinSubgroupRequest {
                community_jid: "155555500001@g.us".into(),
                subgroup_jid: "155555500002@g.us".into(),
            }),
            R::PostToChannel(PostToChannel {
                channel_jid: "155555500001@newsletter".into(),
                text: "Synthetic post".into(),
                local_id: None,
            }),
            R::CreateChannelPoll(ChannelPollDraft {
                channel_jid: "155555500001@newsletter".into(),
                question: "Synthetic question".into(),
                options: vec!["First".into(), "Second".into()],
                selectable_count: 1,
            }),
            R::ReactToChannel(ChannelReact {
                channel_jid: "155555500001@newsletter".into(),
                message_id: "SYNTHETIC-1".into(),
                emoji: String::new(),
            }),
            R::VoteChannelPoll(ChannelVotePoll {
                channel_jid: "155555500001@newsletter".into(),
                poll_message_id: "SYNTHETIC-2".into(),
                option_indices: vec![0],
            }),
        ] {
            assert_eq!(request.access(), Access::Write, "{request:?}");
            assert!(request.is_mutation(), "{request:?}");
        }
    }

    /// Reading a list or resolving a handle changes nothing, so a read-only
    /// agent keeps both of them.
    #[test]
    fn new_reads_stay_reads() {
        for request in [
            R::Blocklist,
            R::LookupUsername {
                username: "example.handle".into(),
                username_key: None,
            },
            R::LookupUsername {
                username: "example.handle".into(),
                username_key: Some("0000".into()),
            },
        ] {
            assert_eq!(request.access(), Access::Read, "{request:?}");
            assert!(!request.is_mutation(), "{request:?}");
        }
    }
}
