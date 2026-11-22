//! The Calls pane's small view model.
//!
//! The daemon owns the only call history this process can see: a bounded ring
//! of finalized calls from its own lifetime. This module keeps the last answer
//! a window received, sorts it for display, and turns each daemon DTO into the
//! labels a row draws. Nothing here invents a call or reaches an account.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::DateTime;
use gpui::{Context, Window};
use oxidezap_core::Chat;
use oxidezap_ipc::wire::dto::CallEventDto;

use super::WhatsAppApp;

/// The daemon cap, repeated here so a window never asks for more than exists.
pub(super) const CALL_LIMIT: usize = 100;

/// Which part of the daemon's log this pane is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CallFilter {
    #[default]
    All,
    Missed,
}

impl CallFilter {
    pub const ALL: [Self; 2] = [Self::All, Self::Missed];

    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Missed => "missed",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => crate::tr!(AllCalls),
            Self::Missed => crate::tr!(MissedCalls),
        }
    }
}

/// The last answer, plus the state needed to show it honestly.
#[derive(Debug, Default)]
pub(crate) struct CallLogState {
    rows: Vec<CallEventDto>,
    loading: bool,
    last_error: Option<String>,
    filter: CallFilter,
}

impl CallLogState {
    /// Replace the rows after a successful answer. Sort here too, so every
    /// reader sees one order even before the view model is rebuilt.
    pub(crate) fn adopt(&mut self, mut calls: Vec<CallEventDto>) {
        calls.sort_unstable_by_key(|call| std::cmp::Reverse(call.timestamp_ms));
        calls.truncate(CALL_LIMIT);
        self.rows = calls;
        self.loading = false;
        self.last_error = None;
    }

    pub(crate) fn begin_load(&mut self) {
        self.loading = true;
        self.last_error = None;
    }

    pub(crate) fn fail(&mut self, detail: impl Into<String>) {
        self.loading = false;
        self.last_error = Some(detail.into());
    }

    pub(crate) fn set_filter(&mut self, filter: CallFilter) {
        self.filter = filter;
    }

    pub(crate) fn rows(&self) -> &[CallEventDto] {
        &self.rows
    }

    pub(crate) fn filter(&self) -> CallFilter {
        self.filter
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.loading
    }

    pub(crate) fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Forget everything tied to the account that just went away.
    pub(crate) fn clear(&mut self) {
        self.rows.clear();
        self.loading = false;
        self.last_error = None;
        self.filter = CallFilter::default();
    }
}

impl WhatsAppApp {
    /// The last answer's rows as this window can label them.
    pub(crate) fn call_rows(&self) -> Vec<CallRow> {
        let own = [
            self.account_jid.as_deref().unwrap_or(""),
            self.account_lid.as_deref().unwrap_or(""),
        ];
        call_rows(
            self.call_log.rows(),
            &self.chats,
            &own,
            self.call_log.filter(),
        )
    }

    pub(crate) fn call_log_state(&self) -> &CallLogState {
        &self.call_log
    }

    pub(crate) fn set_call_filter(&mut self, filter: CallFilter, cx: &mut Context<Self>) {
        if self.call_log.filter() != filter {
            self.call_log.set_filter(filter);
            cx.notify();
        }
    }

    /// Open the chat behind a call row, when this window actually has it.
    /// A log entry whose chat has not been loaded stays a label, not a dead
    /// click that pretends to navigate somewhere.
    pub(crate) fn open_call_chat(
        &mut self,
        jid: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.chat_named(&jid).is_some() {
            self.select_chat(jid, super::ChatOpen::ToCompose, window, cx);
        }
    }

    /// Adopt a fresh daemon answer. One query owns one snapshot: it replaces
    /// the rows instead of merging with an older ring that may have dropped
    /// entries the daemon no longer has.
    pub(crate) fn adopt_call_log(&mut self, calls: Vec<CallEventDto>, cx: &mut Context<Self>) {
        self.call_log.adopt(calls);
        cx.notify();
    }

