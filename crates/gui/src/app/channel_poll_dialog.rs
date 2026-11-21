//! The form behind a native channel poll.
//!
//! A channel poll's option hashes are computed from the names sent on the
//! wire, so the form trims each line and sends exactly what it shows. There is
//! no server round trip here beyond the creation itself: the answer names the
//! message that was opened.

use gpui::{
    App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div,
};
use gpui_component::FocusTrapElement as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, v_flex};
use oxidezap_core::ChannelPollDraft;

use super::*;

/// The boxes a person fills before a poll goes to the daemon.
pub(super) struct ChannelPollDialog {
    jid: String,
    question: Entity<InputState>,
    options: Entity<InputState>,
    selectable_count: Entity<InputState>,
}

/// The field that stopped a draft from being sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelPollFormError {
    Question,
    Options,
    SelectableCount,
}

impl WhatsAppApp {
    pub(crate) fn begin_channel_poll_dialog(
        &mut self,
        jid: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_send()
            || !is_channel_jid(jid)
            || self.channel_poll_dialog.is_some()
            || self.event_dialog.is_some()
            || self.community_dialog.is_some()
            || self.delete_confirmation.is_some()
            || self.paste_preview.is_some()
            || self.edit_draft.is_some()
        {
            return;
        }
        let question = input(crate::tr!(ChannelPollQuestion), window, cx);
        let options = input(crate::tr!(ChannelPollOptions), window, cx);
        let selectable_count = input(crate::tr!(ChannelPollSelectableCount), window, cx);
        question.update(cx, |input, cx| input.focus(window, cx));
        self.channel_poll_dialog = Some(ChannelPollDialog {
            jid: jid.to_string(),
            question,
            options,
            selectable_count,
        });
        cx.notify();
    }

    pub(super) fn cancel_channel_poll_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if self.channel_poll_dialog.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    pub(super) fn submit_channel_poll_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.channel_poll_dialog.as_ref() else {
            return;
        };
        let draft = channel_poll_draft(
            &dialog.jid,
            &dialog.question.read(cx).value(),
            &dialog.options.read(cx).value(),
            &dialog.selectable_count.read(cx).value(),
        );
        let draft = match draft {
            Ok(draft) => draft,
            Err(ChannelPollFormError::Question) => {
                return self
                    .invalid_channel_poll_field(crate::tr!(ChannelPollQuestionRequired), cx);
            }
            Err(ChannelPollFormError::Options) => {
                return self.invalid_channel_poll_field(crate::tr!(ChannelPollOptionsInvalid), cx);
            }
            Err(ChannelPollFormError::SelectableCount) => {
                return self
                    .invalid_channel_poll_field(crate::tr!(ChannelPollSelectableCountInvalid), cx);
            }
        };
        let Some(client) = self.client.as_ref() else {
            self.notify_user(
                crate::tr!(CouldNotCreateChannelPoll),
                notices::Tone::Problem,
                cx,
            );
            return;
        };
        let answer = client.create_channel_poll(draft);
        self.channel_poll_dialog = None;
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let failure = match answer.await {
                Ok(Ok(created)) => {
                    let _ = entity.update(cx, |app, cx| {
                        // The store row names the same message; if it has
                        // arrived, bring its bubble into view. A timeline that
                        // has not loaded it yet simply stays where it is.
                        app.jump_to_message(&created.message_id, cx);
                    });
                    return;
                }
                Ok(Err(failure)) => failure,
                Err(_) => crate::session::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                )),
            };
            let _ = entity.update(cx, |app, cx| {
                app.notify_user(
                    what_went_wrong(crate::tr!(CouldNotCreateChannelPoll), &failure),
                    notices::Tone::Problem,
                    cx,
                );
            });
        })
        .detach();
    }

    fn invalid_channel_poll_field(&mut self, detail: impl Into<String>, cx: &mut Context<Self>) {
        self.notify_user(detail, notices::Tone::Problem, cx);
    }
}

