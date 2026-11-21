//! Serde parity for the protocol 52 message actions.
//!
//! `oxidezap-wire` still depends on neither `oxidezap-core` nor `oxidezap-ipc`:
//! the contract is the JSON the two sides agree on. IPC tags its frames at the
//! top level (`"request"` / `"type"`, payload fields beside the tag); this
//! crate nests the same payload fields under `action`/`params` and
//! `type`/`data`. The literals below name the wire spelling so a field renamed
//! on either side is visible here. Every address and id is synthetic.

use oxidezap_wire::{ClientRequest, DaemonResponse, RequestEnvelope, ResponseEnvelope};
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
fn protocol_52_message_actions_keep_their_wire_shape() {
    // The legacy plane spells these `request` plus flat fields; the wire
    // nests the same fields under `params`.
    assert_request(
        ClientRequest::ForwardMessage {
            source_chat_jid: "155555500001@g.us".into(),
            message_id: "SYNTHETIC-1".into(),
            target_chat_jid: "155555500002@s.whatsapp.net".into(),
        },
        json!({
            "id": 42,
            "action": "forward_message",
            "params": {
                "source_chat_jid": "155555500001@g.us",
                "message_id": "SYNTHETIC-1",
                "target_chat_jid": "155555500002@s.whatsapp.net"
            }
        }),
    );
    assert_request(
        ClientRequest::SetStarred {
            chat_jid: "155555500001@g.us".into(),
            message_id: "SYNTHETIC-2".into(),
            starred: true,
        },
        json!({
            "id": 42,
            "action": "set_starred",
            "params": {
                "chat_jid": "155555500001@g.us",
                "message_id": "SYNTHETIC-2",
                "starred": true
            }
        }),
    );
    assert_request(
        ClientRequest::SearchMessages {
            query: "synthetic".into(),
            chat_jid: None,
            has_media: false,
            limit: 50,
        },
        json!({
            "id": 42,
            "action": "search_messages",
            "params": {"query": "synthetic", "has_media": false, "limit": 50}
        }),
    );

    // The mentions list is additive: an empty one is absent, a composed one
    // travels as an array beside the reply metadata.
    assert_request(
        ClientRequest::SendText {
            to: "155555500002@s.whatsapp.net".into(),
            message: "hello".into(),
            reply_to: None,
            mentions: vec!["155555500003@s.whatsapp.net".into()],
            enqueue_only: false,
        },
        json!({
            "id": 42,
            "action": "send_text",
            "params": {
                "to": "155555500002@s.whatsapp.net",
                "message": "hello",
                "mentions": ["155555500003@s.whatsapp.net"],
                "enqueue_only": false
            }
        }),
    );
    assert_request(
        ClientRequest::SendText {
            to: "155555500002@s.whatsapp.net".into(),
            message: "hello".into(),
            reply_to: None,
            mentions: vec![],
            enqueue_only: false,
        },
        json!({
            "id": 42,
            "action": "send_text",
            "params": {
                "to": "155555500002@s.whatsapp.net",
                "message": "hello",
                "enqueue_only": false
            }
        }),
    );

    // The wire answers a star with its ordinary ack; the legacy plane's
    // `StarChanged` is an IPC event and has no wire counterpart here.
    assert_response(
        DaemonResponse::Ack,
        json!({"id": 42, "status": "ok", "type": "ack"}),
    );
}
