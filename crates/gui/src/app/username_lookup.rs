//! The Contacts pane's view of one username lookup.
//!
//! A lookup is a network read, not state: nothing is shown until somebody asks
//! and the last typed answer stays on screen while a new one is in flight. One
//! request is kept in flight so two presses cannot race two answers into the
//! same card. The key field exists because the server answers `KeyRequired`
//! before disclosing some identities; it is never filled in or guessed here.

use gpui::{AppContext, Context, Entity, Window};
use gpui_component::input::InputState;
use oxidezap_core::{
    USERNAME_MAX_LENGTH, USERNAME_MIN_LENGTH, UsernameLookupResult, normalize_username,
};

use super::WhatsAppApp;
use crate::app::notices::Tone;

/// Which surface the one in-flight username answer belongs to.
///
/// The daemon answer carries no request id, so the surface that asked is
/// remembered beside the request. This keeps an answer asked for by the new
/// chat dialog from becoming a Settings result, and drops an answer whose
/// dialog has already closed rather than showing it anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum LookupOwner {
    /// The Settings Contacts pane, which also owns the idle case.
    #[default]
    Settings,
    /// The new chat dialog, while it is still open.
    NewChat,
    /// A dialog that closed before its answer came back. Drop the answer.
    Abandoned,
}

#[derive(Default)]
pub(super) struct UsernameLookupState {
    result: Option<UsernameLookupResult>,
    busy: bool,
    /// Both fields are built on first render: an input needs a window, and the
    /// tests (and the web bridge) build an app with no window at all.
    username: Option<Entity<InputState>>,
    key: Option<Entity<InputState>>,
}

impl UsernameLookupState {
    fn adopt(&mut self, result: UsernameLookupResult) {
        self.result = Some(result);
        self.busy = false;
    }

    fn failed(&mut self) {
        self.busy = false;
    }

    fn reset(&mut self) {
        self.result = None;
        self.busy = false;
    }
}

impl WhatsAppApp {
    pub(crate) fn ensure_username_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.username_lookup.username.is_none() {
            self.username_lookup.username = Some(
                cx.new(|cx| InputState::new(window, cx).placeholder(crate::tr!(UsernameHandle))),
            );
        }
        if self.username_lookup.key.is_none() {
            self.username_lookup.key = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder(crate::tr!(UsernameKeyOptional))
            }));
        }
    }

    pub(crate) fn username_lookup_result(&self) -> Option<&UsernameLookupResult> {
        self.username_lookup.result.as_ref()
    }

    pub(crate) fn username_lookup_busy(&self) -> bool {
        self.username_lookup.busy
    }

    pub(crate) fn username_field(&self) -> Option<&Entity<InputState>> {
        self.username_lookup.username.as_ref()
    }

    pub(crate) fn username_key_field(&self) -> Option<&Entity<InputState>> {
        self.username_lookup.key.as_ref()
    }

    /// Ask the daemon to resolve what was typed.
    ///
    /// The handle is checked here as well as in the session: a short handle is
    /// a typing mistake, and the server's own limit is worth saying before an
    /// IQ is sent. The key stays in its field so a `KeyRequired` answer can be
    /// acted on with a second press.
    pub(crate) fn lookup_username(&mut self, cx: &mut Context<Self>) {
        if self.username_lookup.busy || self.lookup_owner != LookupOwner::Settings {
            return;
        }
        let Some(client) = self.client().map(|client| client.handle()) else {
            return;
        };
        let (Some(username), Some(key)) = (
            self.username_lookup.username.clone(),
            self.username_lookup.key.clone(),
        ) else {
            return;
        };
        let Some(username) = normalize_username(&username.read(cx).value()) else {
            self.notify_user(
                crate::tr!(invalid_username, USERNAME_MIN_LENGTH, USERNAME_MAX_LENGTH),
                Tone::Problem,
                cx,
            );
            return;
        };
        let typed_key = key.read(cx).value().to_string();
        let typed_key = typed_key.trim();
        let key = (!typed_key.is_empty()).then(|| typed_key.to_string());
        self.username_lookup.busy = true;
        client.lookup_username(username, key);
        cx.notify();
    }

    /// Show the typed answer, and name the two outcomes that are not a result.
    pub(crate) fn adopt_username_lookup(
        &mut self,
        result: UsernameLookupResult,
        cx: &mut Context<Self>,
    ) {
        match self.lookup_owner {
            LookupOwner::NewChat => {
                self.lookup_owner = LookupOwner::Settings;
                self.adopt_new_chat_lookup(result, cx);
                return;
            }
            LookupOwner::Abandoned => {
                // The dialog that asked is gone, so there is no surface that
                // should show this answer. Drop it rather than hand it to
                // Settings, which never asked.
                self.lookup_owner = LookupOwner::Settings;
                return;
            }
            LookupOwner::Settings => {}
        }
        match &result {
            UsernameLookupResult::KeyRequired { .. } => {
                self.notify_user(crate::tr!(UsernameKeyRequired), Tone::Info, cx);
            }
            UsernameLookupResult::NotFound => {
                self.notify_user(crate::tr!(UsernameNotFound), Tone::Info, cx);
            }
            UsernameLookupResult::InvalidUsername => {
                self.notify_user(
                    crate::tr!(invalid_username, USERNAME_MIN_LENGTH, USERNAME_MAX_LENGTH),
                    Tone::Problem,
                    cx,
                );
            }
            UsernameLookupResult::Found(_) => {}
        }
        self.username_lookup.adopt(result);
        cx.notify();
    }

    /// Release the in-flight request and say why there is no answer.
    pub(crate) fn username_lookup_failed(&mut self, detail: &str, cx: &mut Context<Self>) {
        match self.lookup_owner {
            LookupOwner::NewChat => {
                self.lookup_owner = LookupOwner::Settings;
                self.new_chat_lookup_failed(cx);
                return;
            }
            LookupOwner::Abandoned => {
                self.lookup_owner = LookupOwner::Settings;
                return;
            }
            LookupOwner::Settings => {}
        }
        self.username_lookup.failed();
        self.notify_user(
            crate::tr!(username_lookup_failed, detail),
            Tone::Problem,
            cx,
        );
        cx.notify();
    }

    /// Forget an answer when the account it described goes away.
    pub(crate) fn reset_username_lookup(&mut self) {
        // Anything still in flight belonged to the departing account, so no
        // later answer should be adopted as the next account's Settings result.
        self.lookup_owner = LookupOwner::Settings;
        self.username_lookup.reset();
    }
}
