//! Sent-message actions, scoped to the chat and row the menu named.

use super::*;
use gpui::SharedString;
use gpui_component::ActiveTheme as _;
use gpui_component::FocusTrapElement as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement as _;

/// A deletion choice captured from one message's context menu. The request is
/// not made until the user confirms this exact target and scope.
pub(super) struct DeleteConfirmation {
    pub jid: String,
    pub message_id: String,
    pub for_everyone: bool,
    pub chat_name: String,
    pub message_preview: String,
    pub message_time: String,
}

/// A forward waiting for its destination. The source and id are kept together
/// because neither is available once the context menu that opened the picker
/// has gone.
pub(super) struct ForwardDraft {
    pub chat_jid: SharedString,
    pub message_id: SharedString,
}

impl WhatsAppApp {
    /// Open the chat picker for one message.
    ///
    /// Alongside the loaded conversations, ask the store for the address book:
    /// a person saved here but not yet a loaded chat is still a real forward
    /// destination, and upstream forwards by address, not by an existing chat.
    pub(super) fn begin_forward(
        &mut self,
        chat_jid: &str,
        message_id: &str,
        cx: &mut Context<Self>,
    ) {
        self.forward_picker = Some(ForwardDraft {
            chat_jid: chat_jid.into(),
            message_id: message_id.into(),
        });
        self.request_contacts(cx);
        cx.notify();
    }

    /// Ask for the first bounded page of the local address book. An empty
    /// query is a listing, not a search the reader has to type first; a hit in
    /// the book that is already a loaded chat is skipped when rows are built.
    fn request_contacts(&mut self, cx: &mut Context<Self>) {
        let Some(waiting) = self
            .client
            .as_ref()
            .map(|client| client.search_contacts("", super::saved_messages::LIST_LIMIT))
        else {
            return;
        };
        // The rows arrive as `FromDaemon::ContactResults`, not in the ack.
        // Nothing needs saying here if the read failed: the picker still has
        // the loaded conversations, which were always honest targets.
        cx.spawn(async move |_entity, _cx| {
            let _ = waiting.await;
        })
        .detach();
    }

    /// Destinations for the forward picker: loaded chats first, then address-
    /// book rows the store knows that are not already in that list. Names come
    /// from whatever each source holds; neither is invented from the other.
    pub(super) fn forward_targets(&self) -> Vec<(SharedString, SharedString)> {
        let mut seen = std::collections::HashSet::new();
        let mut rows: Vec<(SharedString, SharedString)> = Vec::new();
        for chat in &self.chats {
            let jid: SharedString = chat.jid.clone().into();
            seen.insert(jid.to_string());
            rows.push((chat.name.clone().into(), jid));
        }
        for hit in &self.contact_hits {
            if !seen.insert(hit.jid.clone()) {
                continue;
            }
            let name = hit
                .name
                .as_ref()
                .or(hit.alias.as_ref())
                .or(hit.push_name.as_ref())
                .or(hit.phone.as_ref())
                .unwrap_or(&hit.jid);
            rows.push((name.clone().into(), hit.jid.clone().into()));
        }
        rows
    }

