//! The call history the wire reports.
//!
//! The store keeps no call log — calls are live state, not rows — so the
//! history is what this daemon has seen since it started: finalized calls
//! with their outcome and duration, newest last, bounded. Anything still
//! ringing is not history yet; the live stage answers for it through the
//! usual call queries.

use std::collections::{HashMap, VecDeque};

use wacore_binary::jid::{Jid, JidExt as _};

use oxidezap_core::UiEvent;
use oxidezap_wire::dto::{CallEventDto, CallParticipantDto};

use crate::state::StateHub;

/// How many finalized calls the ring keeps. A log, not an archive: the
/// CLI pages it, and anything older than this is gone with the process.
const RING_CAPACITY: usize = 100;

/// A call in flight, waiting for its outcome.
struct InterimCall {
    chat_jid: String,
    caller_jid: String,
    is_video: bool,
    is_group: bool,
    group_jid: Option<String>,
    participants: Vec<CallParticipantDto>,
    group_transaction_id: Option<u32>,
    group_media: Option<String>,
    incoming: bool,
    started_ms: u64,
    connected_ms: Option<u64>,
}

/// Finalized calls with their outcome, newest last.
pub(crate) struct CallLog {
    interim: HashMap<String, InterimCall>,
    entries: VecDeque<CallEventDto>,
}

impl CallLog {
    pub(crate) fn new() -> Self {
        Self {
            interim: HashMap::new(),
            entries: VecDeque::new(),
        }
    }

