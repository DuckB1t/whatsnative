//! Joining a WhatsApp call link from the GUI.
//!
//! A link is an opaque invite, not an address. The text a person pastes stays
//! a string all the way to the daemon: it is never parsed as a JID, and the
//! token in the preview is never put on screen. What is shown instead is the
//! little the preview is safe to say — who made it, what kind of call it is,
//! and whether there is a waiting room — before the person explicitly agrees
//! to join.

use gpui::{
    App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div,
};
use gpui_component::FocusTrapElement as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Disableable as _, Selectable as _, v_flex};
use oxidezap_core::{
    CALL_LINK_RECIPIENT_JID, CALL_LINK_RECIPIENT_NAME, CallLinkMedia, CallLinkPreview, OutgoingCall,
};

use super::*;

/// The link, the choice it must agree with, and the answer it resolved to.
///
/// `token_or_url` is kept exactly as it was validated: the daemon joins with
/// that string, and replacing it with a JID — or with the token out of the
/// preview — would lose the part of the invite the link actually carries.
pub(super) struct CallLinkDialog {
    link: Entity<InputState>,
    media: CallLinkMedia,
    token_or_url: Option<String>,
    preview: Option<CallLinkPreview>,
    /// Names the one preview request this form is still waiting for. An answer
    /// from a form that was cancelled and reopened must not land on its
    /// replacement.
    preview_request: Option<String>,
    awaiting: bool,
    consent_given: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedCallLink {
    token_or_url: String,
    media: CallLinkMedia,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallLinkFormError {
    Invalid,
    MediaMismatch,
}

/// Validate what was pasted, without ever asking the JID parser about it.
///
/// A full link fixes the media itself, so a video link chosen as voice is a
/// mistake rather than a link to reinterpret. A bare token has no such claim
/// and takes the media the person picked.
fn validate_call_link(
    pasted: &str,
    choice: CallLinkMedia,
) -> Result<ValidatedCallLink, CallLinkFormError> {
    const PREFIX: &str = "https://call.whatsapp.com/";
    let pasted = pasted.trim();
    if pasted.is_empty() {
        return Err(CallLinkFormError::Invalid);
    }

    if let Some(rest) = pasted.strip_prefix(PREFIX) {
        // A query or fragment belongs to the link, not to its path. Splitting
        // the path off first is what lets `?`/`#` carry arbitrary punctuation
        // without being mistaken for another segment.
        let path = match rest.find(['?', '#']) {
            Some(index) => &rest[..index],
            None => rest,
        };
        let mut segments = path.split('/');
        let kind = segments.next().unwrap_or("");
        let token = segments.next().unwrap_or("");
        if segments.next().is_some() || token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(CallLinkFormError::Invalid);
        }
        let media = match kind {
            "audio" => CallLinkMedia::Audio,
            "video" => CallLinkMedia::Video,
            _ => return Err(CallLinkFormError::Invalid),
        };
        if media != choice {
            return Err(CallLinkFormError::MediaMismatch);
        }
        return Ok(ValidatedCallLink {
            token_or_url: pasted.to_string(),
            media,
        });
    }

    // Anything with a scheme, a path, or whitespace is not an opaque token.
    // Notably `@` is *not* a reason to reject it: an invite is not an address,
    // and treating one as such is the mistake this whole form avoids.
    if pasted.contains("://") || pasted.contains('/') || pasted.chars().any(char::is_whitespace) {
        return Err(CallLinkFormError::Invalid);
    }
    Ok(ValidatedCallLink {
        token_or_url: pasted.to_string(),
        media: choice,
    })
}

/// Both halves are required: a preview that was never shown cannot be agreed
/// to, and an agreement without a preview is an agreement to nothing.
fn can_join_call_link(preview_shown: bool, consent_given: bool) -> bool {
    preview_shown && consent_given
}

impl WhatsAppApp {
    pub fn begin_call_link_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send()
            || self.client.is_none()
            || self.call_link_dialog.is_some()
            || self.event_dialog.is_some()
            || self.channel_poll_dialog.is_some()
            || self.community_dialog.is_some()
            || self.delete_confirmation.is_some()
            || self.paste_preview.is_some()
            || self.edit_draft.is_some()
            || self.viewer.read(cx).showing().is_some()
        {
            return;
        }
        if self.calls.read(cx).is_busy() {
            log::warn!("Cannot join a call link: a call is already in progress");
            return;
        }
        if let Some(reason) = crate::platform::calls_belong_to_another_tab() {
            self.notify_user(reason, notices::Tone::Problem, cx);
            return;
        }
        if let Some(reason) = crate::platform::calls_unavailable() {
            self.notify_user(reason, notices::Tone::Problem, cx);
            return;
        }

