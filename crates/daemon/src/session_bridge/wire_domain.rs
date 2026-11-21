//! Explicit conversions between protocol-50 wire DTOs and domain types.
//!
//! `oxidezap-wire` mirrors core and IPC by serde shape, not by Rust identity:
//! it has no dependency on either crate, so a `From` implementation here would
//! run into the orphan rule even if it were the right answer. Naming every
//! field is the point. A wire field that is renamed or added fails to compile
//! until this file decides what it means, rather than quietly arriving on the
//! other side as a default.
//!
//! Requests travel *wire → core*: the session owns the domain shapes and must
//! not learn about an envelope. Results travel *core → wire*: the client owns
//! the DTOs and must not learn about a session. Nothing here goes through
//! JSON — a serde round trip accepts a field the other side never reads and
//! calls the result parity.
//!
//! Every optional stays optional and every collection is moved rather than
//! rebuilt, so an absent value does not turn into a present default on the way
//! across.

use oxidezap_core as core;
use oxidezap_session::EventDraft;
use oxidezap_wire::dto as wire;

// --- Wire -> core: requests the session carries out ---

/// The pin of an event, without the chat it belongs to.
#[must_use]
pub(crate) fn event_location_from_wire(location: wire::EventLocation) -> core::EventLocation {
    core::EventLocation {
        degrees_latitude: location.degrees_latitude,
        degrees_longitude: location.degrees_longitude,
        name: location.name,
    }
}

/// A creation as the session expresses it. The destination stays on the
/// request; the draft deliberately does not carry a second spelling of it.
#[must_use]
pub(crate) fn event_draft_from_wire(event: wire::CreateEvent) -> EventDraft {
    let wire::CreateEvent {
        jid: _,
        name,
        description,
        start_time,
        end_time,
        join_link,
        location,
        is_scheduled_call,
        extra_guests_allowed,
    } = event;
    EventDraft {
        name,
        description,
        start_time,
        end_time,
        join_link,
        location: location.map(event_location_from_wire),
        is_scheduled_call,
        extra_guests_allowed,
    }
}

/// An answer to an invitation, with no extra guest count — that travels beside
/// the response as its own optional field.
#[must_use]
pub(crate) fn event_rsvp_from_wire(rsvp: wire::EventRsvp) -> core::EventRsvp {
    match rsvp {
        wire::EventRsvp::Going => core::EventRsvp::Going,
        wire::EventRsvp::NotGoing => core::EventRsvp::NotGoing,
        wire::EventRsvp::Maybe => core::EventRsvp::Maybe,
    }
}

#[must_use]
fn community_visibility_from_wire(
    visibility: wire::CommunityVisibility,
) -> core::CommunityVisibility {
    match visibility {
        wire::CommunityVisibility::Visible => core::CommunityVisibility::Visible,
        wire::CommunityVisibility::Hidden => core::CommunityVisibility::Hidden,
    }
}

#[must_use]
pub(crate) fn community_draft_from_wire(draft: wire::CommunityDraft) -> core::CommunityDraft {
    core::CommunityDraft {
        name: draft.name,
        description: draft.description,
        closed: draft.closed,
        allow_non_admin_sub_group_creation: draft.allow_non_admin_sub_group_creation,
        create_general_chat: draft.create_general_chat,
    }
}

#[must_use]
pub(crate) fn subgroup_draft_from_wire(draft: wire::SubgroupDraft) -> core::SubgroupDraft {
    core::SubgroupDraft {
        parent_jid: draft.parent_jid,
        name: draft.name,
        participants: draft.participants,
        visibility: community_visibility_from_wire(draft.visibility),
    }
}

#[must_use]
fn link_subgroup_target_from_wire(target: wire::LinkSubgroupTarget) -> core::LinkSubgroupTarget {
    core::LinkSubgroupTarget {
        jid: target.jid,
        visibility: community_visibility_from_wire(target.visibility),
    }
}

#[must_use]
pub(crate) fn link_subgroups_request_from_wire(
    request: wire::LinkSubgroupsRequest,
) -> core::LinkSubgroupsRequest {
    core::LinkSubgroupsRequest {
        community_jid: request.community_jid,
        groups: request
            .groups
            .into_iter()
            .map(link_subgroup_target_from_wire)
            .collect(),
    }
}

#[must_use]
pub(crate) fn join_subgroup_request_from_wire(
    request: wire::JoinSubgroupRequest,
) -> core::JoinSubgroupRequest {
    core::JoinSubgroupRequest {
        community_jid: request.community_jid,
        subgroup_jid: request.subgroup_jid,
    }
}

#[must_use]
pub(crate) fn channel_poll_draft_from_wire(
    draft: wire::ChannelPollDraft,
) -> core::ChannelPollDraft {
    core::ChannelPollDraft {
        channel_jid: draft.channel_jid,
        question: draft.question,
        options: draft.options,
        selectable_count: draft.selectable_count,
    }
}

