//! Acting on a message: replying, retrying, reacting, jumping to a quote.

use super::*;
use gpui::InteractiveElement as _;
use gpui_component::ActiveTheme as _;
use gpui_component::Sizable as _;
use gpui_component::button::ButtonVariants as _;
use oxidezap_core::{ChannelReact, ChannelVotePoll};

/// [`Resend`] with the message let go of.
///
/// The answer borrows the chat and sending needs `self` mutably, so this is
/// the same two cases holding what they name.
enum Retry {
    Text(String),
    /// Boxed: a `MediaContent` is ten times the size of a `String`, and this
    /// value exists for one statement.
    VoiceNote(Box<MediaContent>),
}

impl WhatsAppApp {
    /// Focus target for the call card, so its actions are reachable from the
    /// keyboard while it floats.
    pub fn call_focus(&self) -> &FocusHandle {
        &self.call_focus
    }

    /// Scroll the timeline to a message, for the jump out of a quote.
    ///
    /// A quote is a snapshot, so the original may not be loaded — it can be
    /// older than the window, or deleted. Saying nothing is the honest
    /// outcome there; the quote still shows what was said.
    pub fn jump_to_message(&mut self, message_id: &str, cx: &mut Context<Self>) {
        let Some(chat) = self.selected_chat_data() else {
            return;
        };
        let Some(position) = chat.messages.iter().position(|m| m.id == message_id) else {
            debug!("quoted message {message_id} is outside the loaded window");
            return;
        };
        // Timeline coordinates, not message coordinates: dividers and the
        // typing row are items too, so the two indices diverge.
        let cache = self.message_list_cache.borrow();
        let item_ix = cache.get(&chat.jid).and_then(|cache| {
            cache.items.iter().position(
                |item| matches!(item, TimelineItem::Message { ix, .. } if *ix == position),
            )
        });
        drop(cache);

        if let Some(item_ix) = item_ix {
            self.message_list.scroll_to_reveal_item(item_ix);
            cx.notify();
        }
    }

