//! Calendar invitation carried on a message.
//!
//! Facts only: the times are unix seconds, the coordinates are degrees, and
//! there is deliberately no formatted range or other display text here — the
//! front end owns wording and translation.
//!
//! Coordinates are protocol floats, so `PartialEq` is derived rather than
//! `Eq`. The explicit `Eq` markers below keep `ChatMessage` (and the wire enums
//! carrying it) `Eq` without pushing a loss of `Eq` through every crate above
//! this one. Protocol coordinates are finite; a NaN is not a location.

use serde::{Deserialize, Serialize};

/// A field whose absence means what its default does. Skipping it keeps an
/// ordinary card — which has neither a place nor an override flag — small.
fn is_false(value: &bool) -> bool {
    !*value
}

/// Where an event happens. The name is optional: a pin without a label is
/// still a pin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventLocation {
    pub degrees_latitude: f64,
    pub degrees_longitude: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Eq for EventLocation {}

/// A calendar card attached to a message.
///
/// Every field but `name` has a default, so an older frame that only carried
/// a title still decodes into an uneventful card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventCard {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EventLocation>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_canceled: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_scheduled_call: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub extra_guests_allowed: bool,
}

impl Eq for EventCard {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_card_round_trips_over_the_socket() {
        let card = EventCard {
            name: "Synthetic standup".into(),
            description: Some("A fixture, not an invitation".into()),
            start_time: Some(1_700_000_000),
            end_time: Some(1_700_000_600),
            join_link: Some("https://example.invalid/call".into()),
            location: Some(EventLocation {
                degrees_latitude: -12.5,
                degrees_longitude: 40.25,
                name: Some("Room 1".into()),
            }),
            is_canceled: false,
            is_scheduled_call: true,
            extra_guests_allowed: true,
        };
        let encoded = serde_json::to_value(&card).unwrap();
        assert_eq!(serde_json::from_value::<EventCard>(encoded).unwrap(), card);
    }

    #[test]
    fn an_old_frame_defaults_to_a_title_only() {
        let card: EventCard = serde_json::from_str(r#"{"name":"Synthetic standup"}"#).unwrap();
        assert_eq!(card.name, "Synthetic standup");
        assert_eq!(card.description, None);
        assert_eq!(card.start_time, None);
        assert_eq!(card.end_time, None);
        assert_eq!(card.join_link, None);
        assert_eq!(card.location, None);
        assert!(!card.is_canceled);
        assert!(!card.is_scheduled_call);
        assert!(!card.extra_guests_allowed);

        let location: EventLocation =
            serde_json::from_str(r#"{"degrees_latitude":0.0,"degrees_longitude":1.0}"#).unwrap();
        assert_eq!(location.name, None);
    }

    #[test]
    fn a_message_carries_a_card_and_old_frames_have_none() {
        let mut message = crate::ChatMessage::new_incoming(
            "SYNTHETIC-1".into(),
            "155555500001@lid".into(),
            String::new(),
        );
        message.event = Some(EventCard {
            name: "Synthetic standup".into(),
            description: None,
            start_time: Some(1_700_000_000),
            end_time: None,
            join_link: None,
            location: None,
            is_canceled: false,
            is_scheduled_call: false,
            extra_guests_allowed: false,
        });
        let encoded = serde_json::to_value(&message).unwrap();
        assert_eq!(
            serde_json::from_value::<crate::ChatMessage>(encoded.clone()).unwrap(),
            message
        );
        let mut old = encoded;
        old.as_object_mut().unwrap().remove("event");
        assert!(
            serde_json::from_value::<crate::ChatMessage>(old)
                .unwrap()
                .event
                .is_none()
        );

        message.mark_revoked();
        assert!(message.event.is_none(), "a tombstone keeps no invitation");
    }

    #[test]
    fn rsvp_and_created_ref_have_the_wire_spelling() {
        assert_eq!(
            serde_json::to_string(&EventRsvp::Going).unwrap(),
            "\"going\""
        );
        assert_eq!(
            serde_json::to_string(&EventRsvp::NotGoing).unwrap(),
            "\"not_going\""
        );
        assert_eq!(
            serde_json::to_string(&EventRsvp::Maybe).unwrap(),
            "\"maybe\""
        );
        assert_eq!(
            serde_json::to_string(&CreatedMessageRef {
                chat_jid: "155555500001@g.us".into(),
                message_id: "SYNTHETIC-1".into(),
            })
            .unwrap(),
            r#"{"chat_jid":"155555500001@g.us","message_id":"SYNTHETIC-1"}"#
        );
    }
}
