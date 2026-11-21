//! Serde parity for the protocol 50 payloads shared by the daemon and the CLI.
//!
//! `oxidezap-wire` deliberately depends on neither `oxidezap-core` nor
//! `oxidezap-ipc`: the contract is the JSON the two sides agree on, not Rust
//! type identity. IPC tags its frames at the top level (`"request"` /
//! `"type"`, payload fields beside the tag); this crate nests the same payload
//! fields under `action`/`params` and `type`/`data`. The assertions below name
//! both spellings so a field renamed on either side is visible here. Every
//! address, id and name in this file is synthetic.

use oxidezap_wire::{
    BlockTarget, BlockedContact, ChannelPollCreated, ChannelPollDraft, ChannelReact,
    ChannelVotePoll, ClientRequest, CommunityDraft, CommunityGroupRef, CommunityOperationResult,
    CommunitySubgroupFailure, CommunityVisibility, CommunityWarning, CommunityWarningStep,
    CreateEvent, CreatedMessageRef, DaemonResponse, EventLocation, EventRsvp, JoinSubgroupRequest,
    LinkSubgroupTarget, LinkSubgroupsRequest, PostToChannel, RequestEnvelope, ResolvedUsername,
    RespondEvent, ResponseEnvelope, SubgroupDraft, UsernameLookupResult,
};
use serde_json::{Value, json};

fn assert_request(request: ClientRequest, expected: Value) {
    let envelope = RequestEnvelope { id: 42, request };
    let frame = serde_json::to_value(&envelope).expect("a request serializes");
    assert_eq!(frame, expected, "a request changed shape on the wire");
    let decoded: RequestEnvelope = serde_json::from_value(frame).expect("a request deserializes");
    assert_eq!(decoded, envelope, "a request did not round-trip");
}

fn assert_response(response: DaemonResponse, expected: Value) {
    let envelope = ResponseEnvelope::success(42, response);
    let frame = serde_json::to_value(&envelope).expect("a response serializes");
    assert_eq!(frame, expected, "a response changed shape on the wire");
    let decoded: ResponseEnvelope = serde_json::from_value(frame).expect("a response deserializes");
    assert_eq!(decoded, envelope, "a response did not round-trip");
}

#[test]
fn block_and_username_requests_put_ipc_payloads_on_the_wire() {
    // IPC: {"request":"blocklist"}, {"request":"block","jid":"..."},
    // {"request":"unblock","jid":"..."} and
    // {"request":"lookup_username","username":"...","username_key":"..."}.
    assert_request(
        ClientRequest::Blocklist,
        json!({"id": 42, "action": "blocklist"}),
    );
    assert_request(
        ClientRequest::Block(BlockTarget {
            jid: "15555550100@s.whatsapp.net".into(),
        }),
        json!({
            "id": 42,
            "action": "block",
            "params": {"jid": "15555550100@s.whatsapp.net"}
        }),
    );
    assert_request(
        ClientRequest::Unblock(BlockTarget {
            jid: "100000000000001@lid".into(),
        }),
        json!({
            "id": 42,
            "action": "unblock",
            "params": {"jid": "100000000000001@lid"}
        }),
    );
    assert_request(
        ClientRequest::LookupUsername {
            username: "example.handle".into(),
            username_key: None,
        },
        json!({
            "id": 42,
            "action": "lookup_username",
            "params": {"username": "example.handle"}
        }),
    );
    assert_request(
        ClientRequest::LookupUsername {
            username: "example.handle".into(),
            username_key: Some("0000".into()),
        },
        json!({
            "id": 42,
            "action": "lookup_username",
            "params": {"username": "example.handle", "username_key": "0000"}
        }),
    );
}

