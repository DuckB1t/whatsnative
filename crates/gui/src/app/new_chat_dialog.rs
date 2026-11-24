//! The new chat dialog, built from state this window already holds.
//!
//! There is no GUI IPC request that creates a chat by address. This surface
//! therefore does two honest things: it filters conversations already loaded
//! from the daemon, and it asks the existing username query to resolve a
//! handle. A resolved address is shown as protocol data. It opens a chat only
//! when that address is already one of the loaded conversations. Nothing here
//! invents a contact, sends a message or mutates an account.

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement as _, Styled, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::FocusTrapElement as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Disableable as _, v_flex};
use oxidezap_core::{
    USERNAME_MAX_LENGTH, USERNAME_MIN_LENGTH, UsernameLookupResult, normalize_username,
};

use super::chat_row;
use super::username_lookup::LookupOwner;
use super::{ChatOpen, WhatsAppApp};
use crate::components::Avatar;
use crate::components::parts;
use crate::theme::Metrics;

/// One actionable row in the known chats section.
///
/// The name is the same string the sidebar uses, and the address is the raw JID
/// the daemon supplied. Neither is guessed from the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KnownChat {
    pub(super) jid: String,
    pub(super) name: String,
}

impl KnownChat {
    /// Match the displayed name or the raw address, without caring about case.
    pub(super) fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.name.to_lowercase().contains(&query)
            || self.jid.to_lowercase().contains(&query)
    }
}

/// What the username section can honestly say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NewChatLookupView {
    /// Nothing has been asked yet.
    Guidance,
    /// The existing lookup request is still out.
    Busy,
    /// The daemon refused or lost the request.
    Failed,
    KeyRequired,
    NotFound,
    InvalidUsername,
    /// The protocol answered with an address and whatever fields it supplied.
    Found {
        jid: String,
        pn_jid: Option<String>,
        username: Option<String>,
        name: Option<String>,
        verified_name: Option<String>,
        /// Whether this address is already a loaded chat this window can open.
        openable: bool,
    },
}

impl NewChatLookupView {
    /// One sentence per outcome, from the shared EN and PT catalog.
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Guidance => crate::tr!(UsernameLookupGuidance),
            Self::Busy => crate::tr!(LookingUpUsername),
            Self::Failed => crate::tr!(NewChatLookupFailed),
            Self::KeyRequired => crate::tr!(UsernameKeyRequired),
            Self::NotFound => crate::tr!(UsernameNotFound),
            Self::InvalidUsername => crate::tr!(UsernameInvalid),
            Self::Found { .. } => crate::tr!(NewChatFound),
        }
    }
}

/// Owned facts for one frame of the dialog. The render helper never borrows an
/// app, a chat or a lookup result out of the application state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NewChatModel {
    pub(super) known: Vec<KnownChat>,
    pub(super) lookup: NewChatLookupView,
    pub(super) busy: bool,
}

impl NewChatModel {
    /// Project the current query and lookup answer into owned rows.
    pub(super) fn build(
        app: &WhatsAppApp,
        query: &str,
        result: Option<&UsernameLookupResult>,
        busy: bool,
        failed: bool,
    ) -> Self {
        let known = app
            .chats
            .iter()
            .filter(|chat| !chat.is_status)
            .map(|chat| KnownChat {
                jid: chat.jid.clone(),
                name: chat_row::display_name(&chat.name, app.is_own_number(&chat.jid)),
            })
            .filter(|row| row.matches(query))
            .collect();
        Self {
            known,
            lookup: Self::lookup(app, result, busy, failed),
            busy,
        }
    }

    /// The lookup answer, with the open action decided by `has_chat` alone.
    fn lookup(
        app: &WhatsAppApp,
        result: Option<&UsernameLookupResult>,
        busy: bool,
        failed: bool,
    ) -> NewChatLookupView {
        if busy {
            return NewChatLookupView::Busy;
        }
        if failed {
            return NewChatLookupView::Failed;
        }
        match result {
            None => NewChatLookupView::Guidance,
            Some(UsernameLookupResult::Found(found)) => NewChatLookupView::Found {
                jid: found.jid.clone(),
                pn_jid: found.pn_jid.clone(),
                username: found.username.clone(),
                name: found.name.clone(),
                verified_name: found.verified_name.clone(),
                openable: app.has_chat(&found.jid),
            },
            Some(UsernameLookupResult::KeyRequired { .. }) => NewChatLookupView::KeyRequired,
            Some(UsernameLookupResult::NotFound) => NewChatLookupView::NotFound,
            Some(UsernameLookupResult::InvalidUsername) => NewChatLookupView::InvalidUsername,
        }
    }
}

