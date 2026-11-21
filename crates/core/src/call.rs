//! Call state structures for the UI.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wacore::types::call::{CallAction, IncomingCall as WaIncomingCall};

/// Call ids are plain strings on the wire (and in the voip facade).
pub type CallId = String;

/// One group participant as plain data, in the shape the session and front
/// ends exchange: JID, resolved name, connection state, participant type, error.
pub type CallParticipantRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<u32>,
);

/// The borrowed view of [`CallParticipantRow`] returned when reading a roster.
pub type CallParticipantRef<'a> = (
    &'a str,
    Option<&'a str>,
    Option<&'a str>,
    Option<&'a str>,
    Option<u32>,
);

/// Opaque label used as the recipient of a call-link placement.
///
/// This is NOT a JID: it has no `@` and must never be parsed as one. A call
/// link is identified by its token, not by an address, and treating this label
/// as a JID would send it down the group or user address paths.
pub const CALL_LINK_RECIPIENT_JID: &str = "call-link";

/// Display name for a call-link placement in the call log and outgoing card.
pub const CALL_LINK_RECIPIENT_NAME: &str = "Call link";

/// Which media a call link offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallLinkMedia {
    Audio,
    Video,
}

/// The metadata a link resolves to, before the call is joined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallLinkPreview {
    /// The link identity. Never log this or show it in the UI: it is the
    /// invite itself, and printing it shares the call with whoever can read
    /// the log or see the screen.
    pub token: String,
    pub media: CallLinkMedia,
    pub creator_jid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_pn: Option<String>,
    pub waiting_room_enabled: bool,
    pub is_admin: bool,
}

/// One person in a group call's authoritative roster.
///
/// The library's roster names a participant by JID and, for some snapshots,
/// by connection state. A display name is deliberately optional: it is
/// resolved on the session side from the same address book every other
/// surface uses, and a roster that arrives before the lookup has finished is
/// still worth drawing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallParticipant {
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

/// The authoritative group identity and roster of one call.
///
/// A group offer carries its group JID but not its roster; the roster arrives
/// later in transaction-ordered snapshots owned by the library. Both are kept
/// together here so a front end attaching mid-call gets one answer rather
/// than a group id and a stale participant list that have to be joined up by
/// convention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallGroup {
    pub group_jid: String,
    #[serde(default)]
    pub participants: Vec<CallParticipant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<u32>,
    /// The roster's own media mode (`audio` or `video`), when the snapshot
    /// supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
}

impl CallGroup {
    pub fn new(group_jid: impl Into<String>) -> Self {
        Self {
            group_jid: group_jid.into(),
            participants: Vec::new(),
            transaction_id: None,
            media: None,
        }
    }

    /// The roster as the plain tuples the session and front ends exchange.
    ///
    /// Keeping the tuple shape on the public constructor means a caller does
    /// not have to name this type through the crate's re-export list.
    #[must_use]
    pub fn from_roster(
        group_jid: impl Into<String>,
        participants: Vec<CallParticipantRow>,
        transaction_id: Option<u32>,
        media: Option<String>,
    ) -> Self {
        Self {
            group_jid: group_jid.into(),
            participants: participants
                .into_iter()
                .map(
                    |(jid, name, state, participant_type, error)| CallParticipant {
                        jid,
                        name,
                        state,
                        participant_type,
                        error,
                    },
                )
                .collect(),
            transaction_id,
            media,
        }
    }