        let link = input(crate::tr!(CallLinkPaste), window, cx);
        link.update(cx, |input, cx| input.focus(window, cx));
        self.call_link_dialog = Some(CallLinkDialog {
            link,
            media: CallLinkMedia::Audio,
            token_or_url: None,
            preview: None,
            preview_request: None,
            awaiting: false,
            consent_given: false,
        });
        cx.notify();
    }

    /// Close the form, including any answer still on its way. Dropping the
    /// receiver is enough: the session's pending sweep retires an answer
    /// nobody is listening for any more.
    pub fn cancel_call_link_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if self.call_link_dialog.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    /// The other forms cannot all learn about this one inside this change, so
    /// the root checks on the way down: if one of them opened over the link
    /// form, the link form is the layer that goes away. This keeps the visible
    /// surfaces mutually exclusive without teaching a call path a new owner.
    pub(super) fn retire_conflicting_call_link_dialog(&mut self, cx: &App) {
        let conflict = self.call_link_dialog.is_some()
            && (self.event_dialog.is_some()
                || self.channel_poll_dialog.is_some()
                || self.community_dialog.is_some()
                || self.delete_confirmation.is_some()
                || self.paste_preview.is_some()
                || self.edit_draft.is_some()
                || self.viewer.read(cx).showing().is_some());
        if conflict {
            self.call_link_dialog = None;
        }
    }

    fn set_call_link_media(&mut self, media: CallLinkMedia, cx: &mut Context<Self>) {
        let Some(dialog) = self.call_link_dialog.as_mut() else {
            return;
        };
        if dialog.media != media {
            dialog.media = media;
            dialog.preview = None;
            dialog.token_or_url = None;
            dialog.preview_request = None;
            dialog.consent_given = false;
            cx.notify();
        }
    }

    fn set_call_link_consent(&mut self, given: bool, cx: &mut Context<Self>) {
        let Some(dialog) = self.call_link_dialog.as_mut() else {
            return;
        };
        if dialog.consent_given != given && dialog.preview.is_some() {
            dialog.consent_given = given;
            cx.notify();
        }
    }