/// The dialog's own state, separate from the Settings contact pane.
pub(super) struct NewChatDialog {
    pub(super) query: Entity<InputState>,
    pub(super) username: Entity<InputState>,
    pub(super) key: Entity<InputState>,
    pub(super) result: Option<UsernameLookupResult>,
    pub(super) busy: bool,
    pub(super) failed: bool,
}

impl NewChatDialog {
    fn adopt(&mut self, result: UsernameLookupResult) {
        self.result = Some(result);
        self.busy = false;
        self.failed = false;
    }

    fn failed(&mut self) {
        self.busy = false;
        self.failed = true;
    }
}

impl WhatsAppApp {
    /// Open the dialog. Every other modal surface is mutually exclusive with
    /// it, so only one of them can own the keyboard at a time.
    pub(crate) fn begin_new_chat_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send()
            || self.new_chat_dialog.is_some()
            || self.community_dialog.is_some()
            || self.event_dialog.is_some()
            || self.channel_poll_dialog.is_some()
            || self.delete_confirmation.is_some()
            || self.paste_preview.is_some()
            || self.edit_draft.is_some()
            || self.forward_picker.is_some()
            || self.call_link_dialog.is_some()
            || self.member_label_dialog.is_some()
            || self.viewer.read(cx).showing().is_some()
        {
            return;
        }
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(crate::tr!(NewChatFilter)));
        query.update(cx, |input, cx| input.focus(window, cx));
        self.new_chat_dialog = Some(NewChatDialog {
            query,
            username: cx
                .new(|cx| InputState::new(window, cx).placeholder(crate::tr!(UsernameHandle))),
            key: cx
                .new(|cx| InputState::new(window, cx).placeholder(crate::tr!(UsernameKeyOptional))),
            result: None,
            busy: false,
            failed: false,
        });
        cx.notify();
    }

    /// Close the dialog. Returns whether there was one to close, so one Escape
    /// spends itself here and cannot also act on the surface underneath.
    pub(super) fn cancel_new_chat_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        // A lookup that is still out now belongs to a dialog that no longer
        // exists. Mark it abandoned so its answer is dropped, not shown in
        // Settings, which never asked for it.
        if self.lookup_owner == LookupOwner::NewChat {
            self.lookup_owner = LookupOwner::Abandoned;
        }
        if self.new_chat_dialog.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    /// Open an address only when it names a chat this window already holds.
    /// No creation path exists here, so an absent address changes nothing.
    pub(super) fn open_new_chat_target(
        &mut self,
        jid: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_chat(jid) {
            return;
        }
        let jid = jid.to_string();
        if self.lookup_owner == LookupOwner::NewChat {
            self.lookup_owner = LookupOwner::Abandoned;
        }
        self.new_chat_dialog = None;
        self.select_chat(jid, ChatOpen::ToCompose, window, cx);
    }

    /// Ask the existing username lookup to resolve what was typed.
    pub(super) fn submit_new_chat_lookup(&mut self, cx: &mut Context<Self>) {
        if self.username_lookup_busy() || self.lookup_owner != LookupOwner::Settings {
            return;
        }
        let Some(dialog) = self.new_chat_dialog.as_ref() else {
            return;
        };
        let typed = dialog.username.read(cx).value().to_string();
        let typed_key = dialog.key.read(cx).value().to_string();
        let Some(username) = normalize_username(&typed) else {
            let message = crate::tr!(invalid_username, USERNAME_MIN_LENGTH, USERNAME_MAX_LENGTH);
            self.notify_user(message, super::notices::Tone::Problem, cx);
            if let Some(dialog) = self.new_chat_dialog.as_mut() {
                dialog.adopt(UsernameLookupResult::InvalidUsername);
            }
            cx.notify();
            return;
        };
        let typed_key = typed_key.trim();
        let key = (!typed_key.is_empty()).then(|| typed_key.to_string());
        let Some(client) = self.client().map(|client| client.handle()) else {
            if let Some(dialog) = self.new_chat_dialog.as_mut() {
                dialog.failed();
            }
            cx.notify();
            return;
        };
        self.lookup_owner = LookupOwner::NewChat;
        if let Some(dialog) = self.new_chat_dialog.as_mut() {
            dialog.busy = true;
            dialog.failed = false;
        }
        client.lookup_username(username, key);
        cx.notify();
    }

    /// Keep the typed answer beside the dialog that asked for it. The caller
    /// has already decided that this answer belongs here.
    pub(super) fn adopt_new_chat_lookup(
        &mut self,
        result: UsernameLookupResult,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.new_chat_dialog.as_mut() {
            dialog.adopt(result);
            cx.notify();
        }
    }

    /// Say the lookup did not answer, in the dialog that asked.
    pub(super) fn new_chat_lookup_failed(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.new_chat_dialog.as_mut() {
            dialog.failed();
            cx.notify();
        }
    }
}