    /// Ask the daemon for the finalized calls it still holds.
    ///
    /// The answer is addressed to this window, so it lands in a oneshot rather
    /// than the event stream. Keeping the previous rows while the new answer
    /// is in flight makes a refresh a change of data, not a blank pane.
    pub(crate) fn refresh_calls(&mut self, cx: &mut Context<Self>) {
        let Some(waiting) = self
            .client
            .as_ref()
            .map(|client| client.list_calls(CALL_LIMIT))
        else {
            self.call_log
                .fail(crate::tr!(DaemonDisconnectedBeforeAnswer));
            cx.notify();
            return;
        };
        self.call_log.begin_load();
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let outcome = match waiting.await {
                Ok(Ok(calls)) => Ok(calls),
                Ok(Err(failure)) => Err(failure.detail),
                Err(_) => Err(crate::tr!(DaemonDisconnectedBeforeAnswer).to_string()),
            };
            let _ = entity.update(cx, |app, cx| match outcome {
                Ok(calls) => app.adopt_call_log(calls, cx),
                Err(detail) => {
                    app.call_log.fail(detail);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// One row as the pane draws it. All display strings are resolved before the
/// render helper is called, so the helper holds no borrow of the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallRow {
    pub id: String,
    /// The chat a tap would open. Empty when this window has no such chat.
    pub jid: String,
    pub name: String,
    pub direction: &'static str,
    pub kind: &'static str,
    pub outcome: String,
    pub duration: Option<String>,
    pub time: String,
    pub can_open_chat: bool,
}

/// Build the visible rows, newest first.
///
/// Sorting and filtering happen here rather than in the daemon answer so a
/// filter change costs no IPC. Duplicate call ids collapse: a roster refresh
/// can revisit a call, and the same call drawn twice reads as a second call.
pub(crate) fn call_rows(
    calls: &[CallEventDto],
    chats: &[Arc<Chat>],
    own_jids: &[&str],
    filter: CallFilter,
) -> Vec<CallRow> {
    let mut ordered: Vec<&CallEventDto> = calls.iter().collect();
    ordered.sort_unstable_by_key(|call| std::cmp::Reverse(call.timestamp_ms));

    let mut seen = HashSet::new();
    ordered
        .into_iter()
        .filter(|call| !call.id.is_empty() && seen.insert(call.id.as_str()))
        .filter(|call| match filter {
            CallFilter::All => true,
            CallFilter::Missed => call.outcome == "missed",
        })
        .map(|call| row_of(call, chats, own_jids))
        .collect()
}

fn row_of(call: &CallEventDto, chats: &[Arc<Chat>], own_jids: &[&str]) -> CallRow {
    // A group call is filed under the group even though the offer came from a
    // person. Prefer that group chat when there is one to resolve.
    let group_jid = call.group_jid.as_deref().filter(|_| call.is_group);
    let jid = group_jid.unwrap_or(call.chat_jid.as_str());
    // If either the group or the call's own chat is loaded, the row points at
    // that chat. A group call can arrive before its group is in the list; in
    // that case its direct chat is still a real place to open.
    let known = chats.iter().find(|chat| {
        chat.jid == jid
            || chat.jid == call.chat_jid
            || group_jid.is_some_and(|group| chat.jid == group)
    });
    let open_jid = known.map(|chat| chat.jid.clone()).unwrap_or_else(|| {
        if jid.is_empty() {
            call.caller_jid.clone()
        } else {
            jid.to_string()
        }
    });
    let name = known
        .map(|chat| chat.name.clone())
        .unwrap_or_else(|| open_jid.clone());
    let outgoing = own_jids
        .iter()
        .any(|own| !own.is_empty() && *own == call.caller_jid);
    let outcome = match call.outcome.as_str() {
        "connected" => crate::tr!(CallResultConnected).to_string(),
        "missed" => crate::tr!(CallResultMissed).to_string(),
        "cancelled" => crate::tr!(CallResultCancelled).to_string(),
        _ => crate::tr!(CallResultUnknown).to_string(),
    };
    let time = DateTime::from_timestamp_millis(call.timestamp_ms)
        .map(|when| crate::utils::format_list_time(&when))
        .unwrap_or_default();

    CallRow {
        id: call.id.clone(),
        jid: open_jid,
        name,
        direction: if outgoing {
            crate::tr!(CallDirectionOutgoing)
        } else {
            crate::tr!(CallDirectionIncoming)
        },
        kind: if call.is_video {
            crate::tr!(VideoCall)
        } else {
            crate::tr!(VoiceCall)
        },
        outcome,
        duration: format_duration(call.duration_seconds),
        time,
        can_open_chat: known.is_some(),
    }
}

fn format_duration(seconds: Option<u32>) -> Option<String> {
    seconds.map(|total| format!("{}:{:02}", total / 60, total % 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, at: i64, outcome: &str, video: bool, seconds: Option<u32>) -> CallEventDto {
        CallEventDto {
            id: id.into(),
            chat_jid: "synthetic-caller@example.invalid".into(),
            caller_jid: "synthetic-caller@example.invalid".into(),
            timestamp_ms: at,
            is_video: video,
            is_group: false,
            group_jid: None,
            participants: Vec::new(),
            group_transaction_id: None,
            group_media: None,
            duration_seconds: seconds,
            outcome: outcome.into(),
        }
    }

    fn chat(jid: &str, name: &str) -> Arc<Chat> {
        let mut chat = Chat::new(jid.to_string());
        chat.name = name.to_string();
        Arc::new(chat)
    }

    #[test]
    fn rows_are_sorted_newest_first_and_duplicate_calls_collapse() {
        let calls = vec![
            call("old", 1_000, "connected", false, Some(8)),
            call("new", 3_000, "connected", false, Some(8)),
            call("new", 3_000, "connected", false, Some(8)),
            call("middle", 2_000, "connected", false, Some(8)),
        ];
        let rows = call_rows(&calls, &[], &[], CallFilter::All);
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["new", "middle", "old"]
        );
    }

    #[test]
    fn missed_filter_keeps_only_missed_calls() {
        let calls = vec![
            call("missed", 3_000, "missed", false, None),
            call("connected", 2_000, "connected", true, Some(20)),
            call("cancelled", 1_000, "cancelled", false, None),
        ];
        let rows = call_rows(&calls, &[], &[], CallFilter::Missed);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "missed");
    }

    #[test]
    fn a_group_call_opens_the_known_direct_chat_when_the_group_is_absent() {
        let mut group_call = call("group", 3_000, "connected", true, Some(4));
        group_call.is_group = true;
        group_call.group_jid = Some("synthetic-group@example.invalid".into());
        group_call.chat_jid = "synthetic-peer@example.invalid".into();
        let chats = vec![chat("synthetic-peer@example.invalid", "Synthetic Peer")];
        let rows = call_rows(&[group_call], &chats, &[], CallFilter::All);
        assert_eq!(rows[0].jid, "synthetic-peer@example.invalid");
        assert_eq!(rows[0].name, "Synthetic Peer");
        assert!(rows[0].can_open_chat);
    }

    #[test]
    fn row_labels_resolve_names_and_keep_a_synthetic_fallback() {
        let mut outgoing = call("out", 3_000, "connected", true, Some(65));
        outgoing.caller_jid = "synthetic-self@example.invalid".into();
        let chats = vec![chat("synthetic-caller@example.invalid", "Synthetic Peer")];
        let rows = call_rows(
            &[outgoing.clone()],
            &chats,
            &["synthetic-self@example.invalid"],
            CallFilter::All,
        );
        assert_eq!(rows[0].name, "Synthetic Peer");
        assert_eq!(rows[0].direction, crate::tr!(CallDirectionOutgoing));
        assert_eq!(rows[0].kind, crate::tr!(VideoCall));
        assert_eq!(rows[0].outcome, crate::tr!(CallResultConnected));
        assert_eq!(rows[0].duration.as_deref(), Some("1:05"));
        assert!(rows[0].can_open_chat);

        let mut incoming = call("in", 2_000, "missed", false, None);
        incoming.caller_jid = "synthetic-unknown@example.invalid".into();
        incoming.chat_jid = incoming.caller_jid.clone();
        let rows = call_rows(&[incoming], &[], &[], CallFilter::All);
        assert_eq!(rows[0].name, "synthetic-unknown@example.invalid");
        assert_eq!(rows[0].direction, crate::tr!(CallDirectionIncoming));
        assert_eq!(rows[0].kind, crate::tr!(VoiceCall));
        assert_eq!(rows[0].outcome, crate::tr!(CallResultMissed));
        assert_eq!(rows[0].duration, None);
        assert!(!rows[0].can_open_chat);
    }
}
