//! The form behind native community operations.
//!
//! One dialog, four operations. The mode selector is the navigation: the
//! fields that do not belong to the chosen operation are not drawn, so a
//! person cannot send a link request carrying a half-filled name. Every field
//! stays plain text until submit, where a pure builder turns it into a domain
//! request or names the one thing that is still missing. Only synthetic
//! addresses appear in the tests; nothing here touches an account.

use gpui::{
    App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div,
};
use gpui_component::FocusTrapElement as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme as _, Sizable as _, v_flex};
use oxidezap_core::{
    CommunityDraft, CommunityOperationResult, CommunityVisibility, CommunityWarningStep,
    JoinSubgroupRequest, LinkSubgroupTarget, LinkSubgroupsRequest, SubgroupDraft,
};

use super::notices::Tone;
use super::*;

/// Which of the four operations the form is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CommunityOperation {
    #[default]
    Create,
    CreateSubgroup,
    Link,
    Join,
}

/// The boxes a person fills before a community operation goes to the daemon.
pub(super) struct CommunityDialog {
    operation: CommunityOperation,
    name: Entity<InputState>,
    description: Entity<InputState>,
    parent_jid: Entity<InputState>,
    participants: Entity<InputState>,
    community_jid: Entity<InputState>,
    groups: Entity<InputState>,
    subgroup_jid: Entity<InputState>,
    closed: bool,
    allow_non_admin: bool,
    create_general_chat: bool,
    hidden: bool,
}

/// The one thing that stopped a form from building a request. Kept as data so
/// the submit handler can name it from the catalog rather than passing a bare
/// string through from a test or a helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommunityFormError {
    EmptyName,
    NoParent,
    MissingCommunity,
    BlankSubgroup,
    GroupsAbsent,
}

/// A result grouped into the sentences worth showing. Pure so the report is
/// testable without a window or a language setting.
#[derive(Debug, Default, PartialEq, Eq)]
struct CommunityOutcome {
    created: usize,
    linked: usize,
    joined: bool,
    failed: Vec<(String, u32)>,
    description_warnings: usize,
    approval_pending: usize,
}

impl CommunityOutcome {
    fn from_result(result: &CommunityOperationResult) -> Self {
        let mut outcome = Self {
            created: result.groups.len(),
            linked: result.linked_jids.len(),
            ..Self::default()
        };
        outcome.failed = result
            .failed_groups
            .iter()
            .map(|failure| (failure.jid.clone(), failure.code))
            .collect();
        for warning in &result.warnings {
            match warning.step {
                CommunityWarningStep::SetDescription | CommunityWarningStep::LinkSubgroup => {
                    outcome.description_warnings += 1;
                }
                CommunityWarningStep::JoinApprovalPending => outcome.approval_pending += 1,
            }
        }
        outcome
    }
}

