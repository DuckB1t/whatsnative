//! The form behind a native calendar event.
//!
//! Every field except the name is optional on the wire, and the dialog keeps
//! that shape: empty fields stay absent rather than becoming zeros, dates, or
//! a pin at (0, 0). The only answer that crosses back is the created message
//! reference; the secret belongs to the session.

use std::str::FromStr;

use gpui::{
    App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div,
};
use gpui_component::FocusTrapElement as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, v_flex};
use oxidezap_core::EventLocation;
use oxidezap_ipc::CreateEvent;

use super::*;

/// The boxes a person fills before a creation goes to the daemon.
pub(super) struct EventDraftDialog {
    jid: String,
    name: Entity<InputState>,
    description: Entity<InputState>,
    start_time: Entity<InputState>,
    end_time: Entity<InputState>,
    join_link: Entity<InputState>,
    place: Entity<InputState>,
    latitude: Entity<InputState>,
    longitude: Entity<InputState>,
    is_scheduled_call: bool,
    extra_guests_allowed: bool,
}

impl WhatsAppApp {
    pub(crate) fn begin_event_dialog(
        &mut self,
        jid: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_send()
            || self.event_dialog.is_some()
            || self.channel_poll_dialog.is_some()
            || self.community_dialog.is_some()
            || self.delete_confirmation.is_some()
            || self.paste_preview.is_some()
            || self.edit_draft.is_some()
        {
            return;
        }
        let name = input_with_placeholder(crate::tr!(EventName), window, cx);
        let description = input_with_placeholder(crate::tr!(EventDescription), window, cx);
        let start_time = input_with_placeholder(crate::tr!(EventStart), window, cx);
        let end_time = input_with_placeholder(crate::tr!(EventEnd), window, cx);
        let join_link = input_with_placeholder(crate::tr!(EventJoinLink), window, cx);
        let place = input_with_placeholder(crate::tr!(EventPlace), window, cx);
        let latitude = input_with_placeholder(crate::tr!(EventLatitude), window, cx);
        let longitude = input_with_placeholder(crate::tr!(EventLongitude), window, cx);
        name.update(cx, |input, cx| input.focus(window, cx));
        self.event_dialog = Some(EventDraftDialog {
            jid: jid.to_string(),
            name,
            description,
            start_time,
            end_time,
            join_link,
            place,
            latitude,
            longitude,
            is_scheduled_call: false,
            extra_guests_allowed: false,
        });
        cx.notify();
    }