/// Owned inputs for one frame of the modal.
pub(super) struct NewChatProps {
    pub(super) model: NewChatModel,
    pub(super) query: Entity<InputState>,
    pub(super) username: Entity<InputState>,
    pub(super) key: Entity<InputState>,
    pub(super) entity: Entity<WhatsAppApp>,
    pub(super) metrics: Metrics,
}

/// Render the modal with this crate's own surfaces and theme tokens.
pub(super) fn render_new_chat_dialog(
    props: NewChatProps,
    focus: &FocusHandle,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = props.metrics;
    let cancel = props.entity.clone();
    let submit = props.entity.clone();
    let rows = props.model.known.clone();
    let busy = props.model.busy;
    let lookup = props.model.lookup.clone();

    div()
        .id("new-chat-modal")
        .debug_selector(|| "new-chat-modal".into())
        .track_focus(focus)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(metrics.space_xxl())
        .bg(parts::scrim(cx).opacity(0.92))
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("new-chat-form")
                .flex()
                .flex_col()
                .w_full()
                .max_w(metrics.bubble_max_width())
                .max_h_full()
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
                        .child(crate::tr!(NewChat)),
                )
                .child(
                    v_flex()
                        .gap(metrics.space_sm())
                        .child(section_title(crate::tr!(NewChatKnownTitle), metrics, cx))
                        .child(note(crate::tr!(NewChatKnownHint), metrics, cx))
                        .child(Input::new(&props.query).w_full())
                        .children(
                            rows.iter()
                                .map(|row| known_row(row, props.entity.clone(), metrics, cx)),
                        )
                        .when(rows.is_empty(), |el| {
                            el.child(note(crate::tr!(NewChatNoMatches), metrics, cx))
                        }),
                )
                .child(
                    v_flex()
                        .gap(metrics.space_sm())
                        .child(section_title(crate::tr!(FindByUsername), metrics, cx))
                        .child(note(crate::tr!(NewChatLookupHint), metrics, cx))
                        .child(Input::new(&props.username).w_full())
                        .child(Input::new(&props.key).w_full())
                        .child(lookup_answer(&lookup, props.entity.clone(), metrics, cx))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(metrics.space_md())
                                .child(
                                    Button::new("new-chat-lookup")
                                        .label(crate::tr!(LookUpUsername))
                                        .outline()
                                        .disabled(busy)
                                        .when(!busy, |button| button.cursor_pointer())
                                        .on_click(move |_, _, cx| {
                                            submit.update(cx, |app, cx| {
                                                app.submit_new_chat_lookup(cx);
                                            });
                                        }),
                                )
                                .when(busy, |el| {
                                    el.child(note(crate::tr!(LookingUpUsername), metrics, cx))
                                }),
                        ),
                )
                .child(
                    div().flex().justify_end().gap(metrics.space_md()).child(
                        Button::new("new-chat-cancel")
                            .label(crate::tr!(Cancel))
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |app, cx| {
                                    app.cancel_new_chat_dialog(cx);
                                });
                            }),
                    ),
                ),
        )
        .focus_trap("new-chat-trap", focus)
}

fn section_title(text: &'static str, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .text_size(metrics.text_secondary())
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(cx.theme().foreground)
        .child(text)
}

