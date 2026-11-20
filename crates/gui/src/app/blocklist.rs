//! The Privacy pane's view of the account's block list.
//!
//! The list is server state, so this holds the last answer rather than trying
//! to derive one from chats. One request is kept in flight per pane: a second
//! click while the first is on the wire would only replace an answer nobody
//! had drawn yet, and the busy state is what keeps the buttons honest.
//!
//! A change is correlated, not told. Blocking and unblocking alter the
//! WhatsApp account, so a refusal has to reach the notice stack and a success
//! has to be followed by a fresh read rather than by guessing at the list.

use gpui::{AppContext, Context, Entity, Window};
use gpui_component::input::InputState;
use oxidezap_core::{BlockedContact, normalize_block_target};

use super::WhatsAppApp;
use crate::app::notices::Tone;

#[derive(Default)]
pub(super) struct BlocklistState {
    entries: Vec<BlockedContact>,
    loaded: bool,
    busy: bool,
    /// Built on first render: an input needs a window, and Settings can also be
    /// opened from a call's camera preview, where no Privacy field is drawn.
    target: Option<Entity<InputState>>,
}

impl BlocklistState {
    pub(crate) fn entries(&self) -> &[BlockedContact] {
        &self.entries
    }

    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn busy(&self) -> bool {
        self.busy
    }

    pub(crate) fn target(&self) -> Option<&Entity<InputState>> {
        self.target.as_ref()
    }

    fn adopt(&mut self, entries: Vec<BlockedContact>) {
        self.entries = entries;
        self.loaded = true;
        self.busy = false;
    }

    fn failed(&mut self) {
        self.busy = false;
    }

    fn reset(&mut self) {
        self.entries.clear();
        self.loaded = false;
        self.busy = false;
    }
}

impl WhatsAppApp {
    /// Create the Add field the first time Settings actually draws it.
    ///
    /// A text input needs a window to own its focus state, which is why this
    /// happens in the render pass rather than in `new`: the tests and the web
    /// bridge both build an app with no window at all.
    pub(crate) fn ensure_blocklist_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocklist.target.is_none() {
            self.blocklist.target = Some(
                cx.new(|cx| InputState::new(window, cx).placeholder(crate::tr!(BlockTargetJid))),
            );
        }
    }

    pub(crate) fn blocklist_entries(&self) -> &[BlockedContact] {
        self.blocklist.entries()
    }

    pub(crate) fn blocklist_loaded(&self) -> bool {
        self.blocklist.loaded()
    }

    pub(crate) fn blocklist_busy(&self) -> bool {
        self.blocklist.busy()
    }

    pub(crate) fn blocklist_field(&self) -> Option<&Entity<InputState>> {
        self.blocklist.target()
    }

    /// Ask for the server's list, keeping the last one while it is on the way.
    pub(crate) fn refresh_blocklist(&mut self, cx: &mut Context<Self>) {
        if self.blocklist.busy() {
            return;
        }
        let Some(client) = self.client.as_ref().map(|client| client.handle()) else {
            return;
        };
        self.blocklist.busy = true;
        client.load_blocklist();
        cx.notify();
    }

    /// Adopt a list the daemon answered with.
    pub(crate) fn adopt_blocklist(&mut self, entries: Vec<BlockedContact>, cx: &mut Context<Self>) {
        self.blocklist.adopt(entries);
        cx.notify();
    }

    /// Release the in-flight request and say why it produced no list.
    pub(crate) fn blocklist_failed(&mut self, detail: &str, cx: &mut Context<Self>) {
        self.blocklist.failed();
        self.notify_user(crate::tr!(blocklist_failed, detail), Tone::Problem, cx);
        cx.notify();
    }

    /// Block what is in the Add field.
    ///
    /// Validation happens here as well as in the session: a group or a
    /// malformed string is a typing mistake, not a request worth an IQ, and
    /// the sentence somebody reads should name that rather than a server
    /// rejection. The field is cleared once the address has been accepted
    /// locally, so a second click cannot send the same block twice while the
    /// first is still on the wire.
    pub(crate) fn block_contact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.client.as_ref().map(|client| client.handle()) else {
            return;
        };
        let Some(field) = self.blocklist.target.clone() else {
            return;
        };
        let typed = field.read(cx).value().to_string();
        let target = match normalize_block_target(&typed) {
            Ok(target) => target,
            Err(_) => {
                self.notify_user(crate::tr!(BlockedInvalidTarget), Tone::Problem, cx);
                return;
            }
        };
        field.update(cx, |state, cx| state.set_value("", window, cx));
        let waiting = client.block(target);
        cx.spawn(async move |entity, cx| match waiting.await {
            Ok(Ok(())) => entity.update(cx, |app, cx| {
                app.notify_user(crate::tr!(Blocked), Tone::Info, cx);
                app.refresh_blocklist(cx);
            }),
            Ok(Err(failure)) => entity.update(cx, |app, cx| {
                app.notify_user(crate::tr!(block_failed, &failure), Tone::Problem, cx);
            }),
            Err(_) => entity.update(cx, |app, cx| {
                app.notify_user(
                    crate::tr!(block_failed, "the daemon stopped before it answered"),
                    Tone::Problem,
                    cx,
                );
            }),
        })
        .detach();
    }

    /// Unblock one address from a row in the list.
    pub(crate) fn unblock_contact(&mut self, jid: String, cx: &mut Context<Self>) {
        let Some(client) = self.client.as_ref().map(|client| client.handle()) else {
            return;
        };
        let waiting = client.unblock(jid);
        cx.spawn(async move |entity, cx| match waiting.await {
            Ok(Ok(())) => entity.update(cx, |app, cx| {
                app.notify_user(crate::tr!(Unblocked), Tone::Info, cx);
                app.refresh_blocklist(cx);
            }),
            Ok(Err(failure)) => entity.update(cx, |app, cx| {
                app.notify_user(crate::tr!(unblock_failed, &failure), Tone::Problem, cx);
            }),
            Err(_) => entity.update(cx, |app, cx| {
                app.notify_user(
                    crate::tr!(unblock_failed, "the daemon stopped before it answered"),
                    Tone::Problem,
                    cx,
                );
            }),
        })
        .detach();
    }

    /// Forget an answer when the account it described goes away.
    pub(crate) fn reset_blocklist(&mut self) {
        self.blocklist.reset();
    }
}
