//! Calendar-event creation and RSVP.
//!
//! The library owns the wire: it builds the creation envelope, generates the
//! per-message secret and encrypts each response with it. This module owns the
//! addressing that has to agree with the store — which identity created the
//! event, and where the secret came from — so an RSVP against a stored card
//! uses exactly the same facts a vote against a stored poll does. Neither the
//! secret nor the library's rich creation result leaves this crate: the daemon
//! is handed only the message reference a front end can select.

use oxidezap_core::{CreatedMessageRef, EventLocation, EventRsvp};
use whatsapp_rust::wacore::proto_helpers::MessageExt as _;
use whatsapp_rust::wacore_binary::jid::{Jid, JidExt as _};
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::{EventCreationParams, EventResponseType};

use super::WhatsAppClient;
use crate::exec::Task;

/// A creation as a front end can express it. Names and optional fields stay
/// optional here for the same reason they do on the wire: a card with a title
/// alone is a valid invitation, and only a blank title is refused.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventDraft {
    pub name: String,
    pub description: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub join_link: Option<String>,
    pub location: Option<EventLocation>,
    pub is_scheduled_call: bool,
    pub extra_guests_allowed: bool,
}

impl WhatsAppClient {
    /// Create an event and return only the message that carries it.
    ///
    /// Validation is deliberately duplicated from the library's own: a blank
    /// name is a typing mistake the caller can fix before a request leaves,
    /// and the library's refusal is the backstop rather than the only answer.
    pub fn create_event(
        &self,
        to: String,
        draft: EventDraft,
    ) -> Task<Result<CreatedMessageRef, String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let chat: Jid = to.parse().map_err(|_| "not a chat address".to_string())?;
            if draft.name.trim().is_empty() {
                return Err("event name must not be empty".to_string());
            }
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };

            // `bon`'s builder changes type as fields are set, so every optional
            // field goes through its `maybe_*` setter: the chain keeps one type
            // and an absent value says the same thing as leaving the field out.
            let params = EventCreationParams::builder()
                .name(draft.name)
                .maybe_description(draft.description)
                .maybe_start_time(draft.start_time)
                .maybe_end_time(draft.end_time)
                .maybe_join_link(draft.join_link)
                .maybe_location(draft.location.map(location_to_wire))
                // An absent flag and an explicit false mean the same thing on
                // the wire; only a positive choice is worth setting.
                .maybe_is_scheduled_call(draft.is_scheduled_call.then_some(true))
                .maybe_extra_guests_allowed(draft.extra_guests_allowed.then_some(true))
                .build();

            let created = live
                .client
                .events()
                .create(&chat, params)
                .await
                .map_err(|error| error.to_string())?;
            Ok(CreatedMessageRef {
                chat_jid: to,
                message_id: created.send_result().message_id.to_string(),
            })
        })
    }

    /// Answer an event invitation stored under `event_message_id`.
    ///
    /// A response encrypts to the creation's secret and is keyed to the
    /// creator's identity, neither of which can be guessed from the card a
    /// front end holds. Both are read back from the stored creation, with the
    /// library's secret index as the fallback for compacted rows exactly as
    /// the poll-vote path does. `record_operation` is best-effort: a sent RSVP
    /// whose acknowledgement cannot be tracked is still a sent RSVP.
    pub fn respond_event(
        &self,
        chat_jid: String,
        event_message_id: String,
        response: EventRsvp,
        extra_guest_count: Option<i32>,
    ) -> Task<Result<(), String>> {
        let session = self.session.clone();
        self.exec.spawn(async move {
            let chat: Jid = chat_jid
                .parse()
                .map_err(|_| "not a chat address".to_string())?;
            if event_message_id.is_empty() {
                return Err("the event has no message id".to_string());
            }
            let Some(live) = session.lock().await.clone() else {
                return Err("no session yet".to_string());
            };
            let stored = live
                .chat_store
                .message(&chat, &event_message_id)
                .await
                .map_err(|error| format!("database query failed: {error}"))?
                .ok_or_else(|| "event message not found".to_string())?;
            let proto = stored
                .message
                .as_deref()
                .ok_or_else(|| "that message has no event content".to_string())?;
            let base = proto.get_base_message();
            if base.event_message.as_option().is_none() {
                return Err("that message is not an event".to_string());
            }
            let creator = event_creator(
                stored.from_me,
                &chat,
                &stored.sender_jid,
                live.client.pn(),
                live.client.lid(),
            )?;
            let secret = base
                .message_context_info
                .as_option()
                .and_then(|info| info.message_secret.clone());
            let secret = match secret {
                Some(secret) => secret,
                None => live
                    .chat_store
                    .poll_secret(&chat, &creator, &event_message_id)
                    .await
                    .map_err(|error| format!("database query failed: {error}"))?
                    .ok_or_else(|| "the event's secret was not stored".to_string())?,
            };
            let result = live
                .client
                .events()
                .respond_raw(
                    &chat,
                    &event_message_id,
                    &creator,
                    &secret,
                    response_wire(response),
                    extra_guest_count,
                )
                .await
                .map_err(|error| error.to_string())?;
            if let Err(error) = live
                .chat_store
                .record_operation(&chat, result.message_id.as_str())
            {
                log::warn!(
                    "event RSVP was sent but its acknowledgement could not be tracked: {error}"
                );
            }
            Ok(())
        })
    }
}