fn note(text: &'static str, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .text_size(metrics.text_small())
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

fn protocol_line(
    label: &'static str,
    value: &str,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    div()
        .flex()
        .gap(metrics.space_sm())
        .text_size(metrics.text_small())
        .child(
            div()
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .min_w_0()
                .text_color(cx.theme().foreground)
                .child(value.to_string()),
        )
}

fn known_row(
    row: &KnownChat,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let open = entity.clone();
    let jid = row.jid.clone();
    let name = row.name.clone();
    div()
        .id(SharedString::from(format!("new-chat-row-{jid}")))
        .flex()
        .items_center()
        .gap(metrics.space_md())
        .p(metrics.space_sm())
        .rounded(metrics.radius_md())
        .bg(cx.theme().secondary)
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            let jid = jid.clone();
            open.update(cx, |app, cx| {
                app.open_new_chat_target(&jid, window, cx);
            });
        })
        .child(
            Avatar::new(row.jid.clone(), &row.name, metrics.avatar_inline())
                .on(cx.theme().secondary),
        )
        .child(
            v_flex()
                .min_w_0()
                .gap(metrics.space_xxs())
                .child(
                    div()
                        .text_size(metrics.text_secondary())
                        .text_color(cx.theme().foreground)
                        .child(name),
                )
                .child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(metrics.text_micro())
                        .text_color(cx.theme().muted_foreground)
                        .child(row.jid.clone()),
                ),
        )
}