    /// Start composing a reply to `message_id`.
    pub fn begin_reply(&mut self, message_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit_is_pending() {
            return;
        }
        self.cancel_message_edit(window, cx);
        let Some(chat) = self.selected_chat_data() else {
            return;
        };
        let Some(message) = chat.messages.iter().find(|m| m.id == message_id) else {
            return;
        };

        let sender_name = if message.is_from_me {
            self.account_name.clone().unwrap_or_default()
        } else {
            chat.author_name(message)
                .map(str::to_owned)
                .unwrap_or_else(|| chat.name.clone())
        };
        let preview = message.content.clone();

        // `sender` is sent as the quote's participant, so it has to be a JID.
        // A message this window composed carries the literal `"Me"` — that is
        // what `ChatMessage::new_outgoing` writes, and the optimistic row
        // survives the rename its id gets — so replying to something you had
        // only just sent quoted an author no group has.
        let sender = if message.is_from_me {
            self.account_jid
                .clone()
                .or_else(|| self.account_lid.clone())
                .unwrap_or_else(|| message.sender.clone())
        } else {
            message.sender.clone()
        };

        let draft = ReplyDraft {
            message_id: message_id.to_string(),
            sender,
            sender_name,
            sender_is_self: message.is_from_me,
            preview,
            kind: message
                .media
                .as_ref()
                .and_then(|media| oxidezap_core::QuotedKind::of(&media.media_type)),
        };

        self.reply_to = Some(draft.clone());
        if let Some(input) = &self.input_area {
            input.update(cx, |view, cx| view.set_reply(Some(draft), cx));
        }
        // Replying is a composing gesture: put the caret where the user is
        // about to type rather than making them click into the field.
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Drop the reply being composed.
    pub fn cancel_reply(&mut self, cx: &mut Context<Self>) {
        self.reply_to = None;
        if let Some(input) = &self.input_area {
            input.update(cx, |view, cx| view.set_reply(None, cx));
        }
        cx.notify();
    }

    /// Send a failed message again.
    ///
    /// A retry is a fresh send, not a resurrection: the original keeps its
    /// failed state and its place in the timeline, because that is what
    /// happened. Re-sending the text is the recovery the user asked for.
    pub fn retry_send(&mut self, message_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        // Same rule as the composer: in the offline state the history is
        // readable and nothing else. The timeline is still drawn there, so
        // the retry under a failed bubble was a live send out of a window
        // that says it is not connected.
        if !self.can_send() {
            warn!("Cannot send again: this window is offline");
            return;
        }
        let Some(chat) = self.selected_chat_data() else {
            return;
        };
        let Some(message) = chat.messages.iter().find(|m| m.id == message_id) else {
            return;
        };
        // What the bubble asked before drawing the button, asked again here
        // so the two cannot disagree about what a retry can do.
        let Some(again) = message.resend() else {
            return;
        };
        // A reply that failed is retried as a reply. The draft that produced
        // it was consumed by the first send, so the quote has to come from
        // the message itself.
        let quoted = message.quoted.clone();
        // Lifted off the chat before sending, which needs `self` mutably.
        let again = match again {
            Resend::Text(text) => Retry::Text(text.to_owned()),
            Resend::VoiceNote(media) => Retry::VoiceNote(Box::new(media.clone())),
        };
        let jid = chat.jid.clone();
        let _ = window;

        match again {
            Retry::Text(content) => self.send_quoted(&content, quoted, Vec::new(), cx),
            Retry::VoiceNote(media) => {
                self.send_voice_note(
                    cx,
                    &jid,
                    (*media.data).clone(),
                    media.waveform.as_deref().cloned().unwrap_or_default(),
                    media.duration_secs.unwrap_or(0),
                    quoted,
                );
                cx.notify();
            }
        }
    }

    /// The emojis the quick-react strip offers, in the order it offers them.
    /// Vote on a poll option.
    ///
    /// One tap replaces the single-choice ballot. Multi-select voting is
    /// disabled until the daemon owns the authoritative ballot: another tab
    /// may already have selected options that this window cannot see, and
    /// sending its local vector would silently replace those selections.
    /// Offline the daemon would refuse,
    /// so the tap is refused here where the window can say why. The tap draws
    /// an attempted ballot, not a confirmed one: the daemon's answer is
    /// correlated to this request now, so a refusal takes the mark back and
    /// says why rather than leaving a vote that never reached WhatsApp.
    pub fn vote_poll(
        &mut self,
        chat_jid: &str,
        message_id: &str,
        option_index: u32,
        cx: &mut Context<Self>,
    ) {
        if !self.can_send() {
            warn!("Cannot vote: this window is offline");
            return;
        }
        // Read the limit off the poll itself: no local ballot is authoritative.
        let limit = self
            .find_chat(chat_jid)
            .and_then(|chat| chat.messages.iter().find(|m| m.id == message_id))
            .and_then(|message| message.poll.as_ref())
            .map_or(1, |poll| poll.selectable_count.max(1));
        // Without the daemon's current ballot, a second tab cannot know
        // whether this vote would replace selections made in the first.
        // Refuse multi-select until authoritative ballots are available.
        if limit > 1 {
            warn!("Multiple-choice voting needs synchronized ballots");
            return;
        }
        if let Some(client) = self.client() {
            // A channel poll is not an E2E poll: its ballot names the server's
            // content id and hashes the option text stored with the message.
            let waiting = if is_channel_jid(chat_jid) {
                client.vote_channel_poll(ChannelVotePoll {
                    channel_jid: chat_jid.to_string(),
                    poll_message_id: message_id.to_string(),
                    option_indices: vec![option_index],
                })
            } else {
                client.vote_poll(chat_jid, message_id, vec![option_index])
            };
            let key = (chat_jid.to_string(), message_id.to_string());
            self.attempted_poll_votes.insert(key.clone(), option_index);
            self.invalidate_message_cache(chat_jid, cx);
            cx.notify();
            cx.spawn(async move |entity, cx| {
                let failure = match waiting.await {
                    Ok(Ok(())) => None,
                    Ok(Err(failure)) => Some(failure),
                    Err(_) => Some(crate::session::Failure::worth_retrying(crate::tr!(
                        DaemonDisconnectedBeforeAnswer
                    ))),
                };
                if let Some(failure) = failure {
                    let _ = entity.update(cx, |app, cx| {
                        app.attempted_poll_votes.remove(&key);
                        app.invalidate_message_cache(&key.0, cx);
                        app.notify_user(
                            what_went_wrong(crate::tr!(CouldNotRecordVote), &failure),
                            notices::Tone::Problem,
                            cx,
                        );
                    });
                }
            })
            .detach();
        }
    }

    /// An attempt, not a confirmed selection. The bubble labels it as such.
    pub fn attempted_poll_vote(&self, chat_jid: &str, message_id: &str) -> Option<u32> {
        self.attempted_poll_votes
            .get(&(chat_jid.to_string(), message_id.to_string()))
            .copied()
    }

    /// Answer an event invitation from its card.
    ///
    /// The card paints the attempt before the answer, so a refusal has to
    /// arrive on this same tap: it takes the mark back and says why. A sent
    /// answer stays marked as requested because this window never learns the
    /// ballots of other devices.
    pub fn respond_event(
        &mut self,
        chat_jid: &str,
        message_id: &str,
        response: EventRsvp,
        cx: &mut Context<Self>,
    ) {
        if !self.can_send() {
            warn!("Cannot answer event: this window is offline");
            return;
        }
        let Some(client) = self.client() else {
            return;
        };
        let waiting = client.respond_event(chat_jid, message_id, response, None);
        let key = (chat_jid.to_string(), message_id.to_string());
        self.attempted_event_rsvps.insert(key.clone(), response);
        self.invalidate_message_cache(chat_jid, cx);
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let failure = match waiting.await {
                Ok(Ok(())) => None,
                Ok(Err(failure)) => Some(failure),
                Err(_) => Some(crate::session::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            if let Some(failure) = failure {
                let _ = entity.update(cx, |app, cx| {
                    app.attempted_event_rsvps.remove(&key);
                    app.invalidate_message_cache(&key.0, cx);
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotRecordRsvp), &failure),
                        notices::Tone::Problem,
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    /// The answer this window asked for, not one the network confirmed.
    pub fn attempted_event_rsvp(&self, chat_jid: &str, message_id: &str) -> Option<EventRsvp> {
        self.attempted_event_rsvps
            .get(&(chat_jid.to_string(), message_id.to_string()))
            .copied()
    }

    /// Fixed rather than the full emoji table: this is a reaction control,
    /// not an emoji picker, and anything typed beyond these travels the
    /// composer's own path. Kept here rather than in the bubble so the strip
    /// and the context menu cannot disagree about what a tap sends.
    pub const QUICK_REACTIONS: [&'static str; 6] = ["👍", "❤️", "😂", "😮", "😢", "🙏"];

    /// Open the quick-react strip under one message, or close it when it is
    /// already that message's.
    pub fn toggle_reaction_picker(&mut self, message_id: &str, cx: &mut Context<Self>) {
        let open = self.reaction_picker_for.as_deref() == Some(message_id);
        self.reaction_picker_for = (!open).then(|| message_id.to_string());
        self.reaction_picker_full_target = None;
        // The strip moves every row below it, so the cached measurements the
        // list laid out against are laid out against a timeline without it.
        if let Some(jid) = self.selected_chat_jid() {
            self.invalidate_message_cache(&jid, cx);
        }
        cx.notify();
    }

    /// Build the full picker on first use. Its search field needs a window,
    /// which the application constructor does not have, so this is called
    /// from the click that opens the overlay instead.
    fn ensure_reaction_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reaction_emoji_picker.is_some() {
            return;
        }
        let picker = cx.new(|cx| EmojiPicker::new(window, cx));
        cx.subscribe(&picker, |this, _, event: &EmojiPicked, cx| {
            // A target means the overlay is still open. Picking sends through
            // the same path as the quick strip, then closes the overlay.
            let Some(target) = this.reaction_picker_full_target.take() else {
                return;
            };
            this.toggle_reaction(&target, event.0, cx);
        })
        .detach();
        self.reaction_emoji_picker = Some(picker);
    }

    /// Replace the six-button strip with the searchable emoji table for one
    /// message. The target is captured here, not read from the picker, so a
    /// pick can never land on whichever bubble happens to be selected later.
    pub fn open_full_reaction_picker(
        &mut self,
        message_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_reaction_picker(window, cx);
        self.reaction_picker_for = None;
        self.reaction_picker_full_target = Some(message_id.to_string());
        if let Some(jid) = self.selected_chat_jid() {
            self.invalidate_message_cache(&jid, cx);
        }
        cx.notify();
    }

    pub fn close_full_reaction_picker(&mut self, cx: &mut Context<Self>) -> bool {
        if self.reaction_picker_full_target.take().is_some() {
            self.reaction_picker_for = None;
            cx.notify();
            true
        } else {
            false
        }
    }

    pub fn full_reaction_target(&self) -> Option<&str> {
        self.reaction_picker_full_target.as_deref()
    }

    /// Send `emoji` as our reaction to `message_id`, or take ours back when
    /// it names the reaction already drawn as ours.
    ///
    /// Painted optimistically: the row is updated before the daemon answers,
    /// and the network echo arriving as `ReactionReceived` confirms it. Our
    /// own sender is the LID first, which is the canonical form the echo
    /// carries — stamping the phone number beside it would draw one person
    /// as two until the next history load.
    pub fn toggle_reaction(&mut self, message_id: &str, emoji: &str, cx: &mut Context<Self>) {
        // Same rule as the composer: in the offline state the history is
        // readable and nothing else, and a tap that silently went nowhere
        // would leave the optimistic row as a lie.
        if !self.can_send() {
            warn!("Cannot react: this window is offline");
            return;
        }
        let Some(own) = self
            .account_lid
            .clone()
            .or_else(|| self.account_jid.clone())
        else {
            warn!("Cannot react: the account identity is not known yet");
            return;
        };
        let Some(chat) = self.selected_chat_data() else {
            return;
        };
        let Some(message) = chat.messages.iter().find(|m| m.id == message_id) else {
            return;
        };
        // The row before ours goes on, so a refusal can put it back exactly —
        // including a different emoji of ours this tap may replace.
        let previous_reactions = message.reactions.clone();
        // Lifted before sending, which needs `self` mutably.
        let chat_jid = chat.jid.clone();
        let already_ours = message
            .reactions
            .get(emoji)
            .is_some_and(|senders| senders.contains(&own));
        let send = if already_ours {
            String::new()
        } else {
            emoji.to_string()
        };

        // The account connection, not the control plane: a reaction touches
        // the account, and the daemon refuses account requests anywhere
        // else. `control()` is the process-wide connection beside it, which
        // is why this sent nothing on a page until it moved here.
        if let Some(session) = self.client() {
            // Channels are addressed by the server's content id, which the
            // session resolves from the stored row. An ordinary chat keeps the
            // E2E reaction path and the local bubble echo.
            let waiting = if is_channel_jid(&chat_jid) {
                session.react_to_channel(ChannelReact {
                    channel_jid: chat_jid.clone(),
                    message_id: message_id.to_string(),
                    emoji: send.clone(),
                })
            } else {
                session.send_reaction(&chat_jid, message_id, &send)
            };
            let target_chat = chat_jid.clone();
            let target_id = message_id.to_string();
            cx.spawn(async move |entity, cx| {
                let failure = match waiting.await {
                    Ok(Ok(())) => None,
                    Ok(Err(failure)) => Some(failure),
                    Err(_) => Some(crate::session::Failure::worth_retrying(crate::tr!(
                        DaemonDisconnectedBeforeAnswer
                    ))),
                };
                if let Some(failure) = failure {
                    let _ = entity.update(cx, |app, cx| {
                        if let Some(chat) = app.find_chat_mut(&target_chat) {
                            if let Some(message) =
                                chat.messages.iter_mut().find(|m| m.id == target_id)
                            {
                                message.reactions = previous_reactions;
                            }
                            app.invalidate_message_cache(&target_chat, cx);
                        }
                        app.notify_user(
                            what_went_wrong(crate::tr!(CouldNotSaveReaction), &failure),
                            notices::Tone::Problem,
                            cx,
                        );
                    });
                }
            })
            .detach();
        }
        // Optimistic, in the same form the echo will confirm: an empty send
        // removes rather than adds, which is the one spelling
        // `add_reaction` already gives a removal.
        if let Some(chat) = self.find_chat_mut(&chat_jid) {
            chat.add_reaction(message_id, send, own);
            self.invalidate_message_cache(&chat_jid, cx);
        }
        self.reaction_picker_for = None;
        self.reaction_picker_full_target = None;
        cx.notify();
    }
}

/// The full emoji table as a modal over the conversation. Sits at the app
/// level rather than under the bubble because the timeline virtualises rows:
/// a popover anchored to one would lose its anchor the moment the list moved.
pub(super) fn render_full_reaction_picker(
    picker: &Entity<EmojiPicker>,
    app: &Entity<WhatsAppApp>,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<WhatsAppApp>,
) -> impl IntoElement {
    let metrics = cx.product().metrics;
    // Eleven across, like the composer's own picker, sized to the viewport so
    // the grid is usable on a phone and does not swallow a whole desktop.
    const EMOJI_ACROSS: usize = 11;
    let viewport = window.viewport_size();
    let width = (viewport.width * 0.5)
        .clamp(metrics.icon_button() * 14.0, metrics.icon_button() * 34.0)
        .min(viewport.width - metrics.space_xl());
    let height = (viewport.height * 0.6)
        .max(metrics.icon_button() * 11.0)
        .min(viewport.height - metrics.space_xl());
    let emoji_cell = width / EMOJI_ACROSS as f32;
    picker.update(cx, |table, cx| {
        if table.fit(
            EMOJI_ACROSS,
            ((height - metrics.icon_button() * 5.0) / emoji_cell).floor(),
            emoji_cell,
        ) {
            cx.notify();
        }
    });
    let close_app = app.clone();

    div()
        .id("full-reaction-picker")
        .debug_selector(|| "full-reaction-picker".into())
        .track_focus(focus)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(metrics.space_xl())
        .bg(crate::components::parts::scrim(cx).opacity(0.82))
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .w(width)
                .h(height)
                .p(metrics.space_md())
                .rounded(metrics.radius_lg())
                .bg(cx.theme().background)
                .flex()
                .flex_col()
                .gap(metrics.space_sm())
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(crate::tr!(ChooseEmoji))
                        .child(
                            gpui_component::button::Button::new("full-reaction-close")
                                .debug_selector(|| "full-reaction-close".into())
                                .ghost()
                                .xsmall()
                                .icon(gpui_component::IconName::Close)
                                .on_click(move |_, _, cx| {
                                    close_app.update(cx, |app, cx| {
                                        app.close_full_reaction_picker(cx);
                                    });
                                }),
                        ),
                )
                .child(picker.clone().into_any_element()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_routing_uses_the_newsletter_suffix() {
        assert!(is_channel_jid("155555500001@newsletter"));
        assert!(!is_channel_jid("120363000000000001@g.us"));
        assert!(!is_channel_jid("155555500001@s.whatsapp.net"));
        assert!(!is_channel_jid("status@broadcast"));
        assert!(!is_channel_jid("not a jid"));
    }

    /// The full picker's target is taken only on close, so a pick cannot land
    /// on a different message than the one whose strip opened the table.
    #[gpui::test]
    fn full_reaction_picker_closes_once_and_clears_its_target(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
        let app = cx.update(|cx| cx.new(WhatsAppApp::new));
        app.update(cx, |app, cx| {
            assert!(!app.close_full_reaction_picker(cx));
            app.reaction_picker_full_target = Some("MSG-1".into());
            assert_eq!(app.full_reaction_target(), Some("MSG-1"));
            assert!(app.close_full_reaction_picker(cx));
            assert_eq!(app.full_reaction_target(), None);
            assert!(!app.close_full_reaction_picker(cx));
        });
    }
}