    /// Fold one session event into the log. Runs before the hub folds the
    /// same event: what is recorded here is what happened, and the hub's
    /// post-state would only say what remains.
    pub(crate) fn observe(&mut self, event: &UiEvent, hub: &StateHub) {
        match event {
            UiEvent::IncomingCall(call) => {
                // A group offer arrives from a person, not from the group.
                // Filing it under their chat would show a group call in a 1:1
                // conversation, so the group is the chat and the person stays
                // the caller. Both answers are on the event now.
                let chat_jid = call
                    .group_jid
                    .clone()
                    .unwrap_or_else(|| call.caller_jid.clone());
                let is_group = call.group_jid.is_some() || is_group_jid(&call.caller_jid);
                let group = call.group.as_ref();
                let participants: Vec<CallParticipantDto> = group
                    .map(|group| {
                        group
                            .participants
                            .iter()
                            .map(|p| CallParticipantDto {
                                jid: p.jid.clone(),
                                name: p.name.clone(),
                                state: p.state.clone(),
                                participant_type: p.participant_type.clone(),
                                error: p.error,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(existing) = self.interim.get_mut(&call.call_id) {
                    // A roster refresh is an `IncomingCall` event under an id
                    // the log already holds. It must not turn an outgoing
                    // call into a missed one or restart its clock.
                    if let Some(group) = group {
                        // Name resolution can let an older snapshot finish
                        // last. Keep the newest transaction in the history
                        // too, not just in the live state.
                        let stale = match (existing.group_transaction_id, group.transaction_id) {
                            (Some(current), Some(new)) => new < current,
                            (Some(_), None) => true,
                            _ => false,
                        };
                        if !stale {
                            existing.group_jid = Some(group.group_jid.clone());
                            existing.participants = participants;
                            existing.group_transaction_id = group.transaction_id;
                            existing.group_media = group.media.clone();
                            existing.is_group = true;
                        }
                    }
                    return;
                }
                self.interim.insert(
                    call.call_id.clone(),
                    InterimCall {
                        chat_jid,
                        caller_jid: call.caller_jid.clone(),
                        is_video: call.is_video,
                        is_group,
                        group_jid: call.group_jid.clone(),
                        participants,
                        group_transaction_id: group.and_then(|g| g.transaction_id),
                        group_media: group.and_then(|g| g.media.clone()),
                        incoming: true,
                        started_ms: now_ms(),
                        connected_ms: None,
                    },
                );
            }
            UiEvent::OutgoingCallStarted {
                call_id,
                recipient_jid,
                is_video,
                ..
            } => {
                let caller_jid = own_jid(hub).unwrap_or_default();
                self.interim.insert(
                    call_id.clone(),
                    InterimCall {
                        chat_jid: recipient_jid.clone(),
                        caller_jid,
                        is_video: *is_video,
                        is_group: is_group_jid(recipient_jid),
                        group_jid: is_group_jid(recipient_jid).then(|| recipient_jid.clone()),
                        participants: Vec::new(),
                        group_transaction_id: None,
                        group_media: None,
                        incoming: false,
                        started_ms: now_ms(),
                        connected_ms: None,
                    },
                );
            }
            UiEvent::CallAccepted(call_id) => {
                if let Some(call) = self.interim.get_mut(call_id) {
                    call.connected_ms.get_or_insert(now_ms());
                }
            }
            UiEvent::CallAnswered { call_id, is_video } => {
                if let Some(call) = self.interim.get_mut(call_id) {
                    call.connected_ms.get_or_insert(now_ms());
                    call.is_video = *is_video;
                }
            }
            UiEvent::CallEnded(call_id) => {
                if let Some(call) = self.interim.remove(call_id) {
                    self.push(finalize(call_id, call));
                }
            }
            // Answered or refused on another device, or never a call at
            // all: the event itself says there is nothing honest to write.
            UiEvent::CallEndedElsewhere(call_id) | UiEvent::CallUnrecorded(call_id) => {
                self.interim.remove(call_id);
            }
            UiEvent::OutgoingCallFailed { recipient_jid, .. } => {
                let key = self
                    .interim
                    .iter()
                    .find(|(_, call)| !call.incoming && &call.chat_jid == recipient_jid)
                    .map(|(id, _)| id.clone());
                if let Some(key) = key
                    && let Some(call) = self.interim.remove(&key)
                {
                    self.push(CallEventDto {
                        id: key,
                        outcome: "cancelled".to_string(),
                        ..dto_of(call, None)
                    });
                }
            }
            _ => {}
        }
    }

    /// The finalized calls, newest first, bounded by the ask.
    pub(crate) fn list(&self, limit: usize) -> Vec<CallEventDto> {
        self.entries
            .iter()
            .rev()
            .take(limit.max(1))
            .cloned()
            .collect()
    }

    fn push(&mut self, entry: CallEventDto) {
        self.entries.push_back(entry);
        while self.entries.len() > RING_CAPACITY {
            self.entries.pop_front();
        }
    }
}

/// An ended call as the protocol's call event.
fn finalize(id: &str, call: InterimCall) -> CallEventDto {
    let (outcome, duration) = match (call.incoming, call.connected_ms) {
        (_, Some(connected)) => {
            let elapsed = now_ms().saturating_sub(connected) / 1_000;
            ("connected", Some(elapsed.min(u64::from(u32::MAX)) as u32))
        }
        (true, None) => ("missed", None),
        (false, None) => ("cancelled", None),
    };
    CallEventDto {
        id: id.to_string(),
        outcome: outcome.to_string(),
        ..dto_of(call, duration)
    }
}

fn dto_of(call: InterimCall, duration_seconds: Option<u32>) -> CallEventDto {
    CallEventDto {
        id: String::new(),
        chat_jid: call.chat_jid,
        caller_jid: call.caller_jid,
        timestamp_ms: call.started_ms as i64,
        is_video: call.is_video,
        is_group: call.is_group,
        group_jid: call.group_jid,
        participants: call.participants,
        group_transaction_id: call.group_transaction_id,
        group_media: call.group_media,
        duration_seconds,
        outcome: String::new(),
    }
}

fn is_group_jid(jid: &str) -> bool {
    jid.parse::<Jid>().is_ok_and(|j| j.is_group())
}

/// This account's own JID, for the caller side of outgoing calls.
fn own_jid(hub: &StateHub) -> Option<String> {
    hub.snapshot().account?.jid.clone()
}

fn now_ms() -> u64 {
    wacore::time::now_millis().max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidezap_core::IncomingCall;

    fn incoming(id: &str) -> UiEvent {
        UiEvent::IncomingCall(IncomingCall {
            call_id: id.to_string(),
            caller_name: "Maria".to_string(),
            caller_jid: "559900000001@s.whatsapp.net".to_string(),
            group_jid: None,
            group: None,
            is_video: false,
            is_offline: false,
            received_at: wacore::time::now_utc(),
        })
    }

    /// A missed call lands in the log with its outcome, newest first.
    #[test]
    fn a_missed_call_is_logged() {
        let hub = StateHub::new();
        let mut log = CallLog::new();
        log.observe(&incoming("c1"), &hub);
        assert!(log.list(10).is_empty(), "ringing is not history yet");
        log.observe(&UiEvent::CallEnded("c1".to_string()), &hub);
        let entries = log.list(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome, "missed");
        assert_eq!(entries[0].caller_jid, "559900000001@s.whatsapp.net");
        assert_eq!(entries[0].duration_seconds, None);
    }

    /// A group offer names the person who started it, but the group is the
    /// conversation it belongs to. Both halves have to reach the log or a
    /// group call turns up in somebody's 1:1 history.
    #[test]
    fn a_group_call_is_logged_under_the_group() {
        let hub = StateHub::new();
        let mut log = CallLog::new();
        log.observe(
            &UiEvent::IncomingCall(IncomingCall {
                call_id: "group-call".to_string(),
                caller_name: "Maria".to_string(),
                caller_jid: "559900000001@s.whatsapp.net".to_string(),
                group_jid: Some("120363000000000001@g.us".to_string()),
                group: None,
                is_video: false,
                is_offline: false,
                received_at: wacore::time::now_utc(),
            }),
            &hub,
        );
        log.observe(&UiEvent::CallEnded("group-call".to_string()), &hub);
        let entries = log.list(10);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_group);
        assert_eq!(entries[0].chat_jid, "120363000000000001@g.us");
        assert_eq!(entries[0].caller_jid, "559900000001@s.whatsapp.net");
    }

    /// A group roster is published under the id of a call the daemon already
    /// knows. It must add the roster, not replace an outgoing call with a
    /// second ringing card.
    #[test]
    fn a_group_roster_refresh_keeps_an_outgoing_call_outgoing() {
        let hub = StateHub::new();
        let mut log = CallLog::new();
        log.observe(
            &UiEvent::OutgoingCallStarted {
                call_id: "group-out".to_string(),
                recipient_jid: "120363000000000001@g.us".to_string(),
                placeholder_id: "p".to_string(),
                is_video: false,
            },
            &hub,
        );
        let refresh = IncomingCall {
            call_id: "group-out".to_string(),
            caller_name: "Ada".to_string(),
            caller_jid: "550000000001@s.whatsapp.net".to_string(),
            group_jid: Some("120363000000000001@g.us".to_string()),
            group: None,
            is_video: false,
            is_offline: false,
            received_at: wacore::time::now_utc(),
        }
        .with_group_roster(
            "120363000000000001@g.us",
            vec![(
                "550000000002@s.whatsapp.net".to_string(),
                Some("Grace".to_string()),
                Some("connected".to_string()),
                None,
                None,
            )],
            Some(3),
            Some("audio".to_string()),
        );
        log.observe(&UiEvent::IncomingCall(refresh), &hub);
        log.observe(&UiEvent::CallEnded("group-out".to_string()), &hub);

        let entries = log.list(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome, "cancelled");
        assert_eq!(entries[0].group_transaction_id, Some(3));
        assert_eq!(entries[0].participants.len(), 1);
        assert_eq!(entries[0].participants[0].name.as_deref(), Some("Grace"));
    }

    /// An answered call is connected, with a duration.
    #[test]
    fn an_answered_call_is_connected() {
        let hub = StateHub::new();
        let mut log = CallLog::new();
        log.observe(
            &UiEvent::OutgoingCallStarted {
                call_id: "c2".to_string(),
                recipient_jid: "559900000002@s.whatsapp.net".to_string(),
                placeholder_id: "p2".to_string(),
                is_video: true,
            },
            &hub,
        );
        log.observe(&UiEvent::CallAccepted("c2".to_string()), &hub);
        log.observe(&UiEvent::CallEnded("c2".to_string()), &hub);
        let entries = log.list(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].outcome, "connected");
        assert!(entries[0].is_video);
        assert_eq!(entries[0].duration_seconds, Some(0));
    }

    /// Calls taken elsewhere leave no entry: this device has nothing true
    /// to write down.
    #[test]
    fn elsewhere_and_unrecorded_calls_leave_no_entry() {
        let hub = StateHub::new();
        let mut log = CallLog::new();
        log.observe(&incoming("c3"), &hub);
        log.observe(&UiEvent::CallEndedElsewhere("c3".to_string()), &hub);
        log.observe(&incoming("c4"), &hub);
        log.observe(&UiEvent::CallUnrecorded("c4".to_string()), &hub);
        assert!(log.list(10).is_empty());
    }

    /// The ring is bounded and newest-first.
    #[test]
    fn the_ring_is_bounded_and_newest_first() {
        let hub = StateHub::new();
        let mut log = CallLog::new();
        for n in 0..(RING_CAPACITY + 5) {
            let id = format!("c{n}");
            log.observe(&incoming(&id), &hub);
            log.observe(&UiEvent::CallEnded(id.clone()), &hub);
        }
        let entries = log.list(1000);
        assert_eq!(entries.len(), RING_CAPACITY);
        assert_eq!(entries[0].id, format!("c{}", RING_CAPACITY + 4));
    }
}