/// Validate the fields into the request the daemon understands.
///
/// The option names are kept exactly as shown after trimming their line: the
/// session hashes those same names when the vote goes out, so changing case or
/// whitespace here would make a vote name an option the server never saw.
fn channel_poll_draft(
    jid: &str,
    question: &str,
    options: &str,
    selectable_count: &str,
) -> Result<ChannelPollDraft, ChannelPollFormError> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err(ChannelPollFormError::Question);
    }
    let options: Vec<String> = options
        .lines()
        .map(|line| line.trim().to_string())
        .collect();
    if !(2..=12).contains(&options.len())
        || options.iter().any(String::is_empty)
        || options
            .iter()
            .enumerate()
            .any(|(index, option)| options[..index].contains(option))
    {
        return Err(ChannelPollFormError::Options);
    }
    let selectable_count = if selectable_count.trim().is_empty() {
        1
    } else {
        selectable_count
            .trim()
            .parse::<u32>()
            .map_err(|_| ChannelPollFormError::SelectableCount)?
    };
    if selectable_count > 1 {
        return Err(ChannelPollFormError::SelectableCount);
    }
    Ok(ChannelPollDraft {
        channel_jid: jid.to_string(),
        question,
        options,
        selectable_count,
    })
}

fn input(
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<WhatsAppApp>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

pub(super) fn render_channel_poll_dialog(
    dialog: &ChannelPollDialog,
    app: Entity<WhatsAppApp>,
    focus: &FocusHandle,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let cancel = app.clone();
    let submit = app;

    div()
        .id("channel-poll-create-modal")
        .debug_selector(|| "channel-poll-create-modal".into())
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
                .id("channel-poll-create-form")
                .flex()
                .flex_col()
                .w_full()
                .max_w(metrics.bubble_max_width())
                .p(metrics.space_xxl())
                .rounded(metrics.radius_xl())
                .bg(cx.theme().background)
                .gap(metrics.space_lg())
                .overflow_y_scroll()
                .child(
                    div()
                        .text_size(metrics.text_title())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(crate::tr!(NewChannelPoll)),
                )
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::tr!(conversation, &dialog.jid)),
                )
                .child(field(
                    crate::tr!(ChannelPollQuestion).into(),
                    &dialog.question,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(ChannelPollOptions).into(),
                    &dialog.options,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(ChannelPollSelectableCount).into(),
                    &dialog.selectable_count,
                    metrics,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(metrics.space_md())
                        .child(
                            Button::new("channel-poll-create-cancel")
                                .label(crate::tr!(Cancel))
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |app, cx| {
                                        app.cancel_channel_poll_dialog(cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("channel-poll-create-submit")
                                .primary()
                                .label(crate::tr!(CreateChannelPoll))
                                .on_click(move |_, _, cx| {
                                    submit.update(cx, |app, cx| {
                                        app.submit_channel_poll_dialog(cx);
                                    });
                                }),
                        ),
                ),
        )
        .focus_trap("channel-poll-create-trap", focus)
}

fn field(
    label: SharedString,
    input: &Entity<InputState>,
    metrics: crate::theme::Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    v_flex()
        .gap(metrics.space_xxs())
        .child(
            div()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(Input::new(input).w_full())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL: &str = "155555500001@newsletter";

    #[test]
    fn a_valid_form_keeps_the_option_names_it_shows() {
        let draft = channel_poll_draft(
            CHANNEL,
            "  Synthetic question  ",
            "First\n Second \nThird\n",
            "",
        )
        .expect("a valid form");
        assert_eq!(draft.channel_jid, CHANNEL);
        assert_eq!(draft.question, "Synthetic question");
        assert_eq!(draft.options, ["First", "Second", "Third"]);
        assert_eq!(draft.selectable_count, 1);
    }

    #[test]
    fn the_form_refuses_what_the_channel_api_refuses() {
        assert_eq!(
            channel_poll_draft(CHANNEL, "", "First\nSecond", "1"),
            Err(ChannelPollFormError::Question)
        );
        assert_eq!(
            channel_poll_draft(CHANNEL, "Question", "Only one", "1"),
            Err(ChannelPollFormError::Options)
        );
        assert_eq!(
            channel_poll_draft(CHANNEL, "Question", "First\nFirst", "1"),
            Err(ChannelPollFormError::Options)
        );
        assert_eq!(
            channel_poll_draft(CHANNEL, "Question", "First\n \nSecond", "1"),
            Err(ChannelPollFormError::Options)
        );
        assert_eq!(
            channel_poll_draft(CHANNEL, "Question", &["Option"; 13].join("\n"), "1"),
            Err(ChannelPollFormError::Options)
        );
        assert_eq!(
            channel_poll_draft(CHANNEL, "Question", "First\nSecond", "2"),
            Err(ChannelPollFormError::SelectableCount)
        );
        assert_eq!(
            channel_poll_draft(CHANNEL, "Question", "First\nSecond", "many"),
            Err(ChannelPollFormError::SelectableCount)
        );
        assert_eq!(
            channel_poll_draft(CHANNEL, "Question", "First\nSecond", "0")
                .unwrap()
                .selectable_count,
            0
        );
    }
}