/// Split a multi-line address box. Blank lines and surrounding space are not
/// addresses, so pasting a list from a spreadsheet cannot send an empty JID.
fn split_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Split a comma- or newline-separated box of addresses.
fn split_addresses(text: &str) -> Vec<String> {
    text.split([',', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn optional_text(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn visibility(hidden: bool) -> CommunityVisibility {
    if hidden {
        CommunityVisibility::Hidden
    } else {
        CommunityVisibility::Visible
    }
}

fn draft_community(
    name: &str,
    description: &str,
    closed: bool,
    allow_non_admin: bool,
    create_general_chat: bool,
) -> Result<CommunityDraft, CommunityFormError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CommunityFormError::EmptyName);
    }
    Ok(CommunityDraft {
        name: name.to_string(),
        description: optional_text(description),
        closed,
        allow_non_admin_sub_group_creation: allow_non_admin,
        create_general_chat,
    })
}

fn draft_subgroup(
    parent_jid: &str,
    name: &str,
    participants: &str,
    hidden: bool,
) -> Result<SubgroupDraft, CommunityFormError> {
    let name = name.trim();
    let parent_jid = parent_jid.trim();
    if name.is_empty() {
        return Err(CommunityFormError::EmptyName);
    }
    if parent_jid.is_empty() {
        return Err(CommunityFormError::NoParent);
    }
    Ok(SubgroupDraft {
        parent_jid: parent_jid.to_string(),
        name: name.to_string(),
        participants: split_addresses(participants),
        visibility: visibility(hidden),
    })
}

fn draft_link(
    community_jid: &str,
    groups: &str,
    hidden: bool,
) -> Result<LinkSubgroupsRequest, CommunityFormError> {
    let community_jid = community_jid.trim();
    if community_jid.is_empty() {
        return Err(CommunityFormError::MissingCommunity);
    }
    let listing = visibility(hidden);
    let targets: Vec<LinkSubgroupTarget> = split_lines(groups)
        .into_iter()
        .map(|jid| LinkSubgroupTarget {
            jid,
            visibility: listing,
        })
        .collect();
    if targets.is_empty() {
        return Err(CommunityFormError::GroupsAbsent);
    }
    Ok(LinkSubgroupsRequest {
        community_jid: community_jid.to_string(),
        groups: targets,
    })
}

fn draft_join(
    community_jid: &str,
    subgroup_jid: &str,
) -> Result<JoinSubgroupRequest, CommunityFormError> {
    let community_jid = community_jid.trim();
    let subgroup_jid = subgroup_jid.trim();
    if community_jid.is_empty() {
        return Err(CommunityFormError::MissingCommunity);
    }
    if subgroup_jid.is_empty() {
        return Err(CommunityFormError::BlankSubgroup);
    }
    Ok(JoinSubgroupRequest {
        community_jid: community_jid.to_string(),
        subgroup_jid: subgroup_jid.to_string(),
    })
}

fn new_field(
    placeholder: SharedString,
    window: &mut Window,
    cx: &mut Context<WhatsAppApp>,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
}

impl CommunityFormError {
    fn message(self) -> &'static str {
        match self {
            Self::EmptyName => crate::tr!(CommunityNameRequired),
            Self::NoParent => crate::tr!(CommunityParentRequired),
            Self::MissingCommunity => crate::tr!(CommunityJidRequired),
            Self::BlankSubgroup => crate::tr!(CommunitySubgroupRequired),
            Self::GroupsAbsent => crate::tr!(CommunityGroupsRequired),
        }
    }
}