    pub(super) fn cancel_forward(&mut self, cx: &mut Context<Self>) -> bool {
        if self.forward_picker.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// Ask the daemon to relay one stored message to the picked chat.
    pub(super) fn forward_to(&mut self, target: &str, cx: &mut Context<Self>) {
        let Some(draft) = self.forward_picker.take() else {
            return;
        };
        cx.notify();
        let Some(waiting) = self
            .client
            .as_ref()
            .map(|client| client.forward_message(&draft.chat_jid, &draft.message_id, target))
        else {
            self.notify_user(
                what_went_wrong(
                    crate::tr!(CouldNotForwardMessage),
                    &Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                ),
                notices::Tone::Problem,
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
            let _ = entity.update(cx, |app, cx| {
                if let Some(failure) = failure {
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotForwardMessage), &failure),
                        notices::Tone::Problem,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Mark a message optimistically and ask the account to make it true.
    ///
    /// The row already carries the durable star from hydration. This overlay
    /// only records changes since the window connected, so an explicit false
    /// can beat a hydrated true when the reader removes a star after reload.
    /// A refusal takes the mark back and says why.
    pub(super) fn toggle_starred(
        &mut self,
        chat_jid: &str,
        message_id: &str,
        starred: bool,
        cx: &mut Context<Self>,
    ) {
        let chat_jid = chat_jid.to_string();
        let key = (chat_jid.clone(), message_id.to_string());
        self.starred_messages.insert(key.clone(), starred);
        if !starred {
            self.saved_messages
                .retain(|hit| hit.chat_jid != chat_jid || hit.message_id != message_id);
        }
        self.invalidate_message_cache(&chat_jid, cx);
        cx.notify();
        let Some(waiting) = self
            .client
            .as_ref()
            .map(|client| client.set_starred(&chat_jid, message_id, starred))
        else {
            self.starred_messages.insert(key, !starred);
            self.invalidate_message_cache(&chat_jid, cx);
            self.notify_user(
                what_went_wrong(
                    crate::tr!(CouldNotStarMessage),
                    &Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                ),
                notices::Tone::Problem,
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
            let _ = entity.update(cx, |app, cx| {
                if let Some(failure) = failure {
                    app.starred_messages.insert(key, !starred);
                    app.invalidate_message_cache(&chat_jid, cx);
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotStarMessage), &failure),
                        notices::Tone::Problem,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Apply a star change this window did not necessarily ask for.
    pub(super) fn adopt_star_changed(
        &mut self,
        chat_jid: &str,
        message_id: &str,
        starred: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (chat_jid.to_string(), message_id.to_string());
        let changed = self.starred_messages.insert(key, starred) != Some(starred);
        let rows_before = self.saved_messages.len();
        if !starred {
            // The listing can still hold a row even when this window never
            // drew the star: a front end that opened after the star was set
            // has the rows but no overlay bit to flip.
            self.saved_messages
                .retain(|hit| hit.chat_jid != chat_jid || hit.message_id != message_id);
        }
        if changed {
            self.invalidate_message_cache(chat_jid, cx);
        }
        if changed || self.saved_messages.len() != rows_before {
            cx.notify();
        }
    }

    /// Whether a row should draw its star, asked while the timeline is built.
    ///
    /// The hydrated flag is the fallback; an explicit overlay value, including
    /// false, wins so a recent change beats the row loaded before it.
    pub(crate) fn is_starred(&self, chat_jid: &str, message_id: &str, from_store: bool) -> bool {
        self.starred_messages
            .get(&(chat_jid.to_string(), message_id.to_string()))
            .copied()
            .unwrap_or(from_store)
    }

    /// Mark a message optimistically and ask the account to pin it.
    ///
    /// The library's longest pin is 30 days, so the label and the request both
    /// say "pin" rather than promising a permanent place. Like a star, the row
    /// already carries the durable pin from hydration and this overlay only
    /// holds changes seen since. A refusal takes the mark back and says why.
    pub(super) fn toggle_pinned(
        &mut self,
        chat_jid: &str,
        message_id: &str,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        let chat_jid = chat_jid.to_string();
        let key = (chat_jid.clone(), message_id.to_string());
        self.pinned_messages.insert(key.clone(), pinned);
        if !pinned {
            // The panel reads store rows now, not this overlay. An unpin must
            // drop the row here too or it sits there after the mutation.
            self.pinned_results
                .retain(|hit| hit.chat_jid != chat_jid || hit.message_id != message_id);
        }
        self.invalidate_message_cache(&chat_jid, cx);
        cx.notify();
        let Some(waiting) = self
            .client
            .as_ref()
            .map(|client| client.set_message_pinned(&chat_jid, message_id, pinned))
        else {
            self.pinned_messages.insert(key, !pinned);
            self.invalidate_message_cache(&chat_jid, cx);
            self.notify_user(
                what_went_wrong(
                    crate::tr!(CouldNotPinMessage),
                    &Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                ),
                notices::Tone::Problem,
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
            let _ = entity.update(cx, |app, cx| {
                if let Some(failure) = failure {
                    app.pinned_messages.insert(key, !pinned);
                    app.invalidate_message_cache(&chat_jid, cx);
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotPinMessage), &failure),
                        notices::Tone::Problem,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Apply a pin change this window did not necessarily ask for.
    pub(super) fn adopt_pin_changed(
        &mut self,
        chat_jid: &str,
        message_id: &str,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (chat_jid.to_string(), message_id.to_string());
        let changed = self.pinned_messages.insert(key, pinned) != Some(pinned);
        let rows_before = self.pinned_results.len();
        if !pinned {
            // A front end can unpin a row this window never had an overlay for.
            // The listing is the panel's source now, so drop it there too.
            self.pinned_results
                .retain(|hit| hit.chat_jid != chat_jid || hit.message_id != message_id);
        }
        if changed {
            self.invalidate_message_cache(chat_jid, cx);
        }
        if changed || self.pinned_results.len() != rows_before {
            cx.notify();
        }
    }

    /// Whether a row should draw its pin, asked while the timeline is built.
    /// An explicit overlay value, including false, beats the hydrated row.
    pub(crate) fn is_pinned(&self, chat_jid: &str, message_id: &str, from_store: bool) -> bool {
        self.pinned_messages
            .get(&(chat_jid.to_string(), message_id.to_string()))
            .copied()
            .unwrap_or(from_store)
    }

    /// Keep the saved panel's rows after a store-wide listing.
    ///
    /// The daemon broadcasts this answer without a request id, so there is no
    /// query to compare against: the newest listing replaces the old rows.
    pub(super) fn adopt_starred_results(
        &mut self,
        results: Vec<oxidezap_ipc::MessageSearchHit>,
        cx: &mut Context<Self>,
    ) {
        let mut results = results;
        results.sort_unstable_by_key(|hit| std::cmp::Reverse(hit.timestamp_ms));
        results.truncate(super::saved_messages::LIST_LIMIT);
        self.saved_messages = results;
        cx.notify();
    }

    /// Keep the pinned panel's rows after a store-wide listing.
    ///
    /// The answer is broadcast without a request id, so the newest listing
    /// replaces the old rows exactly as a starred listing does.
    pub(super) fn adopt_pinned_results(
        &mut self,
        results: Vec<oxidezap_ipc::MessageSearchHit>,
        cx: &mut Context<Self>,
    ) {
        let mut results = results;
        results.sort_unstable_by_key(|hit| std::cmp::Reverse(hit.timestamp_ms));
        results.truncate(super::saved_messages::LIST_LIMIT);
        self.pinned_results = results;
        cx.notify();
    }

    /// Keep the address-book rows from the latest local contact search.
    ///
    /// Broadcast like the other listing answers, with no request id to compare
    /// against, so a newer answer replaces the older one wholesale.
    pub(super) fn adopt_contact_results(
        &mut self,
        results: Vec<oxidezap_ipc::ContactHit>,
        cx: &mut Context<Self>,
    ) {
        self.contact_hits = results;
        cx.notify();
    }

    /// Keep the media panel's rows after a store-wide listing.
    ///
    /// The answer is broadcast without a request id, so the newest listing
    /// replaces the old rows exactly as a starred listing does.
    pub(super) fn adopt_media_results(
        &mut self,
        results: Vec<oxidezap_ipc::MessageSearchHit>,
        cx: &mut Context<Self>,
    ) {
        let mut results = results;
        results.sort_unstable_by_key(|hit| std::cmp::Reverse(hit.timestamp_ms));
        results.truncate(super::saved_messages::LIST_LIMIT);
        self.media_messages = results;
        cx.notify();
    }
}

/// The native chat picker a forward opens. It lists loaded conversations and
/// address-book rows from the local store, deduplicated by address: no server
/// directory, and no fabricated destination.
pub(super) fn render_forward_picker(
    focus: &FocusHandle,
    chats: Vec<(SharedString, SharedString)>,
    app: &Entity<WhatsAppApp>,
    cx: &App,
) -> impl IntoElement + use<> {
    use gpui_component::IconName;

    let metrics = cx.product().metrics;
    let cancel_app = app.clone();
    let rows: Vec<_> = chats
        .into_iter()
        .map(|(name, jid)| {
            let target_app = app.clone();
            let target = jid.clone();
            let jid_label = jid.clone();
            div()
                .id(format!("forward-target-{jid}"))
                .debug_selector(|| "forward-target".into())
                .w_full()
                .cursor_pointer()
                .rounded(metrics.radius_md())
                .px(metrics.space_md())
                .py(metrics.space_sm())
                .hover(|el| el.bg(cx.theme().list_hover))
                .on_click(move |_, _, cx| {
                    target_app.update(cx, |app, cx| app.forward_to(&target, cx));
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(metrics.space_sm())
                        .child(
                            div()
                                .text_size(metrics.text_strong())
                                .text_color(cx.theme().foreground)
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(metrics.text_micro())
                                .text_color(cx.theme().muted_foreground)
                                .child(jid_label),
                        ),
                )
        })
        .collect();
    div()
        .id("forward-picker")
        .debug_selector(|| "forward-picker".into())
        .track_focus(focus)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(metrics.space_xxl())
        .child(
            div()
                .w(metrics.reading_width())
                .max_w_full()
                .max_h(metrics.config_block_height())
                .overflow_hidden()
                .rounded(metrics.radius_xl())
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .p(metrics.space_lg())
                .flex()
                .flex_col()
                .gap(metrics.space_md())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(metrics.space_sm())
                        .child(
                            div()
                                .text_size(metrics.text_heading())
                                .text_color(cx.theme().foreground)
                                .child(crate::tr!(ChooseAChat)),
                        )
                        .child(
                            Button::new("forward-picker-close")
                                .icon(IconName::Close)
                                .ghost()
                                .tooltip(crate::tr!(Close))
                                .on_click(move |_, _, cx| {
                                    cancel_app.update(cx, |app, cx| {
                                        app.cancel_forward(cx);
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .flex()
                        .flex_col()
                        .gap(metrics.space_xs())
                        .children(if rows.is_empty() {
                            vec![
                                div()
                                    .text_size(metrics.text_secondary())
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!(NoChatsAvailable))
                                    .into_any_element(),
                            ]
                        } else {
                            rows.into_iter().map(|row| row.into_any_element()).collect()
                        }),
                ),
        )
}

pub(crate) fn can_delete_sent(message: &ChatMessage, for_everyone: bool, now_ms: i64) -> bool {
    message.is_from_me
        && !message.revoked
        && message.system.is_none()
        && message.status.has_left_this_device()
        && (!for_everyone
            || now_ms.saturating_sub(message.timestamp.timestamp_millis())
                <= 2 * 24 * 60 * 60 * 1_000)
}

impl WhatsAppApp {
    pub(super) fn delete_sent_message(
        &mut self,
        jid: &str,
        message_id: &str,
        for_everyone: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (jid.to_owned(), message_id.to_owned());
        if self.pending_message_actions.contains(&key) || self.pending_edits.contains(&key) {
            return;
        }
        let eligible = self
            .find_chat(jid)
            .filter(|_| self.selected_chat.as_deref() == Some(jid))
            .and_then(|chat| {
                chat.messages
                    .iter()
                    .find(|message| message.id == message_id)
            })
            .is_some_and(|message| {
                can_delete_sent(message, for_everyone, wacore::time::now_millis())
            });
        if !self.can_send() || !eligible {
            self.notify_user(
                crate::tr!(ThisMessageCanNoLongerBeDeleted),
                notices::Tone::Problem,
                cx,
            );
            return;
        }
        #[cfg(test)]
        self.delete_attempts
            .push((jid.to_owned(), message_id.to_owned(), for_everyone));
        self.pending_message_actions.insert(key.clone());
        let generation = self.edit_generation;
        let Some(client) = self.client.as_ref() else {
            self.pending_message_actions.remove(&key);
            self.notify_user(crate::tr!(DeleteUnavailable), notices::Tone::Problem, cx);
            return;
        };
        let answer = client.revoke_message(jid.to_owned(), message_id.to_owned(), for_everyone);
        cx.spawn(async move |entity: WeakEntity<Self>, cx| {
            let result = answer.await;
            let _ = entity.update(cx, |app, cx| {
                if app.edit_generation != generation {
                    return;
                }
                app.pending_message_actions.remove(&key);
                match result {
                    Ok(Ok(())) => {
                        if let Some(chat) = app.find_chat_mut(&key.0) {
                            if for_everyone {
                                let newest = chat.messages.last().is_some_and(|m| m.id == key.1);
                                if let Some(message) =
                                    chat.messages.iter_mut().find(|m| m.id == key.1)
                                {
                                    message.mark_revoked();
                                    if newest {
                                        chat.last_message = Some("[Message deleted]".to_owned());
                                    }
                                }
                            } else {
                                chat.remove_message_for_me(&key.1);
                            }
                        }
                        app.invalidate_message_cache(&key.0, cx);
                        app.invalidate_chat_cache();
                        cx.notify();
                    }
                    Ok(Err(failure)) => app.notify_user(
                        what_went_wrong(crate::tr!(DeleteFailed), &failure),
                        notices::Tone::Problem,
                        cx,
                    ),
                    Err(_) => {
                        app.notify_user(crate::tr!(DeleteDisconnected), notices::Tone::Problem, cx)
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn begin_message_delete(
        &mut self,
        jid: &str,
        message_id: &str,
        for_everyone: bool,
        cx: &mut Context<Self>,
    ) {
        if self.delete_confirmation.is_some()
            || self.edit_draft.is_some()
            || self.paste_preview.is_some()
            || self.event_dialog.is_some()
            || self.channel_poll_dialog.is_some()
            || self.community_dialog.is_some()
        {
            return;
        }
        let Some(chat) = self
            .find_chat(jid)
            .filter(|_| self.selected_chat.as_deref() == Some(jid))
        else {
            return;
        };
        let Some(message) = chat
            .messages
            .iter()
            .find(|message| message.id == message_id)
        else {
            return;
        };
        if !self.can_send()
            || !can_delete_sent(message, for_everyone, wacore::time::now_millis())
            || self
                .pending_message_actions
                .contains(&(jid.to_owned(), message_id.to_owned()))
            || self
                .pending_edits
                .contains(&(jid.to_owned(), message_id.to_owned()))
        {
            self.notify_user(
                "This message can no longer be deleted.",
                notices::Tone::Problem,
                cx,
            );
            return;
        }
        let preview = message.preview_text();
        let message_preview: String = preview.chars().take(120).collect();
        self.delete_confirmation = Some(DeleteConfirmation {
            jid: jid.to_owned(),
            message_id: message_id.to_owned(),
            for_everyone,
            chat_name: if chat.name.trim().is_empty() {
                jid.to_owned()
            } else {
                chat.name.clone()
            },
            message_preview,
            message_time: crate::utils::format_time_local(&message.timestamp),
        });
        cx.notify();
    }

    pub(super) fn cancel_message_delete(&mut self, cx: &mut Context<Self>) -> bool {
        if self.delete_confirmation.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    pub(super) fn confirm_message_delete(&mut self, cx: &mut Context<Self>) {
        let Some(confirmation) = self.delete_confirmation.take() else {
            return;
        };
        cx.notify();
        self.delete_sent_message(
            &confirmation.jid,
            &confirmation.message_id,
            confirmation.for_everyone,
            cx,
        );
    }
}

pub(super) fn render_message_delete(
    confirmation: &DeleteConfirmation,
    app: Entity<WhatsAppApp>,
    focus: &FocusHandle,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let scope = if confirmation.for_everyone {
        crate::tr!(DeleteForEveryone)
    } else {
        crate::tr!(DeleteForMe)
    };
    let chat_name = &confirmation.chat_name;
    let preview = &confirmation.message_preview;
    let message_time = &confirmation.message_time;
    let cancel = app.clone();
    div()
        .id("message-delete-modal")
        .debug_selector(|| "message-delete-modal".into())
        .track_focus(focus)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(metrics.space_xxl())
        .bg(crate::components::parts::scrim(cx).opacity(0.92))
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w_full()
                .max_w(metrics.bubble_max_width())
                .p(metrics.space_xxl())
                .rounded(metrics.radius_lg())
                .bg(cx.theme().background)
                .flex()
                .flex_col()
                .gap(metrics.space_lg())
                .child(
                    div()
                        .id("message-delete-scope")
                        .debug_selector(|| "message-delete-scope".into())
                        .child(scope),
                )
                .child(
                    div()
                        .id("message-delete-chat")
                        .debug_selector(|| "message-delete-chat".into())
                        .child(crate::tr!(conversation, chat_name)),
                )
                .child(
                    div()
                        .id("message-delete-preview")
                        .debug_selector(|| "message-delete-preview".into())
                        .child(crate::tr!(message_preview, preview)),
                )
                .child(
                    div()
                        .id("message-delete-time")
                        .debug_selector(|| "message-delete-time".into())
                        .child(crate::tr!(message_time, message_time)),
                )
                .child(crate::tr!(ConfirmDeletionOfThisMessage))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(metrics.space_md())
                        .child(
                            div()
                                .id("message-delete-cancel")
                                .debug_selector(|| "message-delete-cancel".into())
                                .child(
                                    Button::new("message-delete-cancel-button")
                                        .label(crate::tr!(Cancel))
                                        .on_click(move |_, _, cx| {
                                            cancel.update(cx, |app, cx| {
                                                app.cancel_message_delete(cx);
                                            });
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .id("message-delete-confirm")
                                .debug_selector(|| "message-delete-confirm".into())
                                .child(
                                    Button::new("message-delete-confirm-button")
                                        .label(scope)
                                        .danger()
                                        .on_click(move |_, _, cx| {
                                            app.update(cx, |app, cx| {
                                                app.confirm_message_delete(cx)
                                            });
                                        }),
                                ),
                        ),
                ),
        )
        .focus_trap("message-delete-trap", focus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn delete_fixture(
        cx: &mut gpui::TestAppContext,
    ) -> (gpui::VisualTestContext, Entity<WhatsAppApp>) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
            init_app_bindings(cx);
        });
        let mut app_entity = None;
        let window = cx.open_window(gpui::size(gpui::px(1000.), gpui::px(800.)), |window, cx| {
            let app = cx.new(|cx| {
                let mut app = WhatsAppApp::new(cx);
                app.app_state = AppState::Connected;
                app.destination = Destination::Chats;
                let mut chat = Chat::new("peer@example.invalid".into());
                chat.name = "Test chat".into();
                chat.add_message(sent(wacore::time::now_millis()));
                let mut second = sent(wacore::time::now_millis());
                second.id = "SENT-2".into();
                second.content = "second message".into();
                chat.add_message(second);
                app.chats.push(Arc::new(chat));
                let mut other = Chat::new("other@example.invalid".into());
                other.add_message(sent(wacore::time::now_millis()));
                app.chats.push(Arc::new(other));
                app
            });
            app_entity = Some(app.clone());
            gpui_component::Root::new(app, window, cx)
        });
        let app = app_entity.unwrap();
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            app.update(cx, |app, cx| {
                app.select_chat(
                    "peer@example.invalid".into(),
                    ChatOpen::ToCompose,
                    window,
                    cx,
                );
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (cx, app)
    }

    fn sent(now_ms: i64) -> ChatMessage {
        let mut message = ChatMessage::new_outgoing("SENT-1".into(), "hello".into());
        message.timestamp = chrono::Utc.timestamp_millis_opt(now_ms).unwrap();
        message.status = MessageStatus::Sent;
        message
    }

    /// The overlay is deliberately tri-state: an absent key falls back to the
    /// hydrated row, while an explicit false beats a hydrated true. Without
    /// that, removing a star after reload would keep drawing it.
    #[gpui::test]
    fn star_and_pin_overlays_beat_the_hydrated_row(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = delete_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                // Nothing seen since connect: the durable row answers.
                let chat = "peer@example.invalid";
                assert!(!app.is_starred(chat, "SENT-1", false));
                assert!(app.is_starred(chat, "SENT-1", true));
                assert!(!app.is_pinned(chat, "SENT-1", false));
                assert!(app.is_pinned(chat, "SENT-1", true));

                // An observed change wins in both directions, false included.
                app.adopt_star_changed(chat, "SENT-1", false, cx);
                assert!(!app.is_starred(chat, "SENT-1", true));
                app.adopt_star_changed(chat, "SENT-1", true, cx);
                assert!(app.is_starred(chat, "SENT-1", false));

                app.adopt_pin_changed(chat, "SENT-1", false, cx);
                assert!(!app.is_pinned(chat, "SENT-1", true));
                app.adopt_pin_changed(chat, "SENT-1", true, cx);
                assert!(app.is_pinned(chat, "SENT-1", false));
            });
        });
    }

    #[gpui::test]
    fn delete_confirmation_identifies_target_and_cancel_never_attempts_revoke(
        cx: &mut gpui::TestAppContext,
    ) {
        let (mut cx, app) = delete_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.begin_message_delete("peer@example.invalid", "SENT-2", true, cx);
                assert_eq!(app.delete_attempts.len(), 0);
                assert_eq!(
                    app.find_chat("peer@example.invalid")
                        .unwrap()
                        .messages
                        .len(),
                    2
                );
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("message-delete-modal").is_some());
        assert!(cx.debug_bounds("message-delete-scope").is_some());
        assert!(cx.debug_bounds("message-delete-chat").is_some());
        assert!(cx.debug_bounds("message-delete-preview").is_some());
        assert!(cx.debug_bounds("message-delete-time").is_some());
        cx.read(|cx| {
            let app = app.read(cx);
            let confirmation = app.delete_confirmation.as_ref().unwrap();
            assert_eq!(confirmation.jid, "peer@example.invalid");
            assert_eq!(confirmation.message_id, "SENT-2");
            assert!(confirmation.for_everyone);
            assert_eq!(confirmation.chat_name, "Test chat");
            assert_eq!(confirmation.message_preview, "second message");
            assert_eq!(
                confirmation.message_time,
                crate::utils::format_time_local(
                    &app.find_chat("peer@example.invalid").unwrap().messages[1].timestamp
                )
            );
            assert_eq!(app.keyboard_owner, Some(KeyboardOwner::MessageDelete));
        });
        let cancel = cx.debug_bounds("message-delete-cancel").unwrap();
        cx.simulate_click(cancel.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(app.delete_confirmation.is_none());
            assert!(app.delete_attempts.is_empty());
            assert_eq!(
                app.find_chat("peer@example.invalid").unwrap().messages[1].content,
                "second message"
            );
        });
    }

    #[gpui::test]
    fn delete_confirmation_escape_and_chat_switch_send_nothing(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = delete_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.begin_message_delete("peer@example.invalid", "SENT-1", false, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(app.delete_confirmation.is_none());
            assert!(app.delete_attempts.is_empty());
            assert_eq!(app.keyboard_owner, Some(KeyboardOwner::Composer));
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_message_delete("peer@example.invalid", "SENT-1", false, cx);
                app.select_chat(
                    "other@example.invalid".into(),
                    ChatOpen::ToCompose,
                    window,
                    cx,
                );
                app.confirm_message_delete(cx);
                assert!(app.delete_confirmation.is_none());
                assert!(app.delete_attempts.is_empty());
            });
        });
    }

    #[gpui::test]
    fn delete_confirmation_submits_each_scope_once_to_exact_message(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = delete_fixture(cx);
        for (id, for_everyone) in [("SENT-1", false), ("SENT-2", true)] {
            cx.update(|_window, cx| {
                app.update(cx, |app, cx| {
                    app.begin_message_delete("peer@example.invalid", id, for_everyone, cx);
                    assert!(app.delete_attempts.len() <= 1);
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let confirm = cx.debug_bounds("message-delete-confirm").unwrap();
            cx.simulate_click(confirm.center(), gpui::Modifiers::default());
            cx.run_until_parked();
            cx.simulate_click(confirm.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        }
        cx.read(|cx| {
            let app = app.read(cx);
            assert_eq!(
                app.delete_attempts,
                vec![
                    ("peer@example.invalid".into(), "SENT-1".into(), false),
                    ("peer@example.invalid".into(), "SENT-2".into(), true),
                ]
            );
            assert_eq!(
                app.find_chat("peer@example.invalid")
                    .unwrap()
                    .messages
                    .len(),
                2
            );
            assert!(app.notices().read(cx).has_problem("daemon is unavailable"));
        });
    }

    #[gpui::test]
    fn stale_or_busy_delete_confirmation_cannot_attempt_revoke(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = delete_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.begin_message_delete("peer@example.invalid", "SENT-1", true, cx);
                // A second action cannot replace the first modal's target.
                app.begin_message_delete("peer@example.invalid", "SENT-2", false, cx);
                assert_eq!(
                    app.delete_confirmation.as_ref().unwrap().message_id,
                    "SENT-1"
                );
                app.pending_message_actions
                    .insert(("peer@example.invalid".into(), "SENT-1".into()));
                app.confirm_message_delete(cx);
                app.confirm_message_delete(cx);
                assert!(app.delete_attempts.is_empty());
                assert!(app.delete_confirmation.is_none());
            });
        });
    }

    #[gpui::test]
    fn deleted_or_expired_target_is_rechecked_after_confirmation_opens(
        cx: &mut gpui::TestAppContext,
    ) {
        let (mut cx, app) = delete_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.begin_message_delete("peer@example.invalid", "SENT-1", true, cx);
                app.find_chat_mut("peer@example.invalid")
                    .unwrap()
                    .messages
                    .retain(|message| message.id != "SENT-1");
                app.confirm_message_delete(cx);
                assert!(app.delete_attempts.is_empty());
                assert!(app.delete_confirmation.is_none());

                app.begin_message_delete("peer@example.invalid", "SENT-2", true, cx);
                app.find_chat_mut("peer@example.invalid")
                    .unwrap()
                    .messages
                    .iter_mut()
                    .find(|message| message.id == "SENT-2")
                    .unwrap()
                    .timestamp = chrono::Utc
                    .timestamp_millis_opt(wacore::time::now_millis() - 3 * 24 * 60 * 60 * 1_000)
                    .unwrap();
                app.confirm_message_delete(cx);
                assert!(app.delete_attempts.is_empty());
                assert_eq!(
                    app.find_chat("peer@example.invalid").unwrap().messages[0].content,
                    "second message"
                );
            });
        });
    }

    /// Pins are account state: an event about an unselected chat still marks
    /// its row, and an unpin from the account clears only that one message.
    #[gpui::test]
    fn pin_flags_follow_both_this_window_and_the_account(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = delete_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.adopt_pin_changed("peer@example.invalid", "SENT-1", true, cx);
                assert!(app.is_pinned("peer@example.invalid", "SENT-1", false));
                assert_eq!(app.pinned_messages.len(), 1);

                // A repeated broadcast cannot turn one pin into two.
                app.adopt_pin_changed("peer@example.invalid", "SENT-1", true, cx);
                assert_eq!(app.pinned_messages.len(), 1);

                app.adopt_pin_changed("other@example.invalid", "SENT-2", true, cx);
                assert!(app.is_pinned("other@example.invalid", "SENT-2", false));

                app.adopt_pin_changed("peer@example.invalid", "SENT-1", false, cx);
                assert!(!app.is_pinned("peer@example.invalid", "SENT-1", false));
                assert!(app.is_pinned("other@example.invalid", "SENT-2", false));
            });
        });
    }

    /// Without a daemon there is nobody to confirm the request, so the mark
    /// must not linger and the window must say why. The same rollback is used
    /// by a correlated refusal; both start from an optimistic mark.
    #[gpui::test]
    fn pin_without_a_daemon_rolls_back_and_explains(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = delete_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.toggle_pinned("peer@example.invalid", "SENT-1", true, cx);
                assert!(!app.is_pinned("peer@example.invalid", "SENT-1", false));
                assert!(
                    app.notices()
                        .read(cx)
                        .has_problem("Could not update the pin")
                );

                app.adopt_pin_changed("peer@example.invalid", "SENT-2", true, cx);
                app.toggle_pinned("peer@example.invalid", "SENT-2", false, cx);
                assert!(app.is_pinned("peer@example.invalid", "SENT-2", false));
            });
        });
    }

    /// The listing is account-wide and has no query to correlate against, so
    /// the newest answer replaces the old one; a later unstar drops its row.
    #[gpui::test]
    fn starred_results_are_newest_first_bounded_and_removed_with_a_star(
        cx: &mut gpui::TestAppContext,
    ) {
        let (mut cx, app) = delete_fixture(cx);
        let limit = super::super::saved_messages::LIST_LIMIT;
        let hits: Vec<_> = (0..limit + 3)
            .map(|n| oxidezap_ipc::MessageSearchHit {
                chat_jid: "peer@example.invalid".into(),
                message_id: format!("STAR-{n}"),
                snippet: "a synthetic saved message".into(),
                sender_jid: None,
                timestamp_ms: n as i64,
                has_media: false,
                ..Default::default()
            })
            .collect();
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.adopt_starred_results(hits, cx);
                assert_eq!(app.saved_messages.len(), limit);
                assert_eq!(
                    app.saved_messages[0].message_id,
                    format!("STAR-{}", limit + 2)
                );
                assert_eq!(app.saved_messages.last().unwrap().message_id, "STAR-3");

                app.adopt_starred_results(
                    vec![oxidezap_ipc::MessageSearchHit {
                        chat_jid: "peer@example.invalid".into(),
                        message_id: "STAR-9".into(),
                        snippet: "a newer listing".into(),
                        sender_jid: None,
                        timestamp_ms: i64::MAX,
                        has_media: false,
                        ..Default::default()
                    }],
                    cx,
                );
                assert_eq!(app.saved_messages.len(), 1);

                app.adopt_star_changed("peer@example.invalid", "STAR-9", false, cx);
                assert!(app.saved_messages.is_empty());
            });
        });
    }

    /// The pinned listing follows the same newest-first, bounded, newest-answer
    /// wins rules as the saved listing, and does not lose rows just because
    /// this window never loaded the chat they belong to.
    #[gpui::test]
    fn pinned_results_are_newest_first_bounded_and_replace_each_other(
        cx: &mut gpui::TestAppContext,
    ) {
        let (mut cx, app) = delete_fixture(cx);
        let limit = super::super::saved_messages::LIST_LIMIT;
        let hits: Vec<_> = (0..limit + 2)
            .map(|n| oxidezap_ipc::MessageSearchHit {
                chat_jid: "unloaded@example.invalid".into(),
                message_id: format!("PIN-{n}"),
                snippet: "a synthetic pinned message".into(),
                sender_jid: None,
                timestamp_ms: n as i64,
                has_media: false,
                ..Default::default()
            })
            .collect();
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.adopt_pinned_results(hits, cx);
                assert_eq!(app.pinned_results.len(), limit);
                assert_eq!(
                    app.pinned_results[0].message_id,
                    format!("PIN-{}", limit + 1)
                );
                assert_eq!(app.pinned_results.last().unwrap().message_id, "PIN-2");
                assert_eq!(app.pinned_rows().len(), limit, "no row needs a loaded chat");

                app.adopt_pinned_results(
                    vec![oxidezap_ipc::MessageSearchHit {
                        chat_jid: "unloaded@example.invalid".into(),
                        message_id: "PIN-NEW".into(),
                        snippet: "a newer listing".into(),
                        sender_jid: None,
                        timestamp_ms: i64::MAX,
                        has_media: false,
                        ..Default::default()
                    }],
                    cx,
                );
                assert_eq!(app.pinned_results.len(), 1);
                assert_eq!(app.pinned_rows()[0].message_id, "PIN-NEW");

                // Removing a pin drops the row even when this window had no
                // overlay bit for it. The overlay still beats the hydrated flag.
                app.adopt_pin_changed("unloaded@example.invalid", "PIN-NEW", false, cx);
                assert!(app.pinned_results.is_empty());
                assert!(!app.is_pinned("unloaded@example.invalid", "PIN-NEW", true));
            });
        });
    }

    /// A forward can target a saved person who is not a loaded conversation.
    /// Loaded chats come first, an address already in that list is not repeated,
    /// and a nameless address-book row is named by the address itself.
    #[gpui::test]
    fn forward_targets_include_unloaded_contacts_without_duplicates(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = delete_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, _| {
                app.contact_hits = vec![
                    oxidezap_ipc::ContactHit {
                        jid: "peer@example.invalid".into(),
                        name: Some("Duplicate".into()),
                        ..Default::default()
                    },
                    oxidezap_ipc::ContactHit {
                        jid: "saved@example.invalid".into(),
                        name: Some("Saved person".into()),
                        ..Default::default()
                    },
                    oxidezap_ipc::ContactHit {
                        jid: "unnamed@example.invalid".into(),
                        ..Default::default()
                    },
                ];
                let targets = app.forward_targets();
                assert!(targets.len() >= 4, "two chats plus two new contacts");
                let addresses: Vec<_> = targets.iter().map(|(_, jid)| jid.to_string()).collect();
                assert_eq!(
                    addresses
                        .iter()
                        .filter(|jid| **jid == "peer@example.invalid")
                        .count(),
                    1,
                    "a loaded chat is not listed twice"
                );
                let saved = targets
                    .iter()
                    .find(|(_, jid)| jid.as_ref() == "saved@example.invalid")
                    .expect("the saved contact is a target");
                assert_eq!(saved.0.as_ref(), "Saved person");
                let unnamed = targets
                    .iter()
                    .find(|(_, jid)| jid.as_ref() == "unnamed@example.invalid")
                    .expect("the nameless contact is a target");
                assert_eq!(unnamed.0.as_ref(), "unnamed@example.invalid");
            });
        });
    }
}