    /// Resolve the pasted link without joining it. Nothing is drawn as a call
    /// here: the preview is only a thing to look at, and the person still has
    /// to say yes.
    pub fn submit_call_link_dialog(&mut self, cx: &mut Context<Self>) {
        let (pasted, choice) = {
            let Some(dialog) = self.call_link_dialog.as_ref() else {
                return;
            };
            if dialog.awaiting {
                return;
            }
            (dialog.link.read(cx).value().to_string(), dialog.media)
        };

        let validated = match validate_call_link(&pasted, choice) {
            Ok(validated) => validated,
            Err(CallLinkFormError::MediaMismatch) => {
                return self.invalid_call_link(crate::tr!(CallLinkMediaMismatch), cx);
            }
            Err(CallLinkFormError::Invalid) => {
                return self.invalid_call_link(crate::tr!(CallLinkInvalid), cx);
            }
        };
        let Some(client) = self.client.as_ref() else {
            return self.invalid_call_link(crate::tr!(DaemonDisconnectedBeforeAnswer), cx);
        };
        let validated_media = validated.media;
        let request_id = Self::next_local_id("call-link-preview");

        {
            let Some(dialog) = self.call_link_dialog.as_mut() else {
                return;
            };
            dialog.awaiting = true;
            dialog.preview = None;
            dialog.token_or_url = None;
            dialog.preview_request = Some(request_id.clone());
            dialog.consent_given = false;
        }
        cx.notify();

        let answer = client.preview_call_link(validated.token_or_url, validated.media);
        cx.spawn(async move |entity, cx| {
            let answered = match answer.await {
                Ok(answered) => answered,
                Err(_) => Err(crate::session::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            let _ = entity.update(cx, |app, cx| {
                let Some(dialog) = app.call_link_dialog.as_mut() else {
                    return;
                };
                if dialog.preview_request.as_deref() != Some(request_id.as_str()) {
                    return;
                }
                dialog.awaiting = false;
                dialog.preview_request = None;
                dialog.consent_given = false;
                match answered {
                    Ok(preview) if preview.media == validated_media => {
                        dialog.preview = Some(preview);
                        dialog.token_or_url = Some(pasted);
                    }
                    Ok(_) => {
                        dialog.preview = None;
                        dialog.token_or_url = None;
                        app.notify_user(
                            crate::tr!(CallLinkMediaMismatch),
                            notices::Tone::Problem,
                            cx,
                        );
                    }
                    Err(failure) => {
                        dialog.preview = None;
                        dialog.token_or_url = None;
                        let message = what_went_wrong(crate::tr!(CallLinkPreviewTitle), &failure);
                        app.notify_user(message, notices::Tone::Problem, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Join only after a preview was shown *and* the consent box was ticked.
    /// Radios, notices and stale state cannot get a call placed by themselves.
    pub fn confirm_call_link_join(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.call_link_dialog.as_ref() else {
            return;
        };
        if dialog.awaiting || !can_join_call_link(dialog.preview.is_some(), dialog.consent_given) {
            return;
        }
        let Some(token_or_url) = dialog.token_or_url.clone() else {
            return;
        };
        let media = dialog.media;

        // The same guards a direct call obeys. A call link needs no callable
        // chat JID, but it is still a call, and this window still has to be
        // online and free to carry it.
        if !self.can_send() {
            log::warn!("Cannot join call link: this window is offline");
            return;
        }
        if self.client.is_none() {
            log::warn!("Cannot join call link: client is unavailable");
            return;
        }
        if self.calls.read(cx).is_busy() {
            log::warn!("Cannot join call link: a call is already in progress");
            return;
        }
        if let Some(reason) = crate::platform::calls_belong_to_another_tab() {
            self.notify_user(reason, notices::Tone::Problem, cx);
            return;
        }
        if let Some(reason) = crate::platform::calls_unavailable() {
            self.notify_user(reason, notices::Tone::Problem, cx);
            return;
        }

        let placeholder_id = Self::next_local_id("ui-call-link");
        let call = OutgoingCall::new(
            placeholder_id.clone(),
            CALL_LINK_RECIPIENT_JID.to_string(),
            CALL_LINK_RECIPIENT_NAME.to_string(),
            media == CallLinkMedia::Video,
        );
        self.calls
            .update(cx, |calls, cx| calls.place_outgoing(call, cx));

        let Some(client) = self.client.as_ref() else {
            log::warn!("Cannot join call link: client is unavailable");
            return;
        };
        client.join_call_link(token_or_url, media, placeholder_id);
        self.call_link_dialog = None;
        cx.notify();
    }

    fn invalid_call_link(&mut self, detail: impl Into<String>, cx: &mut Context<Self>) {
        self.notify_user(detail, notices::Tone::Problem, cx);
    }
}

fn media_label(media: CallLinkMedia) -> &'static str {
    match media {
        CallLinkMedia::Audio => crate::tr!(CallLinkVoice),
        CallLinkMedia::Video => crate::tr!(CallLinkVideo),
    }
}

/// The preview names a creator by JID and sometimes by phone number. Prefer a
/// name this window already knows; the token is deliberately never consulted.
fn creator_name(app: &Entity<WhatsAppApp>, preview: &CallLinkPreview, cx: &App) -> String {
    if let Some(name) = app
        .read(cx)
        .find_chat(&preview.creator_jid)
        .map(|chat| chat.name.clone())
    {
        return name;
    }
    preview
        .creator_pn
        .clone()
        .unwrap_or_else(|| preview.creator_jid.clone())
}

fn input(
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<WhatsAppApp>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

pub(super) fn render_call_link_dialog(
    dialog: &CallLinkDialog,
    app: Entity<WhatsAppApp>,
    focus: &FocusHandle,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let cancel = app.clone();
    let voice = app.clone();
    let video = app.clone();
    let consent = app.clone();
    let join = app.clone();

    let draft = !dialog.awaiting && dialog.preview.is_none();
    let preview_panel = dialog.preview.as_ref().map(|preview| {
        let creator = creator_name(&app, preview, cx);
        let waiting_room = format!(
            "{}: {}",
            crate::tr!(CallLinkWaitingRoom),
            if preview.waiting_room_enabled {
                crate::tr!(EventOn)
            } else {
                crate::tr!(EventOff)
            }
        );
        div()
            .flex()
            .flex_col()
            .gap(metrics.space_xxs())
            .rounded(metrics.radius_lg())
            .bg(cx.theme().sidebar)
            .p(metrics.space_md())
            .child(
                div()
                    .text_size(metrics.text_small())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().foreground)
                    .child(crate::tr!(CallLinkPreviewTitle)),
            )
            .child(
                div()
                    .text_size(metrics.text_small())
                    .text_color(cx.theme().foreground)
                    .child(creator),
            )
            .child(
                div()
                    .text_size(metrics.text_small())
                    .text_color(cx.theme().muted_foreground)
                    .child(media_label(preview.media).to_string()),
            )
            .child(
                div()
                    .text_size(metrics.text_small())
                    .text_color(cx.theme().muted_foreground)
                    .child(waiting_room),
            )
    });

    let consent_panel = dialog.preview.as_ref().map(|_| {
        div()
            .flex()
            .flex_col()
            .gap(metrics.space_sm())
            .child(
                Checkbox::new("call-link-consent")
                    .label(crate::tr!(CallLinkConsent))
                    .checked(dialog.consent_given)
                    .on_click(move |checked, _, cx| {
                        consent.update(cx, |app, cx| {
                            app.set_call_link_consent(*checked, cx);
                        });
                    }),
            )
            .child(
                Button::new("call-link-join-confirm")
                    .primary()
                    .label(crate::tr!(CallLinkJoinConfirm))
                    .disabled(!dialog.consent_given)
                    .on_click(move |_, _, cx| {
                        join.update(cx, |app, cx| app.confirm_call_link_join(cx));
                    }),
            )
    });

    div()
        .id("call-link-modal")
        .debug_selector(|| "call-link-modal".into())
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
                .id("call-link-form")
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
                        .child(crate::tr!(CallLinkJoin)),
                )
                .when(draft, |el| {
                    el.child(field(
                        crate::tr!(CallLinkPaste).into(),
                        &dialog.link,
                        metrics,
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(metrics.space_sm())
                            .child(
                                Button::new("call-link-voice")
                                    .label(crate::tr!(CallLinkVoice))
                                    .outline()
                                    .selected(dialog.media == CallLinkMedia::Audio)
                                    .on_click(move |_, _, cx| {
                                        voice.update(cx, |app, cx| {
                                            app.set_call_link_media(CallLinkMedia::Audio, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("call-link-video")
                                    .label(crate::tr!(CallLinkVideo))
                                    .outline()
                                    .selected(dialog.media == CallLinkMedia::Video)
                                    .on_click(move |_, _, cx| {
                                        video.update(cx, |app, cx| {
                                            app.set_call_link_media(CallLinkMedia::Video, cx);
                                        });
                                    }),
                            ),
                    )
                })
                .when(dialog.awaiting, |el| {
                    el.child(
                        div()
                            .text_size(metrics.text_small())
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!(CallLinkPreviewTitle)),
                    )
                })
                .children(preview_panel)
                .children(consent_panel)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(metrics.space_md())
                        .child(
                            Button::new("call-link-cancel")
                                .label(crate::tr!(Cancel))
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |app, cx| {
                                        app.cancel_call_link_dialog(cx);
                                    });
                                }),
                        )
                        .when(draft, |el| {
                            el.child(
                                Button::new("call-link-preview")
                                    .primary()
                                    .label(crate::tr!(CallLinkJoin))
                                    .on_click(move |_, _, cx| {
                                        app.update(cx, |app, cx| {
                                            app.submit_call_link_dialog(cx);
                                        });
                                    }),
                            )
                        }),
                ),
        )
        .focus_trap("call-link-trap", focus)
}

fn field(
    label: SharedString,
    input_field: &Entity<InputState>,
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
        .child(Input::new(input_field).w_full())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_call_links_validate_without_parsing_jids() {
        let url = validate_call_link(
            "  https://call.whatsapp.com/audio/abc123?utm=invite  ",
            CallLinkMedia::Audio,
        )
        .unwrap();
        assert_eq!(
            url.token_or_url,
            "https://call.whatsapp.com/audio/abc123?utm=invite"
        );
        assert_eq!(url.media, CallLinkMedia::Audio);

        let fragment = validate_call_link(
            "https://call.whatsapp.com/video/token#section",
            CallLinkMedia::Video,
        )
        .unwrap();
        assert_eq!(
            fragment.token_or_url,
            "https://call.whatsapp.com/video/token#section"
        );
        assert_eq!(fragment.media, CallLinkMedia::Video);

        let bare = validate_call_link("  opaque-token_42  ", CallLinkMedia::Video).unwrap();
        assert_eq!(bare.token_or_url, "opaque-token_42");
        assert_eq!(bare.media, CallLinkMedia::Video);

        // A token that happens to look like an address is still carried
        // verbatim. The point is that the form never asks a JID parser.
        let opaque = "abc.def@not-a-jid";
        assert_eq!(
            validate_call_link(opaque, CallLinkMedia::Audio)
                .unwrap()
                .token_or_url,
            opaque
        );

        for bad in [
            "",
            "http://call.whatsapp.com/audio/token",
            "https://call.whatsapp.com/audio/token/extra",
            "https://call.whatsapp.com/audio/",
            "https://call.whatsapp.com/voice/token",
            "token with a space",
            "token/part",
        ] {
            assert_eq!(
                validate_call_link(bad, CallLinkMedia::Audio),
                Err(CallLinkFormError::Invalid),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_video_link_needs_a_video_media_choice() {
        let link = "https://call.whatsapp.com/video/abc123";
        assert_eq!(
            validate_call_link(link, CallLinkMedia::Audio),
            Err(CallLinkFormError::MediaMismatch)
        );
        assert_eq!(
            validate_call_link(link, CallLinkMedia::Video)
                .unwrap()
                .media,
            CallLinkMedia::Video
        );
        assert_eq!(
            validate_call_link(
                "https://call.whatsapp.com/audio/abc123",
                CallLinkMedia::Video
            ),
            Err(CallLinkFormError::MediaMismatch)
        );
        // A bare token has no declared kind, so the explicit choice governs.
        assert_eq!(
            validate_call_link("abc123", CallLinkMedia::Video)
                .unwrap()
                .media,
            CallLinkMedia::Video
        );
    }

    #[test]
    fn join_requires_a_shown_preview_and_consent() {
        let preview = CallLinkPreview {
            token: "synthetic-token".to_string(),
            media: CallLinkMedia::Video,
            creator_jid: "creator@example.invalid".to_string(),
            creator_pn: None,
            waiting_room_enabled: true,
            is_admin: false,
        };
        assert!(!can_join_call_link(false, false));
        assert!(!can_join_call_link(true, false));
        assert!(!can_join_call_link(false, true));
        assert!(can_join_call_link(
            preview.media == CallLinkMedia::Video,
            true
        ));
    }
}