#[test]
fn blocklist_and_username_answers_put_ipc_fields_on_the_wire() {
    // IPC: {"type":"blocklist","id":..,"entries":[...]} — the entries keep the
    // server's spelling and leave absent optionals out entirely.
    assert_response(
        DaemonResponse::Blocklist {
            entries: vec![
                BlockedContact {
                    jid: "15555550100@s.whatsapp.net".into(),
                    name: Some("Synthetic Contact".into()),
                    blocked_at: Some(1_700_000_000),
                },
                BlockedContact {
                    jid: "100000000000001@lid".into(),
                    name: None,
                    blocked_at: None,
                },
            ],
        },
        json!({
            "id": 42,
            "status": "ok",
            "type": "blocklist",
            "data": {"entries": [
                {
                    "jid": "15555550100@s.whatsapp.net",
                    "name": "Synthetic Contact",
                    "blocked_at": 1_700_000_000
                },
                {"jid": "100000000000001@lid"}
            ]}
        }),
    );
    assert_response(
        DaemonResponse::Blocklist { entries: vec![] },
        json!({
            "id": 42,
            "status": "ok",
            "type": "blocklist",
            "data": {"entries": []}
        }),
    );

    // IPC keeps the four lookup outcomes distinct: `key_required` is an object,
    // the two terminal outcomes are bare strings.
    assert_response(
        DaemonResponse::UsernameLookup {
            result: UsernameLookupResult::Found(ResolvedUsername {
                jid: "100000000000001@lid".into(),
                pn_jid: Some("15555550100@s.whatsapp.net".into()),
                username: Some("example.handle".into()),
                name: Some("Synthetic Contact".into()),
                verified_name: Some("Synthetic Store".into()),
            }),
        },
        json!({
            "id": 42,
            "status": "ok",
            "type": "username_lookup",
            "data": {"result": {"found": {
                "jid": "100000000000001@lid",
                "pn_jid": "15555550100@s.whatsapp.net",
                "username": "example.handle",
                "name": "Synthetic Contact",
                "verified_name": "Synthetic Store"
            }}}
        }),
    );
    assert_response(
        DaemonResponse::UsernameLookup {
            result: UsernameLookupResult::KeyRequired { username: None },
        },
        json!({
            "id": 42,
            "status": "ok",
            "type": "username_lookup",
            "data": {"result": {"key_required": {}}}
        }),
    );
    assert_response(
        DaemonResponse::UsernameLookup {
            result: UsernameLookupResult::KeyRequired {
                username: Some("spelled.back".into()),
            },
        },
        json!({
            "id": 42,
            "status": "ok",
            "type": "username_lookup",
            "data": {"result": {"key_required": {"username": "spelled.back"}}}
        }),
    );
    for (result, expected_result) in [
        (UsernameLookupResult::NotFound, json!("not_found")),
        (
            UsernameLookupResult::InvalidUsername,
            json!("invalid_username"),
        ),
    ] {
        assert_response(
            DaemonResponse::UsernameLookup { result },
            json!({
                "id": 42,
                "status": "ok",
                "type": "username_lookup",
                "data": {"result": expected_result}
            }),
        );
    }
}