    pub(super) fn cancel_event_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if self.event_dialog.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    pub(super) fn submit_event_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.event_dialog.as_ref() else {
            return;
        };
        let name: String = dialog.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.notify_user(crate::tr!(EventNameRequired), notices::Tone::Problem, cx);
            return;
        }
        let description = optional_text(&dialog.description, cx);
        let join_link = optional_text(&dialog.join_link, cx);
        let start_time = match optional_number(&dialog.start_time, cx) {
            Ok(value) => value,
            Err(()) => return self.invalid_event_fields(cx),
        };
        let end_time = match optional_number(&dialog.end_time, cx) {
            Ok(value) => value,
            Err(()) => return self.invalid_event_fields(cx),
        };
        let latitude = match optional_number(&dialog.latitude, cx) {
            Ok(value) => value,
            Err(()) => return self.invalid_event_fields(cx),
        };
        let longitude = match optional_number(&dialog.longitude, cx) {
            Ok(value) => value,
            Err(()) => return self.invalid_event_fields(cx),
        };
        let location = match (latitude, longitude) {
            (Some(degrees_latitude), Some(degrees_longitude)) => Some(EventLocation {
                degrees_latitude,
                degrees_longitude,
                name: optional_text(&dialog.place, cx),
            }),
            (None, None) => None,
            // A label without a pin, or half a pin, is not a location. Sending
            // it would put an event somewhere nobody chose.
            _ => return self.invalid_event_fields(cx),
        };
        let request = CreateEvent {
            jid: dialog.jid.clone(),
            name,
            description,
            start_time,
            end_time,
            join_link,
            location,
            is_scheduled_call: dialog.is_scheduled_call,
            extra_guests_allowed: dialog.extra_guests_allowed,
        };
        let Some(client) = self.client.as_ref() else {
            self.notify_user(crate::tr!(CouldNotCreateEvent), notices::Tone::Problem, cx);
            return;
        };
        let answer = client.create_event(request);
        self.event_dialog = None;
        cx.notify();
        cx.spawn(async move |entity, cx| {
            let failure = match answer.await {
                Ok(Ok(_created)) => None,
                Ok(Err(failure)) => Some(failure),
                Err(_) => Some(crate::session::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            if let Some(failure) = failure {
                let _ = entity.update(cx, |app, cx| {
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotCreateEvent), &failure),
                        notices::Tone::Problem,
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    fn invalid_event_fields(&mut self, cx: &mut Context<Self>) {
        self.notify_user(crate::tr!(EventFieldsInvalid), notices::Tone::Problem, cx);
    }
}

fn input_with_placeholder(
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<WhatsAppApp>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

fn optional_text(field: &Entity<InputState>, cx: &App) -> Option<String> {
    let text = field.read(cx).value().trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn optional_number<T: FromStr>(field: &Entity<InputState>, cx: &App) -> Result<Option<T>, ()> {
    let text = field.read(cx).value();
    let text = text.trim();
    if text.is_empty() {
        Ok(None)
    } else {
        text.parse().map(Some).map_err(|_| ())
    }
}

pub(super) fn render_event_dialog(
    dialog: &EventDraftDialog,
    app: Entity<WhatsAppApp>,
    focus: &FocusHandle,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let cancel = app.clone();
    let scheduled = app.clone();
    let guests = app.clone();
    let value = |on: bool| {
        if on {
            crate::tr!(EventOn)
        } else {
            crate::tr!(EventOff)
        }
    };

    div()
        .id("event-create-modal")
        .debug_selector(|| "event-create-modal".into())
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
                .id("event-create-form")
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
                        .child(crate::tr!(NewEvent)),
                )
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::tr!(conversation, &dialog.jid)),
                )
                .child(field(
                    crate::tr!(EventName).into(),
                    &dialog.name,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(EventDescription).into(),
                    &dialog.description,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(EventStart).into(),
                    &dialog.start_time,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(EventEnd).into(),
                    &dialog.end_time,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(EventJoinLink).into(),
                    &dialog.join_link,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(EventPlace).into(),
                    &dialog.place,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(EventLatitude).into(),
                    &dialog.latitude,
                    metrics,
                    cx,
                ))
                .child(field(
                    crate::tr!(EventLongitude).into(),
                    &dialog.longitude,
                    metrics,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(metrics.space_sm())
                        .child(
                            Button::new("event-scheduled-call")
                                .ghost()
                                .label(format!(
                                    "{}: {}",
                                    crate::tr!(EventScheduledCall),
                                    value(dialog.is_scheduled_call)
                                ))
                                .on_click(move |_, _, cx| {
                                    scheduled.update(cx, |app, cx| {
                                        if let Some(draft) = app.event_dialog.as_mut() {
                                            draft.is_scheduled_call = !draft.is_scheduled_call;
                                            cx.notify();
                                        }
                                    });
                                }),
                        )
                        .child(
                            Button::new("event-extra-guests")
                                .ghost()
                                .label(format!(
                                    "{}: {}",
                                    crate::tr!(EventExtraGuests),
                                    value(dialog.extra_guests_allowed)
                                ))
                                .on_click(move |_, _, cx| {
                                    guests.update(cx, |app, cx| {
                                        if let Some(draft) = app.event_dialog.as_mut() {
                                            draft.extra_guests_allowed =
                                                !draft.extra_guests_allowed;
                                            cx.notify();
                                        }
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(metrics.space_md())
                        .child(
                            Button::new("event-create-cancel")
                                .label(crate::tr!(Cancel))
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |app, cx| {
                                        app.cancel_event_dialog(cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("event-create-submit")
                                .primary()
                                .label(crate::tr!(CreateEvent))
                                .on_click(move |_, _, cx| {
                                    app.update(cx, |app, cx| app.submit_event_dialog(cx));
                                }),
                        ),
                ),
        )
        .focus_trap("event-create-trap", focus)
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