#[must_use]
pub(crate) fn channel_react_from_wire(react: wire::ChannelReact) -> core::ChannelReact {
    core::ChannelReact {
        channel_jid: react.channel_jid,
        message_id: react.message_id,
        // An empty emoji is a removal marker, not a missing field to default.
        emoji: react.emoji,
    }
}

#[must_use]
pub(crate) fn channel_vote_poll_from_wire(vote: wire::ChannelVotePoll) -> core::ChannelVotePoll {
    core::ChannelVotePoll {
        channel_jid: vote.channel_jid,
        poll_message_id: vote.poll_message_id,
        option_indices: vote.option_indices,
    }
}

// --- Core -> wire: answers the client draws ---

#[must_use]
pub(crate) fn blocked_contact_to_wire(contact: core::BlockedContact) -> wire::BlockedContact {
    wire::BlockedContact {
        jid: contact.jid,
        name: contact.name,
        blocked_at: contact.blocked_at,
    }
}

#[must_use]
fn resolved_username_to_wire(resolved: core::ResolvedUsername) -> wire::ResolvedUsername {
    wire::ResolvedUsername {
        jid: resolved.jid,
        pn_jid: resolved.pn_jid,
        username: resolved.username,
        name: resolved.name,
        verified_name: resolved.verified_name,
    }
}

/// All four outcomes stay distinct: `KeyRequired` is not `NotFound`, and the
/// handle it carries is optional on both sides.
#[must_use]
pub(crate) fn username_lookup_result_to_wire(
    result: core::UsernameLookupResult,
) -> wire::UsernameLookupResult {
    match result {
        core::UsernameLookupResult::Found(resolved) => {
            wire::UsernameLookupResult::Found(resolved_username_to_wire(resolved))
        }
        core::UsernameLookupResult::KeyRequired { username } => {
            wire::UsernameLookupResult::KeyRequired { username }
        }
        core::UsernameLookupResult::NotFound => wire::UsernameLookupResult::NotFound,
        core::UsernameLookupResult::InvalidUsername => wire::UsernameLookupResult::InvalidUsername,
    }
}

#[must_use]
pub(crate) fn created_message_ref_to_wire(
    created: core::CreatedMessageRef,
) -> wire::CreatedMessageRef {
    wire::CreatedMessageRef {
        chat_jid: created.chat_jid,
        message_id: created.message_id,
    }
}

#[must_use]
fn community_group_ref_to_wire(group: core::CommunityGroupRef) -> wire::CommunityGroupRef {
    wire::CommunityGroupRef {
        jid: group.jid,
        subject: group.subject,
        description: group.description,
    }
}

#[must_use]
fn community_subgroup_failure_to_wire(
    failure: core::CommunitySubgroupFailure,
) -> wire::CommunitySubgroupFailure {
    wire::CommunitySubgroupFailure {
        jid: failure.jid,
        code: failure.code,
    }
}

#[must_use]
fn community_warning_step_to_wire(step: core::CommunityWarningStep) -> wire::CommunityWarningStep {
    match step {
        core::CommunityWarningStep::SetDescription => wire::CommunityWarningStep::SetDescription,
        core::CommunityWarningStep::LinkSubgroup => wire::CommunityWarningStep::LinkSubgroup,
        core::CommunityWarningStep::JoinApprovalPending => {
            wire::CommunityWarningStep::JoinApprovalPending
        }
    }
}

#[must_use]
fn community_warning_to_wire(warning: core::CommunityWarning) -> wire::CommunityWarning {
    wire::CommunityWarning {
        step: community_warning_step_to_wire(warning.step),
        jid: warning.jid,
        detail: warning.detail,
    }
}

/// Partial successes are the answer, not a footnote: linked and refused groups
/// and every warning keep their order and their optional fields.
#[must_use]
pub(crate) fn community_operation_result_to_wire(
    result: core::CommunityOperationResult,
) -> wire::CommunityOperationResult {
    wire::CommunityOperationResult {
        groups: result
            .groups
            .into_iter()
            .map(community_group_ref_to_wire)
            .collect(),
        linked_jids: result.linked_jids,
        failed_groups: result
            .failed_groups
            .into_iter()
            .map(community_subgroup_failure_to_wire)
            .collect(),
        warnings: result
            .warnings
            .into_iter()
            .map(community_warning_to_wire)
            .collect(),
    }
}

#[must_use]
pub(crate) fn channel_poll_created_to_wire(
    created: core::ChannelPollCreated,
) -> wire::ChannelPollCreated {
    wire::ChannelPollCreated {
        channel_jid: created.channel_jid,
        message_id: created.message_id,
    }
}