#[test]
fn event_requests_and_answer_put_ipc_fields_on_the_wire() {
    // IPC: {"request":"create_event","jid":"...","name":"..."} with every
    // other CreateEvent field skipped when it is absent or false.
    assert_request(
        ClientRequest::CreateEvent(Box::new(CreateEvent {
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
        json!({
            "id": 42,
            "action": "create_event",
            "params": {
                "jid": "155555500001@g.us",
                "name": "Synthetic standup"
            }
        }),
    );
    assert_request(
        ClientRequest::CreateEvent(Box::new(CreateEvent {
            jid: "155555500001@g.us".into(),
            name: "Synthetic standup".into(),
            description: Some("A fixture, not an invitation".into()),
            start_time: Some(1_700_000_000),
            end_time: Some(1_700_000_600),
            join_link: Some("https://example.invalid/call".into()),
            location: Some(EventLocation {
                degrees_latitude: -12.5,
                degrees_longitude: 40.25,
                name: Some("Synthetic Room".into()),
            }),
            is_scheduled_call: true,
            extra_guests_allowed: true,
        })),
        json!({
            "id": 42,
            "action": "create_event",
            "params": {
                "jid": "155555500001@g.us",
                "name": "Synthetic standup",
                "description": "A fixture, not an invitation",
                "start_time": 1_700_000_000,
                "end_time": 1_700_000_600,
                "join_link": "https://example.invalid/call",
                "location": {
                    "degrees_latitude": -12.5,
                    "degrees_longitude": 40.25,
                    "name": "Synthetic Room"
                },
                "is_scheduled_call": true,
                "extra_guests_allowed": true
            }
        }),
    );

    // IPC: {"request":"respond_event",...,"response":"not_going"} and
    // `"response":"going"` with an optional `extra_guest_count`.
    assert_request(
        ClientRequest::RespondEvent(RespondEvent {
            chat_jid: "155555500001@g.us".into(),
            event_message_id: "SYNTHETIC-1".into(),
            response: EventRsvp::NotGoing,
            extra_guest_count: None,
        }),
        json!({
            "id": 42,
            "action": "respond_event",
            "params": {
                "chat_jid": "155555500001@g.us",
                "event_message_id": "SYNTHETIC-1",
                "response": "not_going"
            }
        }),
    );
    assert_request(
        ClientRequest::RespondEvent(RespondEvent {
            chat_jid: "155555500001@g.us".into(),
            event_message_id: "SYNTHETIC-1".into(),
            response: EventRsvp::Going,
            extra_guest_count: Some(2),
        }),
        json!({
            "id": 42,
            "action": "respond_event",
            "params": {
                "chat_jid": "155555500001@g.us",
                "event_message_id": "SYNTHETIC-1",
                "response": "going",
                "extra_guest_count": 2
            }
        }),
    );

    // IPC: {"type":"event_created","id":..,"created":{chat_jid,message_id}}.
    assert_response(
        DaemonResponse::EventCreated {
            created: CreatedMessageRef {
                chat_jid: "155555500001@g.us".into(),
                message_id: "SYNTHETIC-1".into(),
            },
        },
        json!({
            "id": 42,
            "status": "ok",
            "type": "event_created",
            "data": {"created": {
                "chat_jid": "155555500001@g.us",
                "message_id": "SYNTHETIC-1"
            }}
        }),
    );
}

#[test]
fn community_requests_and_answer_put_ipc_fields_on_the_wire() {
    // IPC: {"request":"create_community","name":"..."} — description and all
    // three booleans are skipped when absent/false.
    assert_request(
        ClientRequest::CreateCommunity(CommunityDraft {
            name: "Synthetic community".into(),
            description: None,
            closed: false,
            allow_non_admin_sub_group_creation: false,
            create_general_chat: false,
        }),
        json!({
            "id": 42,
            "action": "create_community",
            "params": {"name": "Synthetic community"}
        }),
    );
    assert_request(
        ClientRequest::CreateCommunity(CommunityDraft {
            name: "Synthetic community".into(),
            description: Some("A synthetic description".into()),
            closed: true,
            allow_non_admin_sub_group_creation: true,
            create_general_chat: true,
        }),
        json!({
            "id": 42,
            "action": "create_community",
            "params": {
                "name": "Synthetic community",
                "description": "A synthetic description",
                "closed": true,
                "allow_non_admin_sub_group_creation": true,
                "create_general_chat": true
            }
        }),
    );

    // IPC: {"request":"create_community_subgroup",...,"visibility":"visible"}
    // with participants omitted while empty.
    assert_request(
        ClientRequest::CreateCommunitySubgroup(SubgroupDraft {
            parent_jid: "155555500001@g.us".into(),
            name: "Synthetic subgroup".into(),
            participants: vec![],
            visibility: CommunityVisibility::Visible,
        }),
        json!({
            "id": 42,
            "action": "create_community_subgroup",
            "params": {
                "parent_jid": "155555500001@g.us",
                "name": "Synthetic subgroup",
                "visibility": "visible"
            }
        }),
    );
    assert_request(
        ClientRequest::CreateCommunitySubgroup(SubgroupDraft {
            parent_jid: "155555500001@g.us".into(),
            name: "Synthetic subgroup".into(),
            participants: vec!["155555500002@s.whatsapp.net".into()],
            visibility: CommunityVisibility::Hidden,
        }),
        json!({
            "id": 42,
            "action": "create_community_subgroup",
            "params": {
                "parent_jid": "155555500001@g.us",
                "name": "Synthetic subgroup",
                "participants": ["155555500002@s.whatsapp.net"],
                "visibility": "hidden"
            }
        }),
    );

    // IPC: {"request":"link_community_subgroups",...,"groups":[{jid,visibility}]}.
    assert_request(
        ClientRequest::LinkCommunitySubgroups(LinkSubgroupsRequest {
            community_jid: "155555500001@g.us".into(),
            groups: vec![LinkSubgroupTarget {
                jid: "155555500002@g.us".into(),
                visibility: CommunityVisibility::Hidden,
            }],
        }),
        json!({
            "id": 42,
            "action": "link_community_subgroups",
            "params": {
                "community_jid": "155555500001@g.us",
                "groups": [{"jid": "155555500002@g.us", "visibility": "hidden"}]
            }
        }),
    );
    assert_request(
        ClientRequest::LinkCommunitySubgroups(LinkSubgroupsRequest {
            community_jid: "155555500001@g.us".into(),
            groups: vec![],
        }),
        json!({
            "id": 42,
            "action": "link_community_subgroups",
            "params": {"community_jid": "155555500001@g.us"}
        }),
    );
    assert_request(
        ClientRequest::JoinCommunitySubgroup(JoinSubgroupRequest {
            community_jid: "155555500001@g.us".into(),
            subgroup_jid: "155555500003@g.us".into(),
        }),
        json!({
            "id": 42,
            "action": "join_community_subgroup",
            "params": {
                "community_jid": "155555500001@g.us",
                "subgroup_jid": "155555500003@g.us"
            }
        }),
    );

    // IPC: {"type":"community_result","id":..,"result":{...}} with partial
    // failures reported beside the successful groups, and `{}` when empty.
    assert_response(
        DaemonResponse::CommunityResult {
            result: CommunityOperationResult {
                groups: vec![CommunityGroupRef {
                    jid: "155555500001@g.us".into(),
                    subject: Some("Synthetic group".into()),
                    description: None,
                }],
                linked_jids: vec!["155555500002@g.us".into()],
                failed_groups: vec![CommunitySubgroupFailure {
                    jid: "155555500003@g.us".into(),
                    code: 403,
                }],
                warnings: vec![CommunityWarning {
                    step: CommunityWarningStep::JoinApprovalPending,
                    jid: Some("155555500004@g.us".into()),
                    detail: None,
                }],
            },
        },
        json!({
            "id": 42,
            "status": "ok",
            "type": "community_result",
            "data": {"result": {
                "groups": [{"jid": "155555500001@g.us", "subject": "Synthetic group"}],
                "linked_jids": ["155555500002@g.us"],
                "failed_groups": [{"jid": "155555500003@g.us", "code": 403}],
                "warnings": [{
                    "step": "join_approval_pending",
                    "jid": "155555500004@g.us"
                }]
            }}
        }),
    );
    assert_response(
        DaemonResponse::CommunityResult {
            result: CommunityOperationResult::default(),
        },
        json!({
            "id": 42,
            "status": "ok",
            "type": "community_result",
            "data": {"result": {}}
        }),
    );
}

#[test]
fn channel_requests_and_answer_put_ipc_fields_on_the_wire() {
    // IPC: {"request":"post_to_channel",...,"local_id":"..."} — an absent
    // optimistic id is skipped, not sent as null.
    assert_request(
        ClientRequest::PostToChannel(PostToChannel {
            channel_jid: "155555500001@newsletter".into(),
            text: "Synthetic post".into(),
            local_id: Some("SYNTHETIC-LOCAL".into()),
        }),
        json!({
            "id": 42,
            "action": "post_to_channel",
            "params": {
                "channel_jid": "155555500001@newsletter",
                "text": "Synthetic post",
                "local_id": "SYNTHETIC-LOCAL"
            }
        }),
    );
    assert_request(
        ClientRequest::PostToChannel(PostToChannel {
            channel_jid: "155555500001@newsletter".into(),
            text: "Synthetic post".into(),
            local_id: None,
        }),
        json!({
            "id": 42,
            "action": "post_to_channel",
            "params": {
                "channel_jid": "155555500001@newsletter",
                "text": "Synthetic post"
            }
        }),
    );

    // IPC: {"request":"create_channel_poll",...,"options":[...]} with the
    // option list omitted while empty.
    assert_request(
        ClientRequest::CreateChannelPoll(ChannelPollDraft {
            channel_jid: "155555500001@newsletter".into(),
            question: "Synthetic question".into(),
            options: vec!["First".into(), "Second".into()],
            selectable_count: 1,
        }),
        json!({
            "id": 42,
            "action": "create_channel_poll",
            "params": {
                "channel_jid": "155555500001@newsletter",
                "question": "Synthetic question",
                "options": ["First", "Second"],
                "selectable_count": 1
            }
        }),
    );
    assert_request(
        ClientRequest::CreateChannelPoll(ChannelPollDraft {
            channel_jid: "155555500001@newsletter".into(),
            question: "Synthetic question".into(),
            options: vec![],
            selectable_count: 1,
        }),
        json!({
            "id": 42,
            "action": "create_channel_poll",
            "params": {
                "channel_jid": "155555500001@newsletter",
                "question": "Synthetic question",
                "selectable_count": 1
            }
        }),
    );

    // An empty emoji is a removal marker, so it must survive serialization.
    assert_request(
        ClientRequest::ReactToChannel(ChannelReact {
            channel_jid: "155555500001@newsletter".into(),
            message_id: "SYNTHETIC-1".into(),
            emoji: String::new(),
        }),
        json!({
            "id": 42,
            "action": "react_to_channel",
            "params": {
                "channel_jid": "155555500001@newsletter",
                "message_id": "SYNTHETIC-1",
                "emoji": ""
            }
        }),
    );
    assert_request(
        ClientRequest::VoteChannelPoll(ChannelVotePoll {
            channel_jid: "155555500001@newsletter".into(),
            poll_message_id: "SYNTHETIC-2".into(),
            option_indices: vec![0, 2],
        }),
        json!({
            "id": 42,
            "action": "vote_channel_poll",
            "params": {
                "channel_jid": "155555500001@newsletter",
                "poll_message_id": "SYNTHETIC-2",
                "option_indices": [0, 2]
            }
        }),
    );
    assert_request(
        ClientRequest::VoteChannelPoll(ChannelVotePoll {
            channel_jid: "155555500001@newsletter".into(),
            poll_message_id: "SYNTHETIC-2".into(),
            option_indices: vec![],
        }),
        json!({
            "id": 42,
            "action": "vote_channel_poll",
            "params": {
                "channel_jid": "155555500001@newsletter",
                "poll_message_id": "SYNTHETIC-2"
            }
        }),
    );

    // IPC: {"type":"channel_poll_created","id":..,"created":{...}}.
    assert_response(
        DaemonResponse::ChannelPollCreated {
            created: ChannelPollCreated {
                channel_jid: "155555500001@newsletter".into(),
                message_id: "SYNTHETIC-3".into(),
            },
        },
        json!({
            "id": 42,
            "status": "ok",
            "type": "channel_poll_created",
            "data": {"created": {
                "channel_jid": "155555500001@newsletter",
                "message_id": "SYNTHETIC-3"
            }}
        }),
    );
}
