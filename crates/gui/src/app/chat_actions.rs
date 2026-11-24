//! Chat-list mutations: pin, mute, archive and mark-unread.
//!
//! These are account state, owned by the daemon's store. A row changes in this
//! window the moment the command is given, and the store's `PinUpdate`,
//! `MuteUpdate`, `ArchiveUpdate` or `MarkChatAsReadUpdate` arrives through the
//! normal history/page refresh to settle it everywhere. The correlated reply to
//! our own request only says whether that optimistic change stood: on a
//! refusal it is taken back and the reason is shown.
//!
//! The overlay is deliberately *not* a second chat store. It holds only the
//! four flags these commands touch, applied when a row is projected for the
//! list; messages, names, ordering and every other field stay with the chat the
//! store gave us. A store-backed merge that agrees with a pending flag clears
//! that flag, so the overlay never outlives the refresh that makes it true.

use super::*;
use crate::app::chat_row::Unread;
use chrono::{DateTime, Utc};
use tokio::sync::oneshot;

/// This window's unconfirmed version of the four mutable chat flags.
///
/// `None` means "no opinion": use what the store said. `Some(false)` is an
/// opinion, so unmuting and unarchiving are distinguishable from never having
/// asked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ChatOverlay {
    pub(super) pinned: Option<bool>,
    pub(super) muted: Option<bool>,
    pub(super) archived: Option<bool>,
    pub(super) unread: Option<bool>,
}

impl ChatOverlay {
    fn merge(&mut self, change: Self) {
        if change.pinned.is_some() {
            self.pinned = change.pinned;
        }
        if change.muted.is_some() {
            self.muted = change.muted;
        }
        if change.archived.is_some() {
            self.archived = change.archived;
        }
        if change.unread.is_some() {
            self.unread = change.unread;
        }
    }

    fn is_default(self) -> bool {
        self == Self::default()
    }
}

/// The flags a sidebar row is drawn from, after this window's overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChatView {
    pub(super) pinned: bool,
    pub(super) archived: bool,
    pub(super) muted_until: Option<DateTime<Utc>>,
    pub(super) unread: Unread,
}

impl ChatView {
    pub(super) fn of(chat: &Chat, overlay: ChatOverlay) -> Self {
        let base = if chat.unread_count > 0 {
            Unread::Count(chat.unread_count)
        } else if chat.manually_unread {
            Unread::Marked
        } else {
            Unread::None
        };
        let unread = match overlay.unread {
            // Marking a chat unread gives it a badge even when it held no
            // unread count; one that already has a count keeps the number.
            Some(true) if base == Unread::None => Unread::Marked,
            Some(false) => Unread::None,
            _ => base,
        };
        let muted_until = match overlay.muted {
            // The store uses `MAX_UTC` for a mute with no expiry, which is
            // exactly the value the row's "Muted" check expects.
            Some(true) => Some(DateTime::<Utc>::MAX_UTC),
            Some(false) => None,
            None => chat.muted_until,
        };
        Self {
            pinned: overlay.pinned.unwrap_or(chat.pinned_at.is_some()),
            archived: overlay.archived.unwrap_or(chat.archived),
            muted_until,
            unread,
        }
    }

    pub(super) fn is_muted(&self) -> bool {
        self.muted_until
            .is_some_and(|until| until > wacore::time::now_utc())
    }
}

impl WhatsAppApp {
    /// The flags one chat is currently drawn with.
    pub(super) fn chat_view(&self, chat: &Chat) -> ChatView {
        let overlay = self
            .chat_overlays
            .get(chat.jid.as_str())
            .copied()
            .unwrap_or_default();
        ChatView::of(chat, overlay)
    }

