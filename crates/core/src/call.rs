//! Call state structures for the UI.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wacore::types::call::{CallAction, IncomingCall as WaIncomingCall};

/// Call ids are plain strings on the wire (and in the voip facade).
pub type CallId = String;

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
        }
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
            is_video,
            is_offline: offer.offline,
            received_at: offer.timestamp.unwrap_or_else(wacore::time::now_utc),
        }
    }

    pub fn initial(&self) -> char {
        self.caller_name.chars().next().unwrap_or('?')
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
    }
}
