//! The Privacy pane's view of the account's server-owned settings: DM read
//! receipts, and the six generic visibility categories.
//!
//! This is server state, so the last answer is kept rather than inferred from
//! chats or from a local preference. The DTO's third state matters here: a
//! missing or newer wire value is shown as unreadable, never as a switch that
//! happens to be off.
//!
//! A change is correlated and never optimistic. The daemon sends the IQ, saves
//! the device snapshot, reads the value back and publishes it before answering
//! this request. A refusal therefore has to reach the notice stack, and the
//! control may move only when that fresh account-wide answer arrives. The
//! dedicated read-receipts path keeps its own state and is deliberately left
//! unchanged beside the generic categories.

use gpui::Context;
use oxidezap_core::{
    PrivacyReadReceipts, PrivacySettingCategory, PrivacySettingValue, PrivacySettings,
};

use super::WhatsAppApp;
use crate::app::notices::Tone;

#[derive(Default)]
pub(super) struct PrivacyState {
    value: Option<PrivacyReadReceipts>,
    loaded: bool,
    busy: bool,
    /// The whole answer for the six categories, as the daemon last published
    /// it. Kept while a newer read is on the way.
    settings: PrivacySettings,
    categories_loaded: bool,
    categories_busy: bool,
    /// Only true when a read failed with no earlier answer to fall back on.
    /// Without it the pane would say "loading" for a read that never came.
    categories_failed: bool,
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

    fn settings(&self) -> &PrivacySettings {
        &self.settings
    }

    fn categories_loaded(&self) -> bool {
        self.categories_loaded
    }

    fn categories_busy(&self) -> bool {
        self.categories_busy
    }

    fn categories_failed(&self) -> bool {
        self.categories_failed
    }

    fn adopt_categories(&mut self, settings: PrivacySettings) {
        self.settings = settings;
        self.categories_loaded = true;
        self.categories_busy = false;
        self.categories_failed = false;
    }

    /// One category read or change at a time, whatever its kind. A fresh ask
    /// stops the last failure from being shown as the state of a new attempt.
    fn begin_category_request(&mut self) -> bool {
        if self.categories_busy {
            return false;
        }
        self.categories_busy = true;
        self.categories_failed = false;
        true
    }

    fn settle_categories(&mut self) {
        self.categories_busy = false;
    }