    /// Fold in one optimistic change and return both the old and new overlay.
    ///
    /// The old one is what a refusal restores; the change itself says which
    /// fields this request owns. If an unrelated refresh has already settled
    /// one of them, [`Self::settle_chat_mutation`] leaves the store's answer
    /// alone instead of resurrecting a stale overlay.
    fn apply_chat_overlay(
        &mut self,
        jid: &str,
        change: ChatOverlay,
        cx: &mut Context<Self>,
    ) -> (ChatOverlay, ChatOverlay) {
        let before = self.chat_overlays.get(jid).copied().unwrap_or_default();
        let mut applied = before;
        applied.merge(change);
        if applied == before {
            return (before, applied);
        }
        self.chat_overlays.insert(jid.to_string(), applied);
        self.invalidate_chat_cache();
        cx.notify();
        (before, applied)
    }

    /// Let an authoritative, store-backed row take back an optimistic flag.
    ///
    /// Called only for rows the store actually claims. A flag is dropped when
    /// the store agrees with what we optimistically said; until then the row
    /// keeps our value, so an older load racing the mutation's own event
    /// cannot make the badge or pin flicker back.
    pub(super) fn reconcile_chat_overlay(&mut self, jid: &str, authoritative: ChatView) {
        let Some(entry) = self.chat_overlays.get_mut(jid) else {
            return;
        };
        if entry.pinned == Some(authoritative.pinned) {
            entry.pinned = None;
        }
        if entry.archived == Some(authoritative.archived) {
            entry.archived = None;
        }
        let muted = authoritative.is_muted();
        if entry.muted == Some(muted) {
            entry.muted = None;
        }
        if entry.unread == Some(authoritative.unread != Unread::None) {
            entry.unread = None;
        }
        if entry.is_default() {
            self.chat_overlays.remove(jid);
        }
    }

