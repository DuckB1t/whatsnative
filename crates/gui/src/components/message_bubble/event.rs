//! A calendar invitation in the WhatsApp Web shape.
//!
//! The card carries the facts the wire sent and nothing it did not: an absent
//! end time has no invented end, an uncounted invitation has no bar, and a tap
//! paints an attempted answer rather than a confirmed one. The session owns the
//! secret that makes an RSVP possible, so this side can only say "requested"
//! until the correlated answer comes back.

use gpui::{
    App, Entity, IntoElement, ParentElement, SharedString, Styled, div, prelude::FluentBuilder as _,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_component::{Icon, IconName};

use crate::app::WhatsAppApp;
use crate::theme::Metrics;
use oxidezap_core::{EventCard, EventRsvp};

/// Draw an event card and its three answers.
///
/// `attempted` is the answer this window just asked for, not one WhatsApp has
/// acknowledged. It is labelled as a request so a refusal can be taken back by
/// the same tap on failure without ever claiming a vote was recorded.
pub fn render_event_card(
    card: &EventCard,
    chat_jid: SharedString,
    message_id: &str,
    attempted: Option<EventRsvp>,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let title: SharedString = if card.name.trim().is_empty() {
        crate::tr!(UntitledEvent).into()
    } else {
        card.name.clone().into()
    };
    let answers = [EventRsvp::Going, EventRsvp::Maybe, EventRsvp::NotGoing];

    v_flex()
        .w_full()
        .gap(metrics.space_md())
        .child(
            h_flex()
                .items_start()
                .gap(metrics.space_sm())
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(cx.theme().primary)
                        .child(Icon::new(IconName::Calendar).with_size(metrics.icon_small())),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .gap(metrics.space_xxs())
                        .child(
                            div()
                                .text_size(metrics.text_body())
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(cx.theme().foreground)
                                .child(title),
                        )
                        .when(card.is_canceled, |el| {
                            el.child(
                                div()
                                    .text_size(metrics.text_small())
                                    .text_color(cx.theme().danger)
                                    .child(crate::tr!(EventCanceled)),
                            )
                        }),
                ),
        )
        .children(
            card.description
                .as_ref()
                .filter(|text| !text.trim().is_empty())
                .map(|text| {
                    div()
                        .text_size(metrics.text_small())
                        .text_color(cx.theme().muted_foreground)
                        .child(SharedString::from(text.clone()))
                }),
        )
        .children(event_time(card).map(|time| {
            div()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(time)
        }))
        .children(card.location.as_ref().map(|location| {
            let place = match location.name.as_deref() {
                Some(name) if !name.trim().is_empty() => name.to_string(),
                _ => format!(
                    "{}, {}",
                    location.degrees_latitude, location.degrees_longitude
                ),
            };
            div()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(SharedString::from(place))
        }))
        .children(
            card.join_link
                .as_ref()
                .filter(|link| !link.trim().is_empty())
                .map(|link| {
                    let url = link.clone();
                    Button::new(SharedString::from(format!("event-join-{message_id}")))
                        .ghost()
                        .label(crate::tr!(OpenJoinLink))
                        .on_click(move |_, _, cx| cx.open_url(&url))
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(metrics.space_xs())
                .children(answers.into_iter().map(|answer| {
                    let answer_entity = entity.clone();
                    let answer_chat = chat_jid.clone();
                    let answer_id = message_id.to_string();
                    let active = attempted == Some(answer);
                    Button::new(SharedString::from(format!(
                        "event-rsvp-{message_id}-{answer:?}"
                    )))
                    .ghost()
                    .disabled(card.is_canceled)
                    .when(active, |button| button.primary())
                    .label(rsvp_label(answer))
                    .on_click(move |_, _, cx| {
                        answer_entity.update(cx, |app, cx| {
                            app.respond_event(&answer_chat, &answer_id, answer, cx);
                        });
                    })
                }))
                .when(attempted.is_some(), |el| {
                    el.child(
                        div()
                            .text_size(metrics.text_small())
                            .text_color(cx.theme().primary)
                            .child(crate::tr!(Requested)),
                    )
                }),
        )
}

fn rsvp_label(answer: EventRsvp) -> SharedString {
    match answer {
        EventRsvp::Going => crate::tr!(Going).into(),
        EventRsvp::NotGoing => crate::tr!(NotGoing).into(),
        EventRsvp::Maybe => crate::tr!(Maybe).into(),
    }
}

/// The range the card actually knows: one end alone is a start, neither is
/// nothing. Times come from the protocol clock through `wacore::time`, then
/// render in the reader's own zone.
fn event_time(card: &EventCard) -> Option<SharedString> {
    match (card.start_time, card.end_time) {
        (Some(start), Some(end)) => {
            Some(crate::tr!(event_time, format_event_time(start), format_event_time(end)).into())
        }
        (Some(start), None) => Some(crate::tr!(event_start, format_event_time(start)).into()),
        (None, _) => None,
    }
}

fn format_event_time(timestamp: i64) -> SharedString {
    wacore::time::from_secs(timestamp)
        .map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| timestamp.to_string())
        .into()
}