    /// The roster in the tuple shape used by [`Self::from_roster`].
    pub fn roster(&self) -> Vec<CallParticipantRef<'_>> {
        self.participants
            .iter()
            .map(|p| {
                (
                    p.jid.as_str(),
                    p.name.as_deref(),
                    p.state.as_deref(),
                    p.participant_type.as_deref(),
                    p.error,
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutgoingCallState {
    Initiating,
    Ringing,
    Connected,
    Declined,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingCall {
    pub call_id: CallId,
    pub recipient_name: String,
    pub recipient_jid: String,
    pub is_video: bool,
    pub state: OutgoingCallState,
    pub initiated_at: DateTime<Utc>,
    /// Group identity and roster, for a call bound to a group.
    ///
    /// Additive and defaulted on the wire: a 1:1 call leaves it out, and a
    /// frame from an older daemon still reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<CallGroup>,
}

impl OutgoingCall {
    pub fn new(
        call_id: impl Into<CallId>,
        recipient_jid: String,
        recipient_name: String,
        is_video: bool,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            recipient_name,
            recipient_jid,
            is_video,
            state: OutgoingCallState::Initiating,
            initiated_at: wacore::time::now_utc(),
            group: None,
        }
    }

    /// Bind this placement to a group before it goes out.
    #[must_use]
    pub fn for_group(mut self, group: CallGroup) -> Self {
        self.group = Some(group);
        self
    }

    /// Bind this placement to a group using the tuple roster shape.
    #[must_use]
    pub fn for_group_roster(
        self,
        group_jid: impl Into<String>,
        participants: Vec<CallParticipantRow>,
        transaction_id: Option<u32>,
        media: Option<String>,
    ) -> Self {
        self.for_group(CallGroup::from_roster(
            group_jid,
            participants,
            transaction_id,
            media,
        ))
    }

    pub fn initial(&self) -> char {
        self.recipient_name.chars().next().unwrap_or('?')
    }

    pub fn set_state(&mut self, state: OutgoingCallState) {
        self.state = state;
    }

    pub fn is_active(&self) -> bool {
        !matches!(
            self.state,
            OutgoingCallState::Declined | OutgoingCallState::Timeout
        )
    }

    pub fn status_message(&self) -> &'static str {
        match self.state {
            OutgoingCallState::Initiating => "Calling...",
            OutgoingCallState::Ringing => "Ringing...",
            OutgoingCallState::Connected => "Connected",
            OutgoingCallState::Declined => "Call declined",
            OutgoingCallState::Timeout => "No answer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncomingCall {
    pub call_id: CallId,
    pub caller_name: String,
    /// Who started the call. For a group call this is the person, not the
    /// group — see [`group_jid`](Self::group_jid).
    pub caller_jid: String,
    /// The group this call belongs to, when the offer says it is one.
    ///
    /// Without this a group call is indistinguishable from a 1:1 call from
    /// whoever started it, and the call log files it under their chat. Kept
    /// separate from `caller_jid` so both answers remain available. Defaulted
    /// on the wire so a frame from an older daemon still reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_jid: Option<String>,
    /// Authoritative group roster, once one has arrived.
    ///
    /// Group control signals reach the session after the library has applied
    /// them. They are republished on this same event with the same call id;
    /// [`CallState::set_incoming`](crate::CallState::set_incoming) recognises
    /// the redelivery and refreshes this field rather than ringing again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<CallGroup>,
    pub is_video: bool,
    pub is_offline: bool,
    pub received_at: DateTime<Utc>,
}

impl IncomingCall {
    /// Build a ringing call from the library's offer.
    ///
    /// The offer itself does not come along: whoever accepts or declines looks
    /// it up by call id in the session's own registry, so a copy here would be
    /// a second owner of a payload only one process can act on — and one that
    /// could not cross the daemon socket if it tried.
    pub fn new(
        call_id: impl Into<CallId>,
        caller_name: String,
        caller_jid: String,
        is_video: bool,
        offer: &WaIncomingCall,
    ) -> Self {
        // The offer is the only place this lives. Reading it here rather than
        // asking every caller to pass it separately means nobody can forget —
        // a group call that reaches the UI without its group is the bug this
        // field exists to prevent.
        let group_jid = match &offer.action {
            CallAction::Offer { group_jid, .. } => {
                group_jid.as_ref().map(|jid| jid.to_non_ad_string())
            }
            _ => None,
        };
        Self {
            call_id: call_id.into(),
            caller_name,
            caller_jid,
            group_jid,
            group: None,
            is_video,
            is_offline: offer.offline,
            received_at: offer.timestamp.unwrap_or_else(wacore::time::now_utc),
        }
    }

    pub fn initial(&self) -> char {
        self.caller_name.chars().next().unwrap_or('?')
    }

    /// Attach the roster learned after the offer, for a refresh event.
    #[must_use]
    pub fn with_group(mut self, group: CallGroup) -> Self {
        self.group_jid = Some(group.group_jid.clone());
        self.group = Some(group);
        self
    }

    /// Attach a roster using the plain tuple shape.
    #[must_use]
    pub fn with_group_roster(
        self,
        group_jid: impl Into<String>,
        participants: Vec<CallParticipantRow>,
        transaction_id: Option<u32>,
        media: Option<String>,
    ) -> Self {
        self.with_group(CallGroup::from_roster(
            group_jid,
            participants,
            transaction_id,
            media,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wacore_binary::jid::Jid;

    fn offer(call_id: &str, group_jid: Option<Jid>) -> WaIncomingCall {
        let from: Jid = "a@s.whatsapp.net".parse().expect("valid jid");
        WaIncomingCall::builder()
            .from(from.clone())
            .stanza_id(call_id.to_string())
            .timestamp(wacore::time::now_utc())
            .offline(false)
            .action(CallAction::Offer {
                call_id: call_id.to_string(),
                call_creator: from,
                caller_pn: None,
                caller_country_code: None,
                device_class: None,
                joinable: true,
                is_video: false,
                audio: Vec::new(),
                group_jid,
            })
            .build()
    }

    #[test]
    fn the_group_comes_from_the_offer_not_the_caller() {
        let group: Jid = "120363000000000001@g.us".parse().expect("valid jid");
        let call = IncomingCall::new(
            "c1",
            "Maria".into(),
            "a@s.whatsapp.net".into(),
            false,
            &offer("c1", Some(group)),
        );
        assert_eq!(call.caller_jid, "a@s.whatsapp.net");
        assert_eq!(call.group_jid.as_deref(), Some("120363000000000001@g.us"));
    }

    #[test]
    fn a_direct_offer_has_no_group() {
        let call = IncomingCall::new(
            "c2",
            "Maria".into(),
            "a@s.whatsapp.net".into(),
            false,
            &offer("c2", None),
        );
        assert_eq!(call.group_jid, None);
    }

    #[test]
    fn a_missing_group_field_reads_as_direct() {
        let json = r#"{
            "call_id":"c3","caller_name":"Maria",
            "caller_jid":"a@s.whatsapp.net","is_video":false,
            "is_offline":false,"received_at":"2026-11-20T00:00:00Z"
        }"#;
        let call: IncomingCall = serde_json::from_str(json).expect("old frame still reads");
        assert_eq!(call.group_jid, None);
        assert_eq!(call.group, None);
    }

    #[test]
    fn a_roster_refresh_carries_the_group_it_names() {
        let group = CallGroup {
            group_jid: "120363000000000001@g.us".to_string(),
            participants: vec![CallParticipant {
                jid: "member@example.invalid".to_string(),
                name: Some("Member".to_string()),
                state: Some("connected".to_string()),
                participant_type: None,
                error: None,
            }],
            transaction_id: Some(7),
            media: Some("audio".to_string()),
        };
        let call = IncomingCall::new(
            "c4",
            "Maria".into(),
            "a@s.whatsapp.net".into(),
            false,
            &offer("c4", None),
        )
        .with_group(group.clone());

        assert_eq!(call.group_jid.as_deref(), Some(group.group_jid.as_str()));
        assert_eq!(call.group, Some(group));
    }
}
