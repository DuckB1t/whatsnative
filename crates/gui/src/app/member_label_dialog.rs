//! Setting this account's own label in one group.
//!
//! The upstream API takes a group and a label, and nothing else: there is no
//! target member to pick, so this form can only ever change *my* label. An
//! empty box is the clear action rather than a validation error, and goes on
//! the wire as the empty string the protocol defines for it.
//!
//! The box is deliberately bounded here. The wire protocol carries strings of
//! any length and the upstream message has its own server-side bounds, so
//! relying on a refusal from there would turn an ordinary typing mistake into
//! a failed request and a notice.

use gpui::{
    App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, Styled, Window, div,
};
use gpui_component::FocusTrapElement as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Disableable as _, v_flex};
use oxidezap_core::is_group_jid;

use super::notices::Tone;
use super::*;

/// A label is a short note about myself, not a message: 64 characters is well
/// inside what a group's member entry can carry, and long enough for any label
/// somebody would put on themselves. Counted in characters rather than bytes
/// so a non-Latin label is not refused for using more of the same letters.
pub(crate) const MAX_MEMBER_LABEL_CHARS: usize = 64;

/// The one thing that stops a label from being sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberLabelError {
    TooLong,
    Newline,
}

/// Trim the box and check the two local bounds.
///
/// An empty result — including a box of only whitespace — is `Ok("")`, the
/// clear request. Trimming is what makes a label of stray spaces mean the same
/// thing as an empty box instead of sending an invisible label.
fn validate_member_label(input: &str) -> Result<String, MemberLabelError> {
    let label = input.trim();
    if label.chars().count() > MAX_MEMBER_LABEL_CHARS {
        return Err(MemberLabelError::TooLong);
    }
    if label.chars().any(|c| c == '\n' || c == '\r') {
        return Err(MemberLabelError::Newline);
    }
    Ok(label.to_string())
}

/// The header control and the form itself ask the same question, so a JID that
/// cannot receive a label never reaches a dialog that would offer one.
pub(crate) fn can_set_member_label(jid: &str, can_send: bool) -> bool {
    can_send && is_group_jid(jid)
}

/// The label being written, and the id of the answer it is waiting for.
pub(super) struct MemberLabelDialog {
    group_jid: String,
    label: Entity<InputState>,
    /// Names the one mutation this form is still waiting for. A reply from a
    /// form that was cancelled and reopened must not land on its replacement —
    /// or raise a notice over a conversation nobody is looking at any more.
    request: Option<String>,
    awaiting: bool,
}

impl WhatsAppApp {
    /// Open the form. Groups only, and only while this window can send.
    pub(crate) fn begin_member_label_dialog(
        &mut self,
        group_jid: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !can_set_member_label(group_jid, self.can_send())
            || self.client().is_none()
            || self.member_label_dialog.is_some()
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
        let label = input(crate::tr!(MemberLabelPlaceholder), window, cx);
        label.update(cx, |input, cx| input.focus(window, cx));
        self.member_label_dialog = Some(MemberLabelDialog {
            group_jid: group_jid.to_string(),
            label,
            request: None,
            awaiting: false,
        });
        cx.notify();
    }

    /// Close the form, including any answer still on its way. Dropping the
    /// receiver is not needed: the id check below discards a stale answer, and
    /// the session's pending sweep retires one nobody is listening for.
    pub(super) fn cancel_member_label_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if self.member_label_dialog.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    /// The other forms cannot all learn about this one inside this change, so
    /// the root checks on the way down: one opening over the label form makes
    /// the label form the layer that goes away. The surfaces stay mutually
    /// exclusive without teaching an unrelated dialog a new owner.
    pub(super) fn retire_conflicting_member_label_dialog(&mut self, cx: &App) {
        let conflict = self.member_label_dialog.is_some()
            && (self.call_link_dialog.is_some()
                || self.event_dialog.is_some()
                || self.channel_poll_dialog.is_some()
                || self.community_dialog.is_some()
                || self.delete_confirmation.is_some()
                || self.paste_preview.is_some()
                || self.edit_draft.is_some()
                || self.viewer.read(cx).showing().is_some());
        if conflict {
            self.member_label_dialog = None;
        }
    }

