//! Lifecycle of the chat detail sheet.
//!
//! The sheet is a view over state this window already owns. It asks for a
//! group's roster through the existing member request and keeps one JID, not a
//! copy of the chat, because the roster and the messages keep arriving while
//! the sheet is open. Nothing here learns about profile data: there is no
//! server-backed About, last seen or phone label to fetch.

use super::*;

impl WhatsAppApp {
    /// Open the detail sheet for `jid`.
    ///
    /// A chat row can open the sheet for a conversation that is not selected,
    /// so selection happens first and the sheet's JID is set afterwards.
    /// `select_chat` deliberately closes a sheet belonging to a different
    /// chat; setting it here is what makes the two gestures agree.
    pub fn open_chat_info(&mut self, jid: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.chat_named(&jid).is_none() {
            return;
        }
        if self.selected_chat.as_deref() != Some(jid.as_str()) {
            self.select_chat(jid.clone(), ChatOpen::ToPreview, window, cx);
        }
        // `select_chat` asks on entry. Asking again here covers a sheet opened
        // for the chat that was already selected, and the request deduplicates
        // itself while one is in flight.
        self.request_group_members(&jid);
        self.chat_info_jid = Some(jid);
        cx.notify();
    }

    /// Put the chat area back to the conversation.
    pub fn close_chat_info(&mut self, cx: &mut Context<Self>) {
        if self.chat_info_jid.take().is_some() {
            cx.notify();
        }
    }

    /// Which chat the sheet describes, if one is open.
    pub fn chat_info_jid(&self) -> Option<&str> {
        self.chat_info_jid.as_deref()
    }

    /// Whether the roster request for `jid` is still out.
    ///
    /// This distinguishes "the connection has not answered" from "there is no
    /// answer to show", so the sheet never calls an unanswered group empty.
    pub(crate) fn group_members_pending(&self, jid: &str) -> bool {
        self.rosters_in_flight.contains(jid)
    }

    /// The mutable chat flags, including this window's unconfirmed changes.
    ///
    /// The sheet has to show what a toggle will act on, not the last row the
    /// store happened to hand us.
    pub(crate) fn chat_info_flags(&self, jid: &str) -> Option<(bool, bool)> {
        let chat = self.chat_named(jid)?;
        let view = self.chat_view(chat);
        Some((view.is_muted(), view.archived))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ChatOpen, WhatsAppApp};
    use gpui::Entity;
    use oxidezap_core::Chat;
    use std::sync::Arc;

    const DIRECT: &str = "synthetic-peer@example.invalid";
    const GROUP: &str = "synthetic-group@g.us.invalid";
    const OTHER: &str = "synthetic-other@example.invalid";

    fn init(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
    }

    fn chats() -> Vec<Arc<Chat>> {
        let mut group = Chat::new(GROUP.to_string());
        group.is_group = true;
        vec![
            Arc::new(Chat::new(DIRECT.to_string())),
            Arc::new(group),
            Arc::new(Chat::new(OTHER.to_string())),
        ]
    }

    fn fixture(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<WhatsAppApp>, &mut gpui::VisualTestContext) {
        init(cx);
        let mut app = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let app_entity = cx.new(|cx| {
                let mut app = WhatsAppApp::new(cx);
                app.chats = chats();
                app
            });
            app = Some(app_entity.clone());
            gpui_component::Root::new(app_entity, window, cx)
        });
        (app.expect("the app entity"), visual)
    }

    #[gpui::test]
    fn opening_and_closing_info_tracks_one_jid(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.chat_info_jid().is_none());
                app.open_chat_info(DIRECT.to_string(), window, cx);
                assert_eq!(app.chat_info_jid(), Some(DIRECT));
                assert_eq!(app.selected_chat.as_deref(), Some(DIRECT));
                assert!(!app.group_members_pending(DIRECT));
                app.close_chat_info(cx);
                assert!(app.chat_info_jid().is_none());
                assert_eq!(app.selected_chat.as_deref(), Some(DIRECT));
            });
        });
    }

    #[gpui::test]
    fn opening_info_for_an_unknown_chat_changes_nothing(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_chat_info("synthetic-missing@example.invalid".to_string(), window, cx);
                assert!(app.chat_info_jid().is_none());
                assert!(app.selected_chat.is_none());
            });
        });
    }

    #[gpui::test]
    fn selecting_another_chat_closes_the_sheet(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_chat_info(DIRECT.to_string(), window, cx);
                assert_eq!(app.chat_info_jid(), Some(DIRECT));
                app.select_chat(OTHER.to_string(), ChatOpen::ToPreview, window, cx);
                assert!(app.chat_info_jid().is_none());
                assert_eq!(app.selected_chat.as_deref(), Some(OTHER));
            });
        });
    }

    #[gpui::test]
    fn clearing_the_account_forgets_the_sheet(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_chat_info(DIRECT.to_string(), window, cx);
                assert_eq!(app.chat_info_jid(), Some(DIRECT));
                app.clear_account_view(window, cx);
                assert!(app.chat_info_jid().is_none());
                assert!(app.selected_chat.is_none());
                assert!(app.chats.is_empty());
            });
        });
    }

    #[cfg(target_os = "linux")]
    #[gpui::test]
    fn opening_group_info_requests_members_once(cx: &mut gpui::TestAppContext) {
        use portable_atomic::AtomicU64;
        use std::io::BufRead;
        use std::os::unix::net::UnixListener;
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "oxidezap-chat-info-{}-{}.sock",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&path).expect("a synthetic socket");
        let endpoint = oxidezap_ipc::Endpoint::connect_at(&path).expect("a client endpoint");
        let (mut peer, _) = listener.accept().expect("the client connected");
        peer.set_read_timeout(Some(Duration::from_secs(2)))
            .expect("a read deadline");
        let client = crate::session::Session::recording_test_session(endpoint);

        init(cx);
        let mut app = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app_entity = cx.new(|cx| {
                let mut app = WhatsAppApp::new(cx);
                app.chats = chats();
                app.install_test_session(Some(client));
                app
            });
            app = Some(app_entity.clone());
            gpui_component::Root::new(app_entity, window, cx)
        });
        let app = app.expect("the app entity");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_chat_info(GROUP.to_string(), window, cx);
                assert!(app.group_members_pending(GROUP));
                app.open_chat_info(GROUP.to_string(), window, cx);
                assert!(app.group_members_pending(GROUP));
            });
        });

        let mut reader = std::io::BufReader::new(&mut peer);
        let mut requested = Vec::new();
        for _ in 0..16 {
            let mut line = String::new();
            let bytes = reader
                .read_line(&mut line)
                .expect("a frame from the synthetic daemon");
            assert!(bytes > 0, "the synthetic daemon wrote no frame");
            let frame: oxidezap_ipc::Request = serde_json::from_str(&line).expect("a client frame");
            if let oxidezap_ipc::ClientRequest::GroupMembers(payload) = frame.request {
                requested.push(payload.jid);
                break;
            }
        }
        assert_eq!(requested, vec![GROUP.to_string()]);
        let _ = std::fs::remove_file(path);
    }
}