    fn fail_categories(&mut self) {
        self.categories_busy = false;
        // An earlier answer is still worth showing; only a first read that
        // failed has nothing to put in its place.
        self.categories_failed = !self.categories_loaded;
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

    pub(crate) fn privacy_categories(&self) -> &PrivacySettings {
        self.privacy.settings()
    }

    pub(crate) fn privacy_categories_loaded(&self) -> bool {
        self.privacy.categories_loaded()
    }

    pub(crate) fn privacy_categories_busy(&self) -> bool {
        self.privacy.categories_busy()
    }

    /// Whether the last read failed with no earlier answer behind it.
    pub(crate) fn privacy_categories_unavailable(&self) -> bool {
        self.privacy.categories_failed()
    }

    /// Ask for the six categories, keeping the last answer on screen while the
    /// new one is in flight. One read or change at a time reaches the daemon.
    pub(crate) fn refresh_privacy_categories(&mut self, cx: &mut Context<Self>) {
        if !self.privacy.begin_category_request() {
            return;
        }
        let Some(client) = self.client.as_ref().map(|client| client.handle()) else {
            self.privacy.settle_categories();
            return;
        };
        client.load_privacy_categories();
        cx.notify();
    }

    /// Adopt the fresh account-wide answer, from a read or from a change.
    pub(crate) fn adopt_privacy_categories(
        &mut self,
        settings: PrivacySettings,
        cx: &mut Context<Self>,
    ) {
        self.privacy.adopt_categories(settings);
        cx.notify();
    }

    /// Release a failed read and say why. An earlier answer stays on screen;
    /// with none to keep, the pane says the categories are unavailable.
    pub(crate) fn privacy_categories_failed(&mut self, detail: &str, cx: &mut Context<Self>) {
        self.privacy.fail_categories();
        self.notify_user(
            crate::tr!(privacy_categories_read_failed, detail),
            Tone::Problem,
            cx,
        );
        cx.notify();
    }

    /// Change one category. No local value is flipped here: the daemon
    /// publishes the fresh server answer before the correlated result, and
    /// that answer alone redraws the row.
    pub(crate) fn set_privacy_category(
        &mut self,
        category: PrivacySettingCategory,
        value: PrivacySettingValue,
        cx: &mut Context<Self>,
    ) {
        if !self.privacy.begin_category_request() {
            return;
        }
        let Some(client) = self.client.as_ref().map(|client| client.handle()) else {
            self.privacy.settle_categories();
            return;
        };
        cx.notify();
        let waiting = client.set_privacy_category(category, value);
        cx.spawn(async move |entity, cx| match waiting.await {
            Ok(Ok(())) => entity.update(cx, |app, cx| {
                app.privacy.settle_categories();
                app.notify_user(crate::tr!(PrivacySettingSaved), Tone::Info, cx);
                cx.notify();
            }),
            Ok(Err(failure)) => entity.update(cx, |app, cx| {
                app.privacy.settle_categories();
                app.notify_user(
                    crate::tr!(privacy_category_change_failed, failure),
                    Tone::Problem,
                    cx,
                );
                cx.notify();
            }),
            Err(_) => entity.update(cx, |app, cx| {
                app.privacy.settle_categories();
                app.notify_user(
                    crate::tr!(
                        privacy_category_change_failed,
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

    /// Forget both answers when the account it described goes away.
    pub(crate) fn reset_privacy(&mut self) {
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

    /// A read and a change share the one slot, whatever the order they come.
    #[test]
    fn only_one_category_request_can_be_in_flight() {
        let mut state = PrivacyState::default();
        assert!(state.begin_category_request());
        assert!(state.categories_busy());
        assert!(
            !state.begin_category_request(),
            "a read or a change, not both"
        );
        state.settle_categories();
        assert!(state.begin_category_request());
        state.adopt_categories(PrivacySettings::default());
        assert!(
            !state.categories_busy(),
            "the published answer releases the pane"
        );
    }

    /// A refresh does not blank the answer already on screen: a row that is a
    /// moment out of date is better than an empty pane while the read runs.
    #[test]
    fn a_refresh_keeps_the_last_answer_in_flight() {
        let mut state = PrivacyState::default();
        let last = PrivacySettings {
            settings: vec![oxidezap_core::PrivacySetting {
                category: PrivacySettingCategory::LastSeen,
                value: PrivacySettingValue::Contacts,
            }],
        };
        state.adopt_categories(last.clone());
        assert!(state.begin_category_request());
        assert_eq!(state.settings(), &last);
        assert!(state.categories_loaded());
        assert!(state.categories_busy());
        assert!(!state.categories_failed());
    }

    /// Recognising a value is not understanding it. Unknown and the disallowed
    /// list stay exactly as reported so the pane can say so rather than draw a
    /// known state over them.
    #[test]
    fn unsupported_values_stay_as_reported() {
        let mut state = PrivacyState::default();
        state.adopt_categories(PrivacySettings {
            settings: vec![
                oxidezap_core::PrivacySetting {
                    category: PrivacySettingCategory::Online,
                    value: PrivacySettingValue::Unknown,
                },
                oxidezap_core::PrivacySetting {
                    category: PrivacySettingCategory::Messages,
                    value: PrivacySettingValue::ContactBlacklist,
                },
            ],
        });
        assert_eq!(
            state
                .settings()
                .get(PrivacySettingCategory::Online)
                .map(|setting| setting.value),
            Some(PrivacySettingValue::Unknown)
        );
        assert_eq!(
            state
                .settings()
                .get(PrivacySettingCategory::Messages)
                .map(|setting| setting.value),
            Some(PrivacySettingValue::ContactBlacklist)
        );
    }

    /// A failed read releases the pane without pretending an answer arrived.
    #[test]
    fn a_refused_category_read_releases_the_pane_without_an_answer() {
        let mut state = PrivacyState::default();
        assert!(state.begin_category_request());
        state.fail_categories();
        assert!(!state.categories_busy());
        assert!(state.categories_failed());
        assert!(!state.categories_loaded());
        assert!(state.settings().settings.is_empty());
        assert!(
            state.begin_category_request(),
            "the failure does not hold the next ask"
        );
        assert!(!state.categories_failed());
    }

    #[test]
    fn leaving_the_account_discards_both_privacy_answers() {
        let mut state = PrivacyState::default();
        state.adopt(PrivacyReadReceipts::All);
        state.adopt_categories(PrivacySettings {
            settings: vec![oxidezap_core::PrivacySetting {
                category: PrivacySettingCategory::GroupAdd,
                value: PrivacySettingValue::Contacts,
            }],
        });
        state.reset();
        assert_eq!(state.value(), None);
        assert!(!state.loaded());
        assert!(!state.busy());
        assert!(state.settings().settings.is_empty());
        assert!(!state.categories_loaded());
        assert!(!state.categories_busy());
        assert!(!state.categories_failed());
    }
}
