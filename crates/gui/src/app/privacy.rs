//! The Privacy pane's view of one account-level setting: DM read receipts.
//!
//! This is server state, so the last answer is kept rather than inferred from
//! chats or from a local preference. The DTO's third state matters here: a
//! missing or newer wire value is shown as unreadable, never as a switch that
//! happens to be off.
//!
//! A change is correlated and never optimistic. The daemon sends the IQ, saves
//! the device snapshot, reads the value back and publishes it before answering
//! this request. A refusal therefore has to reach the notice stack, and the
//! switch may move only when that fresh account-wide answer arrives.

use gpui::Context;
use oxidezap_core::PrivacyReadReceipts;

use super::WhatsAppApp;
use crate::app::notices::Tone;

#[derive(Default)]
pub(super) struct PrivacyState {
    value: Option<PrivacyReadReceipts>,
    loaded: bool,
    busy: bool,
}

impl PrivacyState {
    fn value(&self) -> Option<PrivacyReadReceipts> {
        self.value
    }

    fn loaded(&self) -> bool {
        self.loaded
    }

    fn busy(&self) -> bool {
        self.busy
    }

    fn adopt(&mut self, value: PrivacyReadReceipts) {
        self.value = Some(value);
        self.loaded = true;
        self.busy = false;
    }

    fn begin_change(&mut self) -> bool {
        if self.busy {
            false
        } else {
            self.busy = true;
            true
        }
    }

    fn settle(&mut self) {
        self.busy = false;
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

impl WhatsAppApp {
    pub(crate) fn read_receipts(&self) -> Option<PrivacyReadReceipts> {
        self.privacy.value()
    }

    pub(crate) fn read_receipts_loaded(&self) -> bool {
        self.privacy.loaded()
    }

    pub(crate) fn read_receipts_busy(&self) -> bool {
        self.privacy.busy()
    }

    /// Ask for the server's value, keeping the last one while it is on the way.
    pub(crate) fn refresh_read_receipts(&mut self, cx: &mut Context<Self>) {
        if self.privacy.busy() {
            return;
        }
        let Some(client) = self.client.as_ref().map(|client| client.handle()) else {
            return;
        };
        self.privacy.begin_change();
        client.load_privacy_settings();
        cx.notify();
    }

    /// Adopt the fresh value the daemon published after a read or a change.
    pub(crate) fn adopt_read_receipts(
        &mut self,
        value: PrivacyReadReceipts,
        cx: &mut Context<Self>,
    ) {
        self.privacy.adopt(value);
        cx.notify();
    }

    /// Release a failed read and say why, rather than drawing a stale value as
    /// if it had just been confirmed.
    pub(crate) fn read_receipts_failed(&mut self, detail: &str, cx: &mut Context<Self>) {
        self.privacy.settle();
        self.notify_user(
            crate::tr!(read_receipts_read_failed, detail),
            Tone::Problem,
            cx,
        );
        cx.notify();
    }

    /// Change the account setting. The switch does not move here: the daemon's
    /// fresh value is the only thing allowed to redraw it.
    pub(crate) fn set_read_receipts(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if !self.privacy.begin_change() {
            return;
        }
        let Some(client) = self.client.as_ref().map(|client| client.handle()) else {
            self.privacy.settle();
            return;
        };
        cx.notify();
        let waiting = client.set_read_receipts(enabled);
        cx.spawn(async move |entity, cx| match waiting.await {
            Ok(Ok(())) => entity.update(cx, |app, cx| {
                // The daemon published the server's fresh value before this
                // answer. Only the busy flag is local; the value is not.
                app.privacy.settle();
                app.notify_user(crate::tr!(ReadReceiptsSaved), Tone::Info, cx);
                cx.notify();
            }),
            Ok(Err(failure)) => entity.update(cx, |app, cx| {
                app.privacy.settle();
                app.notify_user(
                    crate::tr!(read_receipts_failed, &failure),
                    Tone::Problem,
                    cx,
                );
                cx.notify();
            }),
            Err(_) => entity.update(cx, |app, cx| {
                app.privacy.settle();
                app.notify_user(
                    crate::tr!(
                        read_receipts_failed,
                        "the daemon stopped before it answered"
                    ),
                    Tone::Problem,
                    cx,
                );
                cx.notify();
            }),
        })
        .detach();
    }

    /// Forget an answer when the account it described goes away.
    pub(crate) fn reset_read_receipts(&mut self) {
        self.privacy.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unread_value_is_never_reported_as_a_known_switch() {
        let mut state = PrivacyState::default();
        assert_eq!(state.value(), None);
        assert!(!state.loaded());

        state.adopt(PrivacyReadReceipts::Unknown);
        assert_eq!(state.value(), Some(PrivacyReadReceipts::Unknown));
        assert!(state.loaded());
        assert!(!state.value().is_some_and(PrivacyReadReceipts::is_known));
        assert!(!state.busy(), "the answer releases the pane");
    }

    #[test]
    fn only_one_change_can_be_in_flight_and_a_refusal_releases_it() {
        let mut state = PrivacyState::default();
        assert!(state.begin_change());
        assert!(state.busy());
        assert!(!state.begin_change());
        state.settle();
        assert!(!state.busy());

        assert!(state.begin_change());
        state.adopt(PrivacyReadReceipts::None);
        assert!(!state.busy());
        assert_eq!(state.value(), Some(PrivacyReadReceipts::None));
    }

    #[test]
    fn leaving_the_account_discards_what_it_had_said() {
        let mut state = PrivacyState::default();
        state.adopt(PrivacyReadReceipts::All);
        state.reset();
        assert_eq!(state.value(), None);
        assert!(!state.loaded());
        assert!(!state.busy());
    }
}
