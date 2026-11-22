//! The two account-level message panels: saved (starred) and pinned.
//!
//! Both reuse the global search's hit rows, because a hit is the same thing in
//! all three places: a chat, a message id and a line of text to click. The
//! saved panel is filled by the daemon's `StarredResults`. There is no listing
//! request for pins yet, so the pinned panel is derived from the `PinChanged`
//! events this window has seen and the messages it has loaded: a pin from a
//! different front end or a message outside the loaded page has no row here.

use gpui::{App, Context, Entity, FocusHandle, Styled, div, prelude::*};
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{Icon, IconName, v_flex};
use oxidezap_ipc::MessageSearchHit;

use super::{KeyboardOwner, WhatsAppApp, search, what_went_wrong};
use crate::theme::ActiveProductTheme as _;

/// How many rows either panel keeps, and what the store is asked to send.
pub(super) const LIST_LIMIT: usize = 50;

/// Which of the two panels a frame is drawing.
#[derive(Clone, Copy)]
pub(super) enum MessagePanel {
    Saved,
    Pinned,
}

impl MessagePanel {
    fn title(self) -> &'static str {
        match self {
            Self::Saved => crate::tr!(SavedMessages),
            Self::Pinned => crate::tr!(PinnedMessages),
        }
    }

    fn empty(self) -> &'static str {
        match self {
            Self::Saved => crate::tr!(NoSavedMessages),
            Self::Pinned => crate::tr!(NoPinnedMessages),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Saved => "saved-messages-panel",
            Self::Pinned => "pinned-messages-panel",
        }
    }

    /// A separate name for the rows, so a panel and its own list do not answer
    /// to the same debug selector.
    fn hit_id(self) -> &'static str {
        match self {
            Self::Saved => "saved-messages-hit",
            Self::Pinned => "pinned-messages-hit",
        }
    }

    fn close(self, app: &Entity<WhatsAppApp>, cx: &mut App) {
        app.update(cx, |app, cx| match self {
            Self::Saved => {
                app.close_saved_messages(cx);
            }
            Self::Pinned => {
                app.close_pinned_messages(cx);
            }
        });
    }
}

impl WhatsAppApp {
    /// Open the saved panel and ask the store for the current rows.
    pub(super) fn open_saved_messages(&mut self, cx: &mut Context<Self>) {
        self.close_global_search(cx);
        self.close_pinned_messages(cx);
        self.saved_messages_panel = true;
        self.keyboard_owner = Some(KeyboardOwner::SavedMessages);
        cx.notify();
        self.request_starred(cx);
    }

    pub(super) fn close_saved_messages(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.saved_messages_panel {
            return false;
        }
        self.saved_messages_panel = false;
        self.keyboard_owner = None;
        cx.notify();
        true
    }

    /// Open the pinned panel from the marks this window has observed.
    pub(super) fn open_pinned_messages(&mut self, cx: &mut Context<Self>) {
        self.close_global_search(cx);
        self.close_saved_messages(cx);
        self.pinned_messages_panel = true;
        self.keyboard_owner = Some(KeyboardOwner::PinnedMessages);
        cx.notify();
    }

    pub(super) fn close_pinned_messages(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.pinned_messages_panel {
            return false;
        }
        self.pinned_messages_panel = false;
        self.keyboard_owner = None;
        cx.notify();
        true
    }