/// Which local identity the RSVP must name as creator.
///
/// Outgoing direct rows can have an empty sender, and a LID-addressed event
/// needs our LID rather than our PN — the same rule the poll vote uses. A
/// group event always names the participant the store recorded.
fn event_creator(
    from_me: bool,
    chat: &Jid,
    sender: &Jid,
    own_pn: Option<Jid>,
    own_lid: Option<Jid>,
) -> Result<Jid, String> {
    if from_me && !chat.is_group() {
        if chat.is_lid() {
            own_lid
                .or(own_pn)
                .ok_or_else(|| "account JID unavailable for event RSVP".to_string())
        } else {
            own_pn.ok_or_else(|| "account JID unavailable for event RSVP".to_string())
        }
    } else {
        Ok(sender.clone())
    }
}

fn response_wire(response: EventRsvp) -> EventResponseType {
    match response {
        EventRsvp::Going => EventResponseType::Going,
        EventRsvp::NotGoing => EventResponseType::NotGoing,
        EventRsvp::Maybe => EventResponseType::Maybe,
    }
}

fn location_to_wire(location: EventLocation) -> wa::message::LocationMessage {
    let mut wire = wa::message::LocationMessage::default();
    wire.degrees_latitude = Some(location.degrees_latitude);
    wire.degrees_longitude = Some(location.degrees_longitude);
    wire.name = location.name;
    wire
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jid(raw: &str) -> Jid {
        raw.parse().expect("synthetic JID parses")
    }

    #[test]
    fn creator_of_a_direct_event_is_our_address_in_the_chats_namespace() {
        let pn = jid("155555500001@s.whatsapp.net");
        let lid = jid("155555500001@lid");
        let peer = jid("155555500002@s.whatsapp.net");
        let direct = jid("155555500003@s.whatsapp.net");
        let group = jid("155555500001-1620000000@g.us");

        assert_eq!(
            event_creator(true, &peer, &direct, Some(pn.clone()), Some(lid.clone())).unwrap(),
            pn
        );
        assert_eq!(
            event_creator(
                true,
                &jid("155555500002@lid"),
                &direct,
                Some(pn.clone()),
                Some(lid.clone())
            )
            .unwrap(),
            lid
        );
        // No LID on this device: the PN is the fallback, not a refusal.
        assert_eq!(
            event_creator(true, &jid("155555500002@lid"), &direct, Some(pn), None).unwrap(),
            jid("155555500001@s.whatsapp.net")
        );
        assert_eq!(
            event_creator(
                false,
                &group,
                &jid("155555500003@s.whatsapp.net"),
                None,
                None
            )
            .unwrap(),
            jid("155555500003@s.whatsapp.net")
        );
    }

    #[test]
    fn every_rsvp_has_its_wire_spelling() {
        assert_eq!(response_wire(EventRsvp::Going), EventResponseType::Going);
        assert_eq!(
            response_wire(EventRsvp::NotGoing),
            EventResponseType::NotGoing
        );
        assert_eq!(response_wire(EventRsvp::Maybe), EventResponseType::Maybe);
    }

    #[test]
    fn a_location_keeps_its_pin_and_label() {
        let wire = location_to_wire(EventLocation {
            degrees_latitude: -12.5,
            degrees_longitude: 40.25,
            name: Some("Room 1".into()),
        });
        assert_eq!(wire.degrees_latitude, Some(-12.5));
        assert_eq!(wire.degrees_longitude, Some(40.25));
        assert_eq!(wire.name.as_deref(), Some("Room 1"));
    }
}