    /// Validate locally, remember which answer belongs to this submission,
    /// then send the label exactly as validated.
    pub(super) fn submit_member_label_dialog(&mut self, cx: &mut Context<Self>) {
        let (group_jid, typed) = {
            let Some(dialog) = self.member_label_dialog.as_ref() else {
                return;
            };
            if dialog.awaiting || dialog.request.is_some() {
                return;
            }
            (
                dialog.group_jid.clone(),
                dialog.label.read(cx).value().to_string(),
            )
        };
        let label = match validate_member_label(&typed) {
            Ok(label) => label,
            Err(MemberLabelError::TooLong) => {
                return self.notify_user(crate::tr!(MemberLabelTooLong), Tone::Problem, cx);
            }
            Err(MemberLabelError::Newline) => {
                return self.notify_user(crate::tr!(MemberLabelSingleLine), Tone::Problem, cx);
            }
        };
        let Some(client) = self.client_handle() else {
            return self.notify_user(crate::tr!(MemberLabelFailed), Tone::Problem, cx);
        };
        let request_id = Self::next_local_id("member-label");
        {
            let Some(dialog) = self.member_label_dialog.as_mut() else {
                return;
            };
            dialog.awaiting = true;
            dialog.request = Some(request_id.clone());
        }
        cx.notify();

        let answer = client.set_group_member_label(&group_jid, &label);
        let cleared = label.is_empty();
        cx.spawn(async move |entity, cx| {
            let outcome = match answer.await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(failure)) => Err(failure),
                Err(_) => Err(crate::session::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            let _ = entity.update(cx, |app, cx| {
                {
                    // A cancelled form's answer, or one from a form that was
                    // replaced while this one was in flight, owns nothing here.
                    let Some(dialog) = app.member_label_dialog.as_mut() else {
                        return;
                    };
                    if dialog.request.as_deref() != Some(request_id.as_str()) {
                        return;
                    }
                    dialog.awaiting = false;
                    dialog.request = None;
                }
                match outcome {
                    Ok(()) => {
                        app.member_label_dialog = None;
                        app.notify_user(
                            if cleared {
                                crate::tr!(MemberLabelCleared)
                            } else {
                                crate::tr!(MemberLabelSaved)
                            },
                            Tone::Info,
                            cx,
                        );
                    }
                    Err(failure) => app.notify_user(
                        what_went_wrong(crate::tr!(MemberLabelFailed), &failure),
                        Tone::Problem,
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn input(
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<WhatsAppApp>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

/// The form's own layer, borrowing the deletion modal's focus handle: only one
/// of these surfaces is ever up, and `sync_overlay_focus` already routes that
/// handle's keyboard.
pub(super) fn render_member_label_dialog(
    dialog: &MemberLabelDialog,
    app: Entity<WhatsAppApp>,
    focus: &FocusHandle,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let cancel = app.clone();
    let submit = app;
    let clearing = dialog.label.read(cx).value().trim().is_empty();

    div()
        .id("member-label-modal")
        .debug_selector(|| "member-label-modal".into())
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
                .id("member-label-form")
                .flex()
                .flex_col()
                .w_full()
                .max_w(metrics.bubble_max_width())
                .p(metrics.space_xxl())
                .rounded(metrics.radius_xl())
                .bg(cx.theme().background)
                .gap(metrics.space_lg())
                .child(
                    div()
                        .text_size(metrics.text_title())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(crate::tr!(MemberLabelTitle)),
                )
                .child(field(
                    crate::tr!(MemberLabelHint).into(),
                    &dialog.label,
                    metrics,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(metrics.space_md())
                        .child(
                            Button::new("member-label-cancel")
                                .label(crate::tr!(Cancel))
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |app, cx| {
                                        app.cancel_member_label_dialog(cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("member-label-submit")
                                .primary()
                                .label(if clearing {
                                    crate::tr!(MemberLabelClear)
                                } else {
                                    crate::tr!(MemberLabelSave)
                                })
                                .disabled(dialog.awaiting)
                                .on_click(move |_, _, cx| {
                                    submit.update(cx, |app, cx| {
                                        app.submit_member_label_dialog(cx);
                                    });
                                }),
                        ),
                ),
        )
        .focus_trap("member-label-trap", focus)
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
    fn empty_input_is_a_clear_not_a_validation_error() {
        assert_eq!(validate_member_label(""), Ok(String::new()));
        assert_eq!(validate_member_label("   \t "), Ok(String::new()));
        assert_eq!(
            validate_member_label("  synthetic label  "),
            Ok("synthetic label".to_string())
        );
        assert_eq!(validate_member_label("🚀"), Ok("🚀".to_string()));
    }

    #[test]
    fn an_overlong_or_multiline_label_is_refused_before_a_request() {
        let at_limit = "x".repeat(MAX_MEMBER_LABEL_CHARS);
        assert_eq!(validate_member_label(&at_limit), Ok(at_limit));
        assert_eq!(
            validate_member_label(&"x".repeat(MAX_MEMBER_LABEL_CHARS + 1)),
            Err(MemberLabelError::TooLong)
        );
        assert_eq!(
            validate_member_label("line one\nline two"),
            Err(MemberLabelError::Newline)
        );
    }

    #[test]
    fn the_label_action_is_group_only_and_needs_a_live_session() {
        const GROUP: &str = "155555500001@g.us";
        assert!(can_set_member_label(GROUP, true));
        assert!(!can_set_member_label(GROUP, false));
        assert!(!can_set_member_label("155555500002@s.whatsapp.net", true));
        assert!(!can_set_member_label("status@broadcast", true));
        assert!(!can_set_member_label("not a jid", true));
    }

    /// No account data: a local Unix socket stands in for the daemon, and only
    /// the frames the form wrote are read from the other end.
    #[cfg(target_os = "linux")]
    #[gpui::test]
    fn submitting_sends_the_exact_label_and_an_empty_label_for_clear(
        cx: &mut gpui::TestAppContext,
    ) {
        use portable_atomic::AtomicU64;
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixListener;
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        const GROUP: &str = "155555500001@g.us";

        fn read_frame(reader: &mut impl BufRead) -> oxidezap_ipc::Request {
            let mut line = String::new();
            let bytes = reader.read_line(&mut line).expect("a frame from the peer");
            assert!(bytes > 0, "the synthetic daemon wrote no frame");
            serde_json::from_str(&line).expect("a client frame")
        }

        /// A window also says hello and asks for its first chat page when it
        /// opens; those are startup, not what this form sent. Everything else
        /// on the wire is a failure of this test, not something to skip.
        fn read_label_request(reader: &mut impl BufRead) -> (bool, String, String) {
            loop {
                let frame = read_frame(reader);
                let correlated = frame.id.is_some();
                match frame.request {
                    oxidezap_ipc::ClientRequest::SetGroupMemberLabel { group_jid, label } => {
                        return (correlated, group_jid, label);
                    }
                    oxidezap_ipc::ClientRequest::Hello { .. }
                    | oxidezap_ipc::ClientRequest::LoadChats(_) => {}
                    other => panic!("an unexpected request was sent: {other:?}"),
                }
            }
        }

        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "oxidezap-member-label-{}-{}.sock",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&path).expect("a synthetic socket");
        let endpoint = oxidezap_ipc::Endpoint::connect_at(&path).expect("a client endpoint");
        let (mut peer, _) = listener.accept().expect("the client connected");
        peer.set_read_timeout(Some(Duration::from_secs(2)))
            .expect("a read deadline");
        let client = crate::session::Session::recording_test_session(endpoint);

        cx.update(|cx| {
            cx.set_app_identity("org.oxidezap.test", "OxideZap Test");
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
        let mut app_entity = None;
        let window = cx.open_window(gpui::size(gpui::px(800.), gpui::px(600.)), |window, cx| {
            let app = cx.new(|cx| {
                let mut app = WhatsAppApp::new(cx);
                app.app_state = AppState::Connected;
                app.install_test_session(Some(client));
                app
            });
            app_entity = Some(app.clone());
            gpui_component::Root::new(app, window, cx)
        });
        let app = app_entity.expect("the app was created");
        let mut reader = BufReader::new(&mut peer);
        let _hello = read_frame(&mut reader);

        window
            .update(cx, |_, window, cx| {
                app.update(cx, |app, cx| {
                    app.begin_member_label_dialog(GROUP, window, cx);
                    let input = app
                        .member_label_dialog
                        .as_ref()
                        .expect("a group can open the form")
                        .label
                        .clone();
                    input.update(cx, |state, cx| state.set_value("VIP 🚀", window, cx));
                    app.submit_member_label_dialog(cx);
                });
            })
            .expect("the window accepted the submission");

        let (correlated, group_jid, label) = read_label_request(&mut reader);
        assert!(correlated, "a mutation is correlated");
        assert_eq!(group_jid, GROUP);
        assert_eq!(label, "VIP 🚀");

        // Reopening after a cancel makes a new form. Its empty box is the
        // clear request, which keeps the empty string on the wire.
        window
            .update(cx, |_, window, cx| {
                app.update(cx, |app, cx| {
                    assert!(app.cancel_member_label_dialog(cx));
                    app.begin_member_label_dialog(GROUP, window, cx);
                    let input = app
                        .member_label_dialog
                        .as_ref()
                        .expect("the form reopened")
                        .label
                        .clone();
                    input.update(cx, |state, cx| state.set_value("", window, cx));
                    app.submit_member_label_dialog(cx);
                });
            })
            .expect("the window accepted the clear");

        let (_, group_jid, label) = read_label_request(&mut reader);
        assert_eq!(group_jid, GROUP);
        assert_eq!(label, "");
        let _ = std::fs::remove_file(path);
    }
}