    /// The listing is a local read: the ack only says it ran. The rows arrive
    /// separately as `FromDaemon::StarredResults`, like a search answer.
    fn request_starred(&mut self, cx: &mut Context<Self>) {
        let Some(waiting) = self
            .client
            .as_ref()
            .map(|client| client.list_starred_messages(LIST_LIMIT))
        else {
            self.notify_user(
                what_went_wrong(
                    crate::tr!(CouldNotLoadSavedMessages),
                    &super::Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                ),
                super::notices::Tone::Problem,
                cx,
            );
            return;
        };
        cx.spawn(async move |entity, cx| {
            let failure = match waiting.await {
                Ok(Ok(())) => None,
                Ok(Err(failure)) => Some(failure),
                Err(_) => Some(super::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            let _ = entity.update(cx, |app, cx| {
                if let Some(failure) = failure {
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotLoadSavedMessages), &failure),
                        super::notices::Tone::Problem,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn rows_for_hits<'a>(
        &self,
        hits: impl IntoIterator<Item = &'a MessageSearchHit>,
    ) -> Vec<search::SearchResultRow> {
        hits.into_iter()
            .map(|hit| search::SearchResultRow {
                chat_jid: hit.chat_jid.clone().into(),
                chat_name: self
                    .chat_named(&hit.chat_jid)
                    .map(|chat| chat.name.clone())
                    .unwrap_or_else(|| hit.chat_jid.clone())
                    .into(),
                message_id: hit.message_id.clone().into(),
                snippet: hit.snippet.clone().into(),
                timestamp_ms: hit.timestamp_ms,
                has_media: hit.has_media,
            })
            .collect()
    }

    /// Rows for the saved panel, from the latest store-wide listing.
    pub(super) fn saved_rows(&self) -> Vec<search::SearchResultRow> {
        self.rows_for_hits(self.saved_messages.iter())
    }

    /// Rows for the pinned panel, newest first. Only chats and messages this
    /// window has loaded can be named here; see the module note.
    pub(super) fn pinned_rows(&self) -> Vec<search::SearchResultRow> {
        let hits: Vec<MessageSearchHit> = self
            .pinned_messages
            .iter()
            .filter_map(|(chat_jid, message_id)| {
                let message = self
                    .chat_named(chat_jid)?
                    .messages
                    .iter()
                    .find(|message| &message.id == message_id)?;
                Some(MessageSearchHit {
                    chat_jid: chat_jid.clone(),
                    message_id: message_id.clone(),
                    snippet: message.preview_text(),
                    sender_jid: None,
                    timestamp_ms: message.timestamp.timestamp_millis(),
                    has_media: message.media.is_some(),
                })
            })
            .collect();
        let mut rows = self.rows_for_hits(hits.iter());
        rows.sort_unstable_by_key(|row| std::cmp::Reverse(row.timestamp_ms));
        rows.truncate(LIST_LIMIT);
        rows
    }
}

/// The panel shell, shared by the saved and pinned lists.
pub(super) fn render_panel(
    panel: MessagePanel,
    rows: &[search::SearchResultRow],
    focus: &FocusHandle,
    app: &Entity<WhatsAppApp>,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let close_app = app.clone();
    let result_rows = search::render_hit_rows(rows, app, panel.hit_id(), cx);
    div()
        .id(panel.id())
        .debug_selector(|| panel.id().into())
        .track_focus(focus)
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .pt(metrics.space_xxl())
        .child(
            v_flex()
                .w(metrics.reading_width())
                .max_w_full()
                .rounded(metrics.radius_xl())
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .p(metrics.space_lg())
                .gap(metrics.space_md())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(metrics.space_sm())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(metrics.space_sm())
                                .child(
                                    Icon::new(gpui_kit_assets::IconName::Bookmark)
                                        .with_size(metrics.icon())
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .child(
                                    div()
                                        .text_size(metrics.text_heading())
                                        .text_color(cx.theme().foreground)
                                        .child(panel.title()),
                                ),
                        )
                        .child(
                            Button::new(format!("{}-close", panel.id()))
                                .icon(IconName::Close)
                                .ghost()
                                .tooltip(crate::tr!(Close))
                                .on_click(move |_, _, cx| panel.close(&close_app, cx)),
                        ),
                )
                .child(div().flex_1().min_h_0().overflow_y_scrollbar().children(
                    if rows.is_empty() {
                        vec![
                            div()
                                .text_size(metrics.text_secondary())
                                .text_color(cx.theme().muted_foreground)
                                .child(panel.empty())
                                .into_any_element(),
                        ]
                    } else {
                        result_rows
                    },
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::body::tests::connected_app_fixture;
    use chrono::TimeZone as _;
    use oxidezap_core::ChatMessage;

    const CHAT: &str = "peer@example.invalid";

    fn message(id: &str, content: &str, timestamp_ms: i64) -> ChatMessage {
        let mut message = ChatMessage::new_incoming(id.into(), CHAT.into(), content.into());
        message.timestamp = chrono::Utc.timestamp_millis_opt(timestamp_ms).unwrap();
        message
    }

    fn saved_hit(id: &str, timestamp_ms: i64) -> MessageSearchHit {
        MessageSearchHit {
            chat_jid: CHAT.into(),
            message_id: id.into(),
            snippet: "a synthetic saved message".into(),
            sender_jid: None,
            timestamp_ms,
            has_media: false,
        }
    }

    /// A panel row is a place to jump to, exactly like a search hit. Both
    /// panels get their own debug selector so a test can tell them apart.
    #[gpui::test]
    fn saved_and_pinned_panels_show_rows_and_open_the_hit(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = connected_app_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.add_message_to_chat(
                    CHAT,
                    message("SAVED-1", "saved line", 1_700_000_000_000),
                    cx
                ));
                assert!(app.add_message_to_chat(
                    CHAT,
                    message("PINNED-1", "pinned line", 1_700_000_001_000),
                    cx
                ));
                app.adopt_starred_results(vec![saved_hit("SAVED-1", 1_700_000_000_000)], cx);
                app.adopt_pin_changed(CHAT, "PINNED-1", true, cx);
                app.open_saved_messages(cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("saved-messages-panel").is_some());
        let saved_row = cx
            .debug_bounds("saved-messages-hit")
            .expect("a saved row is drawn");
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(app.saved_messages_panel);
            assert!(!app.pinned_messages_panel);
            assert_eq!(app.keyboard_owner, Some(KeyboardOwner::SavedMessages));
            assert_eq!(app.saved_rows().len(), 1);
            assert_eq!(app.saved_rows()[0].message_id, "SAVED-1");
        });

        cx.simulate_click(saved_row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(!app.saved_messages_panel);
            assert_ne!(app.keyboard_owner, Some(KeyboardOwner::SavedMessages));
            assert_eq!(
                app.focused_message
                    .as_ref()
                    .map(|(jid, id)| (jid.as_str(), id.as_str())),
                Some((CHAT, "SAVED-1"))
            );
            assert_eq!(app.selected_chat.as_deref(), Some(CHAT));
        });

        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.open_pinned_messages(cx));
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("pinned-messages-panel").is_some());
        let pinned_row = cx
            .debug_bounds("pinned-messages-hit")
            .expect("a pinned row is drawn");
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(app.pinned_messages_panel);
            assert!(!app.saved_messages_panel);
            assert_eq!(app.keyboard_owner, Some(KeyboardOwner::PinnedMessages));
            assert_eq!(app.pinned_rows().len(), 1);
            assert_eq!(app.pinned_rows()[0].message_id, "PINNED-1");
        });

        cx.simulate_click(pinned_row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(!app.pinned_messages_panel);
            assert_eq!(
                app.focused_message
                    .as_ref()
                    .map(|(jid, id)| (jid.as_str(), id.as_str())),
                Some((CHAT, "PINNED-1"))
            );
        });
    }
}