impl WhatsAppApp {
    /// Open the form. The same focus handle serves every mutually exclusive
    /// modal, so a second one cannot be opened on top of this one.
    pub(crate) fn begin_community_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send()
            || self.community_dialog.is_some()
            || self.event_dialog.is_some()
            || self.delete_confirmation.is_some()
            || self.paste_preview.is_some()
            || self.edit_draft.is_some()
        {
            return;
        }
        let name = new_field(crate::tr!(CommunityName).into(), window, cx);
        name.update(cx, |input, cx| input.focus(window, cx));
        self.community_dialog = Some(CommunityDialog {
            operation: CommunityOperation::Create,
            name,
            description: new_field(crate::tr!(CommunityDescription).into(), window, cx),
            parent_jid: new_field(crate::tr!(CommunityParentJid).into(), window, cx),
            participants: new_field(crate::tr!(CommunityParticipants).into(), window, cx),
            community_jid: new_field(crate::tr!(CommunityJid).into(), window, cx),
            groups: new_field(crate::tr!(CommunityGroups).into(), window, cx),
            subgroup_jid: new_field(crate::tr!(CommunitySubgroupJid).into(), window, cx),
            // The library's own creation defaults, shown honestly rather than
            // hidden behind an acknowledgement.
            closed: true,
            allow_non_admin: false,
            create_general_chat: true,
            hidden: false,
        });
        cx.notify();
    }

    pub(super) fn cancel_community_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if self.community_dialog.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    /// Switch the operation. Only the fields for it are drawn, but every box
    /// keeps its text so switching away and back loses nothing.
    fn set_community_operation(&mut self, operation: CommunityOperation, cx: &mut Context<Self>) {
        if let Some(dialog) = self.community_dialog.as_mut() {
            dialog.operation = operation;
            cx.notify();
        }
    }

    fn toggle_community_flag(&mut self, flag: CommunityFlag, cx: &mut Context<Self>) {
        if let Some(dialog) = self.community_dialog.as_mut() {
            match flag {
                CommunityFlag::Closed => dialog.closed = !dialog.closed,
                CommunityFlag::AllowNonAdmin => dialog.allow_non_admin = !dialog.allow_non_admin,
                CommunityFlag::GeneralChat => {
                    dialog.create_general_chat = !dialog.create_general_chat;
                }
                CommunityFlag::Hidden => dialog.hidden = !dialog.hidden,
            }
            cx.notify();
        }
    }

    /// Validate locally, take the form down so a second press cannot send
    /// twice, then await the typed answer.
    pub(super) fn submit_community_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.community_dialog.as_ref() else {
            return;
        };
        let value = |field: &Entity<InputState>| field.read(cx).value().to_string();
        let name = value(&dialog.name);
        let description = value(&dialog.description);
        let parent_jid = value(&dialog.parent_jid);
        let participants = value(&dialog.participants);
        let community_jid = value(&dialog.community_jid);
        let groups = value(&dialog.groups);
        let subgroup_jid = value(&dialog.subgroup_jid);
        let (operation, closed, allow, general, hidden) = (
            dialog.operation,
            dialog.closed,
            dialog.allow_non_admin,
            dialog.create_general_chat,
            dialog.hidden,
        );

        let Some(client) = self.client.as_ref() else {
            self.notify_user(crate::tr!(CommunityCouldNotStart), Tone::Problem, cx);
            return;
        };

        let answer = match operation {
            CommunityOperation::Create => {
                match draft_community(&name, &description, closed, allow, general) {
                    Ok(draft) => client.create_community(draft),
                    Err(error) => return self.invalid_community_form(error, cx),
                }
            }
            CommunityOperation::CreateSubgroup => {
                match draft_subgroup(&parent_jid, &name, &participants, hidden) {
                    Ok(draft) => client.create_community_subgroup(draft),
                    Err(error) => return self.invalid_community_form(error, cx),
                }
            }
            CommunityOperation::Link => match draft_link(&community_jid, &groups, hidden) {
                Ok(request) => client.link_community_subgroups(request),
                Err(error) => return self.invalid_community_form(error, cx),
            },
            CommunityOperation::Join => match draft_join(&community_jid, &subgroup_jid) {
                Ok(request) => client.join_community_subgroup(request),
                Err(error) => return self.invalid_community_form(error, cx),
            },
        };

        self.community_dialog = None;
        cx.notify();
        let window = window.window_handle();
        cx.spawn(async move |entity, cx| {
            let result = answer.await.unwrap_or_else(|_| {
                Err(crate::session::Failure::worth_retrying(crate::tr!(
                    CommunityDisconnected
                )))
            });
            let _ = window.update(cx, |_, window, cx| {
                let _ = entity.update(cx, |app, cx| match result {
                    Ok(result) => app.report_community_result(operation, &result, window, cx),
                    Err(failure) => app.notify_user(
                        what_went_wrong(crate::tr!(CommunityCouldNotStart), &failure),
                        Tone::Problem,
                        cx,
                    ),
                });
            });
        })
        .detach();
    }

    fn invalid_community_form(&mut self, error: CommunityFormError, cx: &mut Context<Self>) {
        self.notify_user(error.message(), Tone::Problem, cx);
    }

    /// Say what happened, including the parts that did not. A partial result
    /// is not an error notice: the groups that exist matter more than the ones
    /// that do not, so success and each problem get their own sentence.
    fn report_community_result(
        &mut self,
        operation: CommunityOperation,
        result: &CommunityOperationResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let outcome = CommunityOutcome::from_result(result);
        match operation {
            CommunityOperation::Create | CommunityOperation::CreateSubgroup => {
                if outcome.created > 0 {
                    self.notify_user(
                        crate::tr!(community_groups_created, outcome.created),
                        Tone::Info,
                        cx,
                    );
                }
            }
            CommunityOperation::Link => {
                if outcome.linked > 0 {
                    self.notify_user(
                        crate::tr!(community_groups_linked, outcome.linked),
                        Tone::Info,
                        cx,
                    );
                }
                for (jid, code) in &outcome.failed {
                    self.notify_user(
                        crate::tr!(community_link_failed, jid, code),
                        Tone::Problem,
                        cx,
                    );
                }
            }
            CommunityOperation::Join => {
                if outcome.created > 0 {
                    self.notify_user(crate::tr!(CommunityJoined), Tone::Info, cx);
                }
            }
        }
        if outcome.description_warnings > 0 {
            self.notify_user(crate::tr!(CommunityDescriptionFailed), Tone::Problem, cx);
        }
        if outcome.approval_pending > 0 {
            self.notify_user(crate::tr!(CommunityApprovalPending), Tone::Info, cx);
        }

        // A created or joined group that is already in the chat list can be
        // opened now. One that has not arrived over the event stream yet stays
        // in the notice above; guessing a selection for a JID we cannot name
        // would leave an empty conversation pane on screen.
        if matches!(
            operation,
            CommunityOperation::Create
                | CommunityOperation::CreateSubgroup
                | CommunityOperation::Join
        ) && let Some(jid) = result.groups.first().map(|group| group.jid.clone())
            && self.chat_named(&jid).is_some()
        {
            self.select_chat(jid, ChatOpen::ToPreview, window, cx);
        }
    }
}