    /// Give a mutation the overlay it asked for and wait for its answer.
    ///
    /// On success nothing is written here: the store's event is the single
    /// source of truth for the settled row, and the reconciliation above
    /// retires our overlay when that event arrives. A refusal (or a vanished
    /// daemon) restores the previous overlay and says why.
    fn send_chat_mutation<F>(
        &mut self,
        jid: &str,
        before: ChatOverlay,
        change: ChatOverlay,
        failure_label: &'static str,
        request: F,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(&Session, &str) -> oneshot::Receiver<Result<(), Failure>>,
    {
        let jid = jid.to_string();
        let Some(waiting) = self.client().map(|client| request(client, &jid)) else {
            self.settle_chat_mutation(
                &jid,
                before,
                change,
                Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                failure_label,
                cx,
            );
            return;
        };
        cx.spawn(async move |entity, cx| {
            let failure = match waiting.await {
                Ok(Ok(())) => None,
                Ok(Err(failure)) => Some(failure),
                Err(_) => Some(Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            if let Some(failure) = failure {
                let _ = entity.update(cx, |app, cx| {
                    app.settle_chat_mutation(&jid, before, change, failure, failure_label, cx);
                });
            }
        })
        .detach();
    }

    /// Undo the fields of a refused mutation that are still ours.
    ///
    /// Only the fields this request actually touched are considered: another
    /// action may have changed a different flag of the same chat while this
    /// one was in flight, and its optimistic value must survive our refusal.
    fn settle_chat_mutation(
        &mut self,
        jid: &str,
        before: ChatOverlay,
        change: ChatOverlay,
        failure: Failure,
        failure_label: &'static str,
        cx: &mut Context<Self>,
    ) {
        let mut restored = false;
        if let Some(entry) = self.chat_overlays.get_mut(jid) {
            for (current, changed, previous) in [
                (&mut entry.pinned, change.pinned, before.pinned),
                (&mut entry.muted, change.muted, before.muted),
                (&mut entry.archived, change.archived, before.archived),
                (&mut entry.unread, change.unread, before.unread),
            ] {
                if changed.is_some() && *current == changed {
                    *current = previous;
                    restored = true;
                }
            }
            if entry.is_default() {
                self.chat_overlays.remove(jid);
            }
        }
        if restored {
            self.invalidate_chat_cache();
        }
        self.notify_user(
            what_went_wrong(failure_label, &failure),
            notices::Tone::Problem,
            cx,
        );
    }

    /// Mark a conversation unread from its row's context menu.
    pub(crate) fn mark_chat_unread(&mut self, jid: &str, cx: &mut Context<Self>) {
        let change = ChatOverlay {
            unread: Some(true),
            ..Default::default()
        };
        let (before, _) = self.apply_chat_overlay(jid, change, cx);
        self.send_chat_mutation(
            jid,
            before,
            change,
            crate::tr!(CouldNotMarkChatUnread),
            |client, jid| client.mark_unread(jid),
            cx,
        );
    }

    /// Pin or unpin a conversation in the chat list.
    pub(crate) fn set_chat_pinned(&mut self, jid: &str, pin: bool, cx: &mut Context<Self>) {
        let change = ChatOverlay {
            pinned: Some(pin),
            ..Default::default()
        };
        let (before, _) = self.apply_chat_overlay(jid, change, cx);
        self.send_chat_mutation(
            jid,
            before,
            change,
            crate::tr!(CouldNotPinChat),
            move |client, jid| client.pin_chat(jid, pin),
            cx,
        );
    }

    /// Mute or unmute a conversation's notifications.
    ///
    /// No bounded duration here: the row has no natural control for choosing
    /// one, and `None` asks the daemon for the indefinite mute the menu names.
    /// Unmuting is `Some(0)`, the protocol's unmute spelling.
    pub(crate) fn set_chat_muted(&mut self, jid: &str, mute: bool, cx: &mut Context<Self>) {
        let change = ChatOverlay {
            muted: Some(mute),
            ..Default::default()
        };
        let (before, _) = self.apply_chat_overlay(jid, change, cx);
        self.send_chat_mutation(
            jid,
            before,
            change,
            crate::tr!(CouldNotMuteConversation),
            move |client, jid| client.mute_chat(jid, (!mute).then_some(0)),
            cx,
        );
    }

    /// Move a conversation into or out of the archived list.
    pub(crate) fn set_chat_archived(&mut self, jid: &str, archive: bool, cx: &mut Context<Self>) {
        let change = ChatOverlay {
            archived: Some(archive),
            ..Default::default()
        };
        let (before, _) = self.apply_chat_overlay(jid, change, cx);
        self.send_chat_mutation(
            jid,
            before,
            change,
            crate::tr!(CouldNotArchiveChat),
            move |client, jid| client.archive_chat(jid, archive),
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JID: &str = "155555500001@g.us";

    fn fixture(cx: &mut gpui::TestAppContext, client: Option<Session>) -> Entity<WhatsAppApp> {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
        cx.new(|cx| {
            let mut app = WhatsAppApp::new(cx);
            app.install_test_session(client);
            app.chats.push(Arc::new(Chat::new(JID.to_string())));
            app
        })
    }

    fn view(app: &Entity<WhatsAppApp>, cx: &App) -> ChatView {
        app.read(cx).chat_view(app.read(cx).find_chat(JID).unwrap())
    }

    #[gpui::test]
    fn a_refused_chat_mutation_restores_the_previous_flags(cx: &mut gpui::TestAppContext) {
        let app = fixture(cx, None);
        cx.update(|cx| {
            // No daemon: the action itself has to take the optimistic mark
            // back and explain why.
            app.update(cx, |app, cx| app.set_chat_pinned(JID, true, cx));
            assert!(!view(&app, cx).pinned);
            assert!(
                app.read(cx)
                    .notices()
                    .read(cx)
                    .has_problem(crate::tr!(CouldNotPinChat))
            );

            // The same path serves a correlated refusal. Applied by hand here
            // so the test does not need a scripted daemon.
            let change = ChatOverlay {
                archived: Some(true),
                ..Default::default()
            };
            app.update(cx, |app, cx| {
                app.apply_chat_overlay(JID, change, cx);
                assert!(app.chat_view(app.find_chat(JID).unwrap()).archived);
                app.settle_chat_mutation(
                    JID,
                    ChatOverlay::default(),
                    change,
                    Failure::worth_retrying("synthetic refusal"),
                    crate::tr!(CouldNotArchiveChat),
                    cx,
                );
                assert!(!app.chat_view(app.find_chat(JID).unwrap()).archived);
                assert!(
                    app.notices()
                        .read(cx)
                        .has_problem(crate::tr!(CouldNotArchiveChat))
                );
            });
        });
    }

    #[gpui::test]
    fn a_refusal_leaves_an_unrelated_pending_flag_alone(cx: &mut gpui::TestAppContext) {
        let app = fixture(cx, None);
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                let pin = ChatOverlay {
                    pinned: Some(true),
                    ..Default::default()
                };
                let (before, _) = app.apply_chat_overlay(JID, pin, cx);
                app.apply_chat_overlay(
                    JID,
                    ChatOverlay {
                        muted: Some(true),
                        ..Default::default()
                    },
                    cx,
                );
                app.settle_chat_mutation(
                    JID,
                    before,
                    pin,
                    Failure::worth_retrying("synthetic refusal"),
                    crate::tr!(CouldNotPinChat),
                    cx,
                );
                let view = app.chat_view(app.find_chat(JID).unwrap());
                assert!(!view.pinned, "the refused pin went back");
                assert!(view.is_muted(), "the still-pending mute survived");
            });
        });
    }

    #[gpui::test]
    fn a_store_row_that_agrees_retires_the_optimistic_flag(cx: &mut gpui::TestAppContext) {
        let app = fixture(cx, None);
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.apply_chat_overlay(
                    JID,
                    ChatOverlay {
                        pinned: Some(true),
                        muted: Some(true),
                        unread: Some(true),
                        ..Default::default()
                    },
                    cx,
                );
                let chat = app.find_chat(JID).unwrap();
                let projected = app.chat_view(chat);
                assert!(projected.pinned);
                assert!(projected.is_muted());
                assert_ne!(projected.unread, Unread::None);
                let pinned = super::super::chats::ChatFilter::Pinned;
                assert!(pinned.matches_view(chat, &projected));

                let mut authoritative = Chat::from_store(JID.to_string(), "Synthetic".into(), 0);
                authoritative.pinned_at = Some(
                    chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("synthetic pin time"),
                );
                authoritative.muted_until = Some(DateTime::<Utc>::MAX_UTC);
                authoritative.manually_unread = true;
                app.merge_chats(vec![authoritative], cx);
                assert!(
                    !app.chat_overlays.contains_key(JID),
                    "the store now says the same thing"
                );
            });
        });
    }

    /// A stalled peer leaves the request unanswered, so the optimistic row is
    /// observable before the refusal is simulated. No message crosses the
    /// socket: this only gives the session somewhere to write the request.
    #[cfg(target_os = "linux")]
    #[gpui::test]
    fn a_pending_chat_mutation_shows_its_optimistic_row(cx: &mut gpui::TestAppContext) {
        use portable_atomic::AtomicU64;
        use std::sync::atomic::Ordering;

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "oxidezap-chat-actions-{}-{}.sock",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let endpoint = oxidezap_ipc::Endpoint::connect_at(&path).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        let client = crate::session::Session::recording_test_session(endpoint);
        let app = fixture(cx, Some(client));
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.set_chat_pinned(JID, true, cx);
                assert!(app.chat_view(app.find_chat(JID).unwrap()).pinned);
                app.set_chat_archived(JID, false, cx);
                assert!(!app.chat_view(app.find_chat(JID).unwrap()).archived);
                app.set_chat_muted(JID, true, cx);
                assert!(app.chat_view(app.find_chat(JID).unwrap()).is_muted());
                app.mark_chat_unread(JID, cx);
                assert_ne!(
                    app.chat_view(app.find_chat(JID).unwrap()).unread,
                    Unread::None
                );
            });
        });
        let _ = std::fs::remove_file(path);
    }
}
