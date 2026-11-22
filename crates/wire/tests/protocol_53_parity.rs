//! Serde parity for protocol 53's pin and saved-message listing.
//!
//! As in protocol 52, the contract is the JSON the two sides agree on: IPC
//! tags its frames at the top level (`"request"` / `"type"`) while this crate
//! nests the same payload fields under `action`/`params` and `type`/`data`.
//! The literals below name the wire spelling so a field renamed on either side
//! is visible here. Every address and id is synthetic.

use oxidezap_wire::{ClientRequest, DaemonResponse, RequestEnvelope, ResponseEnvelope};
use serde_json::{Value, json};

fn assert_request(request: ClientRequest, expected: Value) {
    let envelope = RequestEnvelope { id: 42, request };
    let frame = serde_json::to_value(&envelope).expect("a request serializes");
    assert_eq!(frame, expected, "a request changed shape on the wire");
    let decoded: RequestEnvelope = serde_json::from_value(frame).expect("a request deserializes");
    assert_eq!(decoded, envelope, "a request did not round-trip");
}

#[test]
fn protocol_53_pin_and_starred_listing_keep_their_wire_shape() {
    // Pinning mirrors a star: the same addressing, a boolean, and an ordinary
    // ack. No duration travels — the daemon promises the library's longest.
    assert_request(
        ClientRequest::SetMessagePinned {
            chat_jid: "155555500001@g.us".into(),
            message_id: "SYNTHETIC-1".into(),
            pinned: true,
        },
        json!({
            "id": 42,
            "action": "set_message_pinned",
            "params": {
                "chat_jid": "155555500001@g.us",
                "message_id": "SYNTHETIC-1",
                "pinned": true
            }
        }),
    );
    // The starred listing was already scriptable; this pins its wire spelling
    // too, now that the window has a panel that depends on it.
    assert_request(
        ClientRequest::ListStarredMessages { limit: 50 },
        json!({
            "id": 42,
            "action": "list_starred_messages",
            "params": {"limit": 50}
        }),
    );

    // Both mutations answer with the ordinary ack. `PinChanged` and
    // `StarredResults` are IPC events and have no wire counterpart here.
    let envelope = ResponseEnvelope::success(42, DaemonResponse::Ack);
    let frame = serde_json::to_value(&envelope).expect("a response serializes");
    assert_eq!(frame, json!({"id": 42, "status": "ok", "type": "ack"}));
}