#[derive(Clone, Copy)]
enum CommunityFlag {
    Closed,
    AllowNonAdmin,
    GeneralChat,
    Hidden,
}

/// Render the modal, borrowing the same focus handle as the other mutually
/// exclusive surfaces.
pub(super) fn render_community_dialog(
    dialog: &CommunityDialog,
    app: Entity<WhatsAppApp>,
    focus: &FocusHandle,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let cancel = app.clone();
    let create = app.clone();
    let mode = dialog.operation;
    let value = |on: bool| {
        if on {
            crate::tr!(EventOn)
        } else {
            crate::tr!(EventOff)
        }
    };

    div()
        .id("community-create-modal")
        .debug_selector(|| "community-create-modal".into())
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
                .id("community-create-form")
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
                        .child(crate::tr!(NewCommunity)),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(metrics.space_xxs())
                        .child(mode_button(
                            "community-mode-create",
                            crate::tr!(CommunityOperationCreate),
                            mode == CommunityOperation::Create,
                            app.clone(),
                            CommunityOperation::Create,
                        ))
                        .child(mode_button(
                            "community-mode-subgroup",
                            crate::tr!(CommunityOperationSubgroup),
                            mode == CommunityOperation::CreateSubgroup,
                            app.clone(),
                            CommunityOperation::CreateSubgroup,
                        ))
                        .child(mode_button(
                            "community-mode-link",
                            crate::tr!(CommunityOperationLink),
                            mode == CommunityOperation::Link,
                            app.clone(),
                            CommunityOperation::Link,
                        ))
                        .child(mode_button(
                            "community-mode-join",
                            crate::tr!(CommunityOperationJoin),
                            mode == CommunityOperation::Join,
                            app.clone(),
                            CommunityOperation::Join,
                        )),
                )
                .when(
                    matches!(
                        mode,
                        CommunityOperation::Create | CommunityOperation::CreateSubgroup
                    ),
                    |el| {
                        el.child(field(
                            crate::tr!(CommunityName).into(),
                            &dialog.name,
                            metrics,
                            cx,
                        ))
                    },
                )
                .when(mode == CommunityOperation::Create, |el| {
                    el.child(field(
                        crate::tr!(CommunityDescription).into(),
                        &dialog.description,
                        metrics,
                        cx,
                    ))
                })
                .when(mode == CommunityOperation::CreateSubgroup, |el| {
                    el.child(field(
                        crate::tr!(CommunityParentJid).into(),
                        &dialog.parent_jid,
                        metrics,
                        cx,
                    ))
                    .child(field(
                        crate::tr!(CommunityParticipants).into(),
                        &dialog.participants,
                        metrics,
                        cx,
                    ))
                })
                .when(
                    matches!(mode, CommunityOperation::Link | CommunityOperation::Join),
                    |el| {
                        el.child(field(
                            crate::tr!(CommunityJid).into(),
                            &dialog.community_jid,
                            metrics,
                            cx,
                        ))
                    },
                )
                .when(mode == CommunityOperation::Link, |el| {
                    el.child(field(
                        crate::tr!(CommunityGroups).into(),
                        &dialog.groups,
                        metrics,
                        cx,
                    ))
                })
                .when(mode == CommunityOperation::Join, |el| {
                    el.child(field(
                        crate::tr!(CommunitySubgroupJid).into(),
                        &dialog.subgroup_jid,
                        metrics,
                        cx,
                    ))
                })
                .when(mode == CommunityOperation::Create, |el| {
                    el.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(metrics.space_sm())
                            .child(flag_button(
                                "community-flag-closed",
                                crate::tr!(CommunityClosed),
                                dialog.closed,
                                value(dialog.closed),
                                app.clone(),
                                CommunityFlag::Closed,
                            ))
                            .child(flag_button(
                                "community-flag-non-admin",
                                crate::tr!(CommunityAllowNonAdmin),
                                dialog.allow_non_admin,
                                value(dialog.allow_non_admin),
                                app.clone(),
                                CommunityFlag::AllowNonAdmin,
                            ))
                            .child(flag_button(
                                "community-flag-general",
                                crate::tr!(CommunityGeneralChat),
                                dialog.create_general_chat,
                                value(dialog.create_general_chat),
                                app.clone(),
                                CommunityFlag::GeneralChat,
                            )),
                    )
                })
                .when(
                    matches!(
                        mode,
                        CommunityOperation::CreateSubgroup | CommunityOperation::Link
                    ),
                    |el| {
                        el.child(flag_button(
                            "community-flag-hidden",
                            crate::tr!(CommunityHidden),
                            dialog.hidden,
                            value(dialog.hidden),
                            app.clone(),
                            CommunityFlag::Hidden,
                        ))
                    },
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(metrics.space_md())
                        .child(
                            Button::new("community-cancel")
                                .label(crate::tr!(Cancel))
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |app, cx| {
                                        app.cancel_community_dialog(cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("community-submit")
                                .primary()
                                .label(mode.label())
                                .on_click(move |_, window, cx| {
                                    create.update(cx, |app, cx| {
                                        app.submit_community_dialog(window, cx);
                                    });
                                }),
                        ),
                ),
        )
        .focus_trap("community-create-trap", focus)
}