fn lookup_answer(
    view: &NewChatLookupView,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let base = v_flex()
        .gap(metrics.space_xxs())
        .child(note(view.label(), metrics, cx));
    match view {
        NewChatLookupView::Found {
            jid,
            pn_jid,
            username,
            name,
            verified_name,
            openable,
        } => {
            let mut answer = base.child(protocol_line(crate::tr!(JidLabel), jid, metrics, cx));
            if let Some(pn_jid) = pn_jid.as_deref() {
                answer = answer.child(protocol_line(crate::tr!(PhoneNumber), pn_jid, metrics, cx));
            }
            if let Some(username) = username.as_deref() {
                answer = answer.child(protocol_line(
                    crate::tr!(UsernameLabel),
                    username,
                    metrics,
                    cx,
                ));
            }
            if let Some(name) = name.as_deref() {
                answer = answer.child(protocol_line(crate::tr!(DisplayName), name, metrics, cx));
            }
            if let Some(verified_name) = verified_name.as_deref() {
                answer = answer.child(protocol_line(
                    crate::tr!(VerifiedNameLabel),
                    verified_name,
                    metrics,
                    cx,
                ));
            }
            if !*openable {
                return answer.child(note(crate::tr!(NewChatResolvedOnly), metrics, cx));
            }
            let open = entity.clone();
            let address = jid.clone();
            answer.child(
                Button::new("new-chat-open-resolved")
                    .label(crate::tr!(NewChatOpenChat))
                    .primary()
                    .on_click(move |_, window, cx| {
                        let address = address.clone();
                        open.update(cx, |app, cx| {
                            app.open_new_chat_target(&address, window, cx);
                        });
                    }),
            )
        }
        _ => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Entity;
    use oxidezap_core::{Chat, ResolvedUsername};
    use std::sync::Arc;

    const FIRST: &str = "synthetic-first@example.invalid";
    const SECOND: &str = "synthetic-second@example.invalid";
    const MISSING: &str = "synthetic-missing@example.invalid";

    fn init(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
    }

    fn fixture(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<WhatsAppApp>, &mut gpui::VisualTestContext) {
        init(cx);
        let mut app = None;
        let (_, visual) = cx.add_window_view(|window, cx| {
            let app_entity = cx.new(|cx| {
                let mut app = WhatsAppApp::new(cx);
                app.app_state = super::super::AppState::Connected;
                let mut first = Chat::new(FIRST.to_string());
                first.name = "Synthetic First".to_string();
                let mut second = Chat::new(SECOND.to_string());
                second.name = "Synthetic Second".to_string();
                app.chats = vec![Arc::new(first), Arc::new(second)];
                app
            });
            app = Some(app_entity.clone());
            gpui_component::Root::new(app_entity, window, cx)
        });
        (app.expect("the app entity"), visual)
    }

    fn found(jid: &str) -> UsernameLookupResult {
        UsernameLookupResult::Found(ResolvedUsername {
            jid: jid.to_string(),
            ..Default::default()
        })
    }

    #[test]
    fn known_rows_filter_on_the_displayed_name_and_the_raw_address() {
        let rows = [
            KnownChat {
                jid: FIRST.to_string(),
                name: "Synthetic First".to_string(),
            },
            KnownChat {
                jid: SECOND.to_string(),
                name: "Synthetic Second".to_string(),
            },
        ];
        let by_name: Vec<_> = rows.iter().filter(|row| row.matches("FIRST")).collect();
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].jid, FIRST);
        let by_address: Vec<_> = rows
            .iter()
            .filter(|row| row.matches("second@example.INVALID"))
            .collect();
        assert_eq!(by_address.len(), 1);
        assert_eq!(by_address[0].jid, SECOND);
        assert_eq!(rows.iter().filter(|row| row.matches("missing")).count(), 0);
    }

    #[gpui::test]
    fn the_known_section_lists_only_loaded_chats_that_match(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                let second = NewChatModel::build(app, "SECOND", None, false, false);
                assert_eq!(
                    second.known,
                    vec![KnownChat {
                        jid: SECOND.to_string(),
                        name: "Synthetic Second".to_string(),
                    }]
                );
                let by_address =
                    NewChatModel::build(app, "first@example.invalid", None, false, false);
                assert_eq!(
                    by_address.known,
                    vec![KnownChat {
                        jid: FIRST.to_string(),
                        name: "Synthetic First".to_string(),
                    }]
                );
                let none = NewChatModel::build(app, "missing", None, false, false);
                assert!(none.known.is_empty());
                assert_eq!(app.chats.len(), 2);
            });
        });
    }

    #[test]
    fn an_unknown_username_is_normalized_or_refused_locally() {
        assert_eq!(
            normalize_username(" @synthetic.handle ").as_deref(),
            Some("synthetic.handle")
        );
        assert_eq!(normalize_username("ab"), None);
        assert_eq!(normalize_username(" @ "), None);
        assert_eq!(
            normalize_username(&"a".repeat(USERNAME_MAX_LENGTH + 1)),
            None
        );
        assert_eq!(
            normalize_username(&"a".repeat(USERNAME_MIN_LENGTH)).as_deref(),
            Some("aaa")
        );
    }

    #[test]
    fn typed_lookup_answers_have_their_own_sentence() {
        assert_eq!(
            NewChatLookupView::Guidance.label(),
            crate::tr!(UsernameLookupGuidance)
        );
        assert_eq!(
            NewChatLookupView::Busy.label(),
            crate::tr!(LookingUpUsername)
        );
        assert_eq!(
            NewChatLookupView::Failed.label(),
            crate::tr!(NewChatLookupFailed)
        );
        assert_eq!(
            NewChatLookupView::KeyRequired.label(),
            crate::tr!(UsernameKeyRequired)
        );
        assert_eq!(
            NewChatLookupView::NotFound.label(),
            crate::tr!(UsernameNotFound)
        );
        assert_eq!(
            NewChatLookupView::InvalidUsername.label(),
            crate::tr!(UsernameInvalid)
        );
        assert_eq!(
            NewChatLookupView::Found {
                jid: FIRST.to_string(),
                pn_jid: None,
                username: None,
                name: None,
                verified_name: None,
                openable: false,
            }
            .label(),
            crate::tr!(NewChatFound)
        );
    }

    #[gpui::test]
    fn a_found_answer_opens_only_an_address_already_loaded(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let matching = NewChatModel::build(app, "", Some(&found(FIRST)), false, false);
                assert_eq!(
                    matching.lookup,
                    NewChatLookupView::Found {
                        jid: FIRST.to_string(),
                        pn_jid: None,
                        username: None,
                        name: None,
                        verified_name: None,
                        openable: true,
                    }
                );
                app.open_new_chat_target(FIRST, window, cx);
                assert_eq!(app.selected_chat.as_deref(), Some(FIRST));

                let absent = NewChatModel::build(app, "", Some(&found(MISSING)), false, false);
                assert_eq!(
                    absent.lookup,
                    NewChatLookupView::Found {
                        jid: MISSING.to_string(),
                        pn_jid: None,
                        username: None,
                        name: None,
                        verified_name: None,
                        openable: false,
                    }
                );
                let before = app.chats.len();
                app.open_new_chat_target(MISSING, window, cx);
                assert_eq!(app.chats.len(), before);
                assert_eq!(app.selected_chat.as_deref(), Some(FIRST));
            });
        });
    }

    #[gpui::test]
    fn an_invalid_handle_stays_in_the_dialog_without_a_request(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_new_chat_dialog(window, cx);
                let dialog = app.new_chat_dialog.as_ref().expect("the dialog is open");
                dialog
                    .username
                    .update(cx, |input, cx| input.set_value("ab", window, cx));
                app.submit_new_chat_lookup(cx);
                assert_eq!(
                    app.new_chat_dialog.as_ref().unwrap().result,
                    Some(UsernameLookupResult::InvalidUsername)
                );
                assert_eq!(app.lookup_owner, LookupOwner::Settings);
                assert!(!app.username_lookup_busy());
            });
        });
        // The invalid-handle path posts a notice. Park background work so the
        // notice sweeper cannot outlive the test scheduler during teardown.
        cx.run_until_parked();
    }

    #[gpui::test]
    fn an_answer_goes_to_the_dialog_that_asked(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_new_chat_dialog(window, cx);
                app.lookup_owner = LookupOwner::NewChat;
                app.adopt_username_lookup(found(FIRST), cx);
                assert_eq!(
                    app.new_chat_dialog.as_ref().unwrap().result,
                    Some(found(FIRST))
                );
                assert!(app.username_lookup_result().is_none());
                assert_eq!(app.lookup_owner, LookupOwner::Settings);
            });
        });
    }

    #[gpui::test]
    fn an_answer_for_a_closed_dialog_is_dropped(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_new_chat_dialog(window, cx);
                app.lookup_owner = LookupOwner::NewChat;
                assert!(app.cancel_new_chat_dialog(cx));
                assert_eq!(app.lookup_owner, LookupOwner::Abandoned);
                app.adopt_username_lookup(found(FIRST), cx);
                assert!(app.username_lookup_result().is_none());
                assert_eq!(app.lookup_owner, LookupOwner::Settings);
            });
        });
    }

    #[gpui::test]
    fn opening_and_cancelling_the_dialog_leaves_no_stale_state(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.selected_chat = Some(FIRST.to_string());
                app.chat_info_jid = Some(FIRST.to_string());
                app.begin_new_chat_dialog(window, cx);
                assert!(app.new_chat_dialog.is_some());
                app.lookup_owner = LookupOwner::NewChat;
                app.adopt_username_lookup(found(FIRST), cx);
                assert!(app.new_chat_dialog.as_ref().unwrap().result.is_some());
                assert_eq!(app.lookup_owner, LookupOwner::Settings);
                assert!(app.cancel_new_chat_dialog(cx));
                assert!(app.new_chat_dialog.is_none());
                assert!(!app.cancel_new_chat_dialog(cx));
                assert_eq!(app.selected_chat.as_deref(), Some(FIRST));
                assert_eq!(app.chat_info_jid.as_deref(), Some(FIRST));
            });
        });
    }

    #[gpui::test]
    fn one_escape_closes_only_the_dialog(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.chat_info_jid = Some(FIRST.to_string());
                app.begin_new_chat_dialog(window, cx);
                assert!(app.new_chat_dialog.is_some());
                app.close_overlay(window, cx);
                assert!(app.new_chat_dialog.is_none());
                assert_eq!(app.chat_info_jid.as_deref(), Some(FIRST));
            });
        });
    }

    #[gpui::test]
    fn clearing_the_account_or_losing_the_session_closes_the_dialog(cx: &mut gpui::TestAppContext) {
        let (app, cx) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_new_chat_dialog(window, cx);
                assert!(app.new_chat_dialog.is_some());
                app.lookup_owner = LookupOwner::NewChat;
                app.leave_connected_view(Some(window), cx);
                assert!(app.new_chat_dialog.is_none());
                assert_eq!(app.lookup_owner, LookupOwner::Settings);
                app.begin_new_chat_dialog(window, cx);
                assert!(app.new_chat_dialog.is_some());
                app.lookup_owner = LookupOwner::NewChat;
                app.clear_account_view(window, cx);
                assert!(app.new_chat_dialog.is_none());
                assert_eq!(app.lookup_owner, LookupOwner::Settings);
            });
        });
    }
}