impl CommunityOperation {
    fn label(self) -> &'static str {
        match self {
            Self::Create => crate::tr!(CommunityOperationCreate),
            Self::CreateSubgroup => crate::tr!(CommunityOperationSubgroup),
            Self::Link => crate::tr!(CommunityOperationLink),
            Self::Join => crate::tr!(CommunityOperationJoin),
        }
    }
}

fn mode_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    app: Entity<WhatsAppApp>,
    operation: CommunityOperation,
) -> impl IntoElement + use<> {
    Button::new(id)
        .small()
        .label(label)
        .when(active, |button| button.primary())
        .when(!active, |button| button.ghost())
        .on_click(move |_, _, cx| {
            app.update(cx, |app, cx| app.set_community_operation(operation, cx));
        })
}

fn flag_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    state: &'static str,
    app: Entity<WhatsAppApp>,
    flag: CommunityFlag,
) -> impl IntoElement + use<> {
    Button::new(id)
        .ghost()
        .small()
        .label(format!("{label}: {state}"))
        .when(active, |button| button.primary())
        .on_click(move |_, _, cx| {
            app.update(cx, |app, cx| app.toggle_community_flag(flag, cx));
        })
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

    #[test]
    fn a_community_draft_trims_its_name_and_keeps_its_flags() {
        let draft = draft_community("  Synthetic  ", "  About it  ", true, false, true).unwrap();
        assert_eq!(draft.name, "Synthetic");
        assert_eq!(draft.description.as_deref(), Some("About it"));
        assert!(draft.closed);
        assert!(!draft.allow_non_admin_sub_group_creation);
        assert!(draft.create_general_chat);
    }

    #[test]
    fn an_empty_name_is_refused_before_a_request() {
        assert_eq!(
            draft_community("   ", "", false, false, false).unwrap_err(),
            CommunityFormError::EmptyName
        );
    }

    #[test]
    fn blank_description_stays_absent() {
        let draft = draft_community("Synthetic", "  ", false, false, false).unwrap();
        assert_eq!(draft.description, None);
    }

    #[test]
    fn a_subgroup_draft_splits_participants_and_keeps_visibility() {
        let draft = draft_subgroup(
            " 155555500001@g.us ",
            " Subgroup ",
            "155555500002@lid,\n 155555500003@s.whatsapp.net \n\n",
            true,
        )
        .unwrap();
        assert_eq!(draft.parent_jid, "155555500001@g.us");
        assert_eq!(draft.name, "Subgroup");
        assert_eq!(
            draft.participants,
            vec![
                "155555500002@lid".to_string(),
                "155555500003@s.whatsapp.net".to_string(),
            ]
        );
        assert_eq!(draft.visibility, CommunityVisibility::Hidden);
    }

    #[test]
    fn a_subgroup_needs_a_name_and_a_parent() {
        assert_eq!(
            draft_subgroup("155555500001@g.us", "  ", "", false).unwrap_err(),
            CommunityFormError::EmptyName
        );
        assert_eq!(
            draft_subgroup("  ", "Subgroup", "", false).unwrap_err(),
            CommunityFormError::NoParent
        );
    }

    #[test]
    fn a_link_request_keeps_one_target_per_line_and_applies_visibility() {
        let request = draft_link(
            " 155555500001@g.us ",
            "155555500002@g.us\n\n 155555500003@g.us \n",
            false,
        )
        .unwrap();
        assert_eq!(request.community_jid, "155555500001@g.us");
        assert_eq!(request.groups.len(), 2);
        assert_eq!(request.groups[0].jid, "155555500002@g.us");
        assert_eq!(request.groups[1].jid, "155555500003@g.us");
        assert_eq!(request.groups[0].visibility, CommunityVisibility::Visible);
    }

    #[test]
    fn a_link_request_needs_a_community_and_at_least_one_group() {
        assert_eq!(
            draft_link("  ", "155555500002@g.us", false).unwrap_err(),
            CommunityFormError::MissingCommunity
        );
        assert_eq!(
            draft_link("155555500001@g.us", "  \n \n", false).unwrap_err(),
            CommunityFormError::GroupsAbsent
        );
    }

    #[test]
    fn a_join_request_needs_both_addresses() {
        assert_eq!(
            draft_join("  ", "155555500002@g.us").unwrap_err(),
            CommunityFormError::MissingCommunity
        );
        assert_eq!(
            draft_join("155555500001@g.us", "  ").unwrap_err(),
            CommunityFormError::BlankSubgroup
        );
        let request = draft_join("155555500001@g.us", "155555500002@g.us").unwrap();
        assert_eq!(request.community_jid, "155555500001@g.us");
        assert_eq!(request.subgroup_jid, "155555500002@g.us");
    }

    #[test]
    fn address_boxes_ignore_blank_and_padded_entries() {
        assert_eq!(split_lines(" a \n\n  b\t\n"), vec!["a", "b"]);
        assert_eq!(split_addresses(" a, \n b ,, c "), vec!["a", "b", "c"]);
    }

    #[test]
    fn an_outcome_groups_partial_success_and_each_warning() {
        let result = CommunityOperationResult {
            groups: vec![oxidezap_core::CommunityGroupRef {
                jid: "155555500001@g.us".into(),
                subject: None,
                description: None,
            }],
            linked_jids: vec!["155555500002@g.us".into()],
            failed_groups: vec![oxidezap_core::CommunitySubgroupFailure {
                jid: "155555500003@g.us".into(),
                code: 403,
            }],
            warnings: vec![
                oxidezap_core::CommunityWarning {
                    step: CommunityWarningStep::SetDescription,
                    jid: None,
                    detail: None,
                },
                oxidezap_core::CommunityWarning {
                    step: CommunityWarningStep::JoinApprovalPending,
                    jid: None,
                    detail: None,
                },
            ],
        };
        let outcome = CommunityOutcome::from_result(&result);
        assert_eq!(outcome.created, 1);
        assert_eq!(outcome.linked, 1);
        assert_eq!(outcome.failed, vec![("155555500003@g.us".into(), 403)]);
        assert_eq!(outcome.description_warnings, 1);
        assert_eq!(outcome.approval_pending, 1);
    }

    #[test]
    fn an_empty_result_has_nothing_to_report() {
        let outcome = CommunityOutcome::from_result(&CommunityOperationResult::default());
        assert_eq!(outcome, CommunityOutcome::default());
    }
}
