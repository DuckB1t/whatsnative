//! The detail sheet for one conversation.
//!
//! Everything on this surface is owned by the caller before the render helper
//! runs. That keeps the helper a function of one frame's facts: it reads the
//! theme, but it never borrows the app, a chat or a roster out of it. The
//! roster is the connection's member list, never `Chat::participants`, which
//! only records senders this window has happened to see.

use std::sync::Arc;

use gpui::{App, Entity, IntoElement, ParentElement, Styled, div};
use gpui_component::ActiveTheme as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{IconName, Sizable};
use oxidezap_core::{Chat, ChatMessage, GroupRoster};

use crate::app::WhatsAppApp;
use crate::components::parts;
use crate::session::MediaCache;
use crate::theme::Metrics;

use super::Avatar;

/// What the window knows about a group's membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RosterState {
    /// Not a group, so there is no roster to ask for.
    NotGroup,
    /// The request is out. There is no answer yet, and silence is not emptiness.
    Waiting,
    /// No roster and no request in flight: the connection did not give us one.
    Unavailable,
    /// The connection answered.
    Ready(GroupRoster),
}

/// Owned facts the sheet draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatInfoModel {
    pub(crate) jid: String,
    pub(crate) display_name: String,
    pub(crate) avatar_key: Option<String>,
    pub(crate) is_group: bool,
    pub(crate) roster: RosterState,
    pub(crate) loaded_media_count: usize,
    pub(crate) muted: bool,
    pub(crate) archived: bool,
}

impl ChatInfoModel {
    /// Build the view model from a chat and the facts this window has for it.
    ///
    /// `loaded_media_count` is counted over the loaded history only. It cannot
    /// claim a server-wide media total, so every label built here says "loaded".
    pub(crate) fn new(
        chat: &Chat,
        is_own_number: bool,
        roster: Option<&GroupRoster>,
        roster_requested: bool,
        loaded_media_count: usize,
        muted: bool,
        archived: bool,
    ) -> Self {
        let roster = if !chat.is_group {
            RosterState::NotGroup
        } else if let Some(roster) = roster {
            RosterState::Ready(roster.clone())
        } else if roster_requested {
            RosterState::Waiting
        } else {
            RosterState::Unavailable
        };
        Self {
            jid: chat.jid.clone(),
            display_name: crate::app::chat_row::display_name(&chat.name, is_own_number),
            avatar_key: chat.avatar_cache_key.clone(),
            is_group: chat.is_group,
            roster,
            loaded_media_count,
            muted,
            archived,
        }
    }

    /// The kind of conversation, stated rather than left to the avatar badge.
    pub(crate) fn kind_label(&self) -> String {
        if self.is_group {
            crate::tr!(ChatInfoGroup).to_string()
        } else {
            crate::tr!(ChatInfoDirect).to_string()
        }
    }

    /// The roster line, or the honest reason there isn't one.
    pub(crate) fn roster_label(&self) -> Option<String> {
        match &self.roster {
            RosterState::NotGroup => None,
            RosterState::Waiting => Some(crate::tr!(ChatInfoMembersPending).to_string()),
            RosterState::Unavailable => Some(crate::tr!(ChatInfoMembersUnavailable).to_string()),
            // The same roster logic the header uses. An empty answer says
            // nothing, so it gets the unavailable sentence rather than a
            // count of zero people.
            RosterState::Ready(roster) => Some(
                super::chat_header::members_line(roster)
                    .unwrap_or_else(|| crate::tr!(ChatInfoMembersUnavailable).to_string()),
            ),
        }
    }
}

/// How many loaded rows carry media.
///
/// The timeline cache is the horizon here. A count over everything this
/// window holds is a count of loaded history, never a claim about the store
/// or the server.
pub(crate) fn loaded_media_count(messages: &[Arc<ChatMessage>]) -> usize {
    messages
        .iter()
        .filter(|message| message.media.is_some())
        .count()
}

/// The current notification state, in words rather than a guessed icon.
pub(crate) fn muted_label(muted: bool) -> String {
    if muted {
        crate::tr!(Muted).to_string()
    } else {
        crate::tr!(ChatInfoNotMuted).to_string()
    }
}

/// The current archive state, in words rather than a filter chip.
pub(crate) fn archived_label(archived: bool) -> String {
    if archived {
        crate::tr!(Archived).to_string()
    } else {
        crate::tr!(ChatInfoNotArchived).to_string()
    }
}

/// The sheet's title, which names the fact it describes.
pub(crate) fn info_title(is_group: bool) -> String {
    if is_group {
        crate::tr!(GroupInfo).to_string()
    } else {
        crate::tr!(ContactInfo).to_string()
    }
}

/// Owned inputs to the render helper.
pub(crate) struct ChatInfoProps {
    pub(crate) model: ChatInfoModel,
    pub(crate) media: Option<Arc<dyn MediaCache>>,
    pub(crate) entity: Entity<WhatsAppApp>,
    pub(crate) metrics: Metrics,
    /// A phone gives the sheet the whole chat-area slot, so it needs the way
    /// back. On a desktop the sheet sits beside the timeline and closes in
    /// place.
    pub(crate) mobile: bool,
}

/// A label and its value, one row of the sheet.
fn field_row(label: String, value: String, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_baseline()
        .justify_between()
        .gap(metrics.space_lg())
        .child(
            div()
                .flex_shrink_0()
                .text_size(metrics.text_small())
                .text_color(parts::subtle(cx))
                .child(label),
        )
        .child(
            div()
                .min_w_0()
                .text_right()
                .text_size(metrics.text_small())
                .text_color(cx.theme().foreground)
                .child(value),
        )
}

/// One state row with a real mutation behind its button.
#[allow(clippy::too_many_arguments)]
fn toggle_row(
    id: &'static str,
    label: String,
    value: String,
    action: String,
    jid: String,
    target: bool,
    mute: bool,
    entity: &Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let button_entity = entity.clone();
    let button_jid = jid.clone();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(metrics.space_lg())
        .child(
            parts::detail_stack()
                .gap(metrics.space_xxs())
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(parts::subtle(cx))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(metrics.text_body())
                        .text_color(cx.theme().foreground)
                        .child(value),
                ),
        )
        .child(
            Button::new(id)
                .label(action)
                .ghost()
                .small()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    let entity = button_entity.clone();
                    let jid = button_jid.clone();
                    entity.update(cx, |app, cx| {
                        if mute {
                            app.set_chat_muted(&jid, target, cx);
                        } else {
                            app.set_chat_archived(&jid, target, cx);
                        }
                    });
                }),
        )
}

/// Draw the sheet. All state was copied into `props` by the caller.
pub(crate) fn render_chat_info(props: ChatInfoProps, cx: &App) -> impl IntoElement + use<> {
    let ChatInfoProps {
        model,
        media,
        entity,
        metrics,
        mobile,
    } = props;
    let jid = model.jid.clone();
    let title = info_title(model.is_group);
    let kind = model.kind_label();
    let roster = model.roster_label();
    let media_count = model.loaded_media_count.to_string();
    let close_entity = entity.clone();
    let navigation = if mobile {
        Button::new("chat-info-back")
            .icon(IconName::ArrowLeft)
            .label(crate::tr!(BackToConversation))
            .ghost()
            .small()
            .tooltip(crate::tr!(BackToConversation))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                close_entity.update(cx, |app, cx| app.close_chat_info(cx));
            })
            .into_any_element()
    } else {
        let close_entity = entity.clone();
        Button::new("chat-info-close")
            .icon(IconName::Close)
            .ghost()
            .accessibility_label(crate::tr!(Close))
            .tooltip(crate::tr!(Close))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                close_entity.update(cx, |app, cx| app.close_chat_info(cx));
            })
            .into_any_element()
    };

    div()
        .h_full()
        .w_full()
        .flex()
        .flex_col()
        .bg(cx.theme().sidebar)
        .border_l_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .h(metrics.header_height())
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(metrics.space_lg())
                .px(metrics.space_xl())
                .border_b_1()
                .border_color(cx.theme().border)
                .child(navigation)
                .child(
                    parts::one_line()
                        .flex_1()
                        .min_w_0()
                        .text_size(metrics.text_heading())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(title),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p(metrics.space_xl())
                .flex()
                .flex_col()
                .gap(metrics.space_xxl())
                // The identity is facts only. There is no About, last seen,
                // phone label or profile field to put here.
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(metrics.space_lg())
                        .child(
                            Avatar::new(
                                model.jid.clone(),
                                &model.display_name,
                                metrics.avatar_call(),
                            )
                            .group(model.is_group)
                            .picture(model.avatar_key.clone(), media)
                            .on(cx.theme().sidebar),
                        )
                        .child(
                            parts::detail_stack()
                                .gap(metrics.space_xs())
                                .child(
                                    parts::one_line()
                                        .text_size(metrics.text_strong())
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(cx.theme().foreground)
                                        .child(model.display_name.clone()),
                                )
                                .child(
                                    parts::one_line()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_size(metrics.text_meta())
                                        .text_color(parts::subtle(cx))
                                        .child(model.jid.clone()),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(metrics.space_md())
                        .child(field_row(
                            crate::tr!(ChatInfoKind).to_string(),
                            kind,
                            metrics,
                            cx,
                        ))
                        .child(field_row(
                            crate::tr!(ChatInfoIdentifier).to_string(),
                            model.jid.clone(),
                            metrics,
                            cx,
                        ))
                        .children(roster.map(|line| {
                            field_row(crate::tr!(ChatInfoMembers).to_string(), line, metrics, cx)
                        }))
                        .child(field_row(
                            crate::tr!(ChatInfoLoadedMedia).to_string(),
                            media_count,
                            metrics,
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(metrics.space_md())
                        .child(toggle_row(
                            "chat-info-mute",
                            crate::tr!(ChatInfoNotifications).to_string(),
                            muted_label(model.muted),
                            if model.muted {
                                crate::tr!(UnmuteConversation).to_string()
                            } else {
                                crate::tr!(MuteConversation).to_string()
                            },
                            jid.clone(),
                            !model.muted,
                            true,
                            &entity,
                            metrics,
                            cx,
                        ))
                        .child(toggle_row(
                            "chat-info-archive",
                            crate::tr!(Archive).to_string(),
                            archived_label(model.archived),
                            if model.archived {
                                crate::tr!(Unarchive).to_string()
                            } else {
                                crate::tr!(Archive).to_string()
                            },
                            jid,
                            !model.archived,
                            false,
                            &entity,
                            metrics,
                            cx,
                        )),
                )
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(parts::subtle(cx))
                        .child(crate::tr!(ChatInfoLimits)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Language, TextKey};
    use oxidezap_core::GroupMember;

    fn direct() -> Chat {
        Chat::with_name(
            "synthetic-peer@example.invalid".to_string(),
            "Synthetic Peer".to_string(),
        )
    }

    fn group() -> Chat {
        let mut chat = Chat::with_name(
            "synthetic-group@example.invalid".to_string(),
            "Synthetic Group".to_string(),
        );
        chat.is_group = true;
        chat
    }

    fn roster(named: &[&str], unnamed: usize, includes_me: bool) -> GroupRoster {
        let mut members: Vec<GroupMember> = named
            .iter()
            .enumerate()
            .map(|(index, name)| GroupMember {
                jid: format!("member-{index}@example.invalid"),
                name: Some((*name).to_string()),
                is_self: false,
            })
            .collect();
        members.extend((0..unnamed).map(|index| GroupMember {
            jid: format!("stranger-{index}@example.invalid"),
            name: None,
            is_self: false,
        }));
        if includes_me {
            members.push(GroupMember {
                jid: "synthetic-self@example.invalid".to_string(),
                name: None,
                is_self: true,
            });
        }
        GroupRoster {
            jid: "synthetic-group@example.invalid".to_string(),
            members,
        }
    }

    fn english() {
        Language::activate(Language::English);
    }

    fn message(id: &str, with_media: bool) -> ChatMessage {
        if with_media {
            ChatMessage::new_outgoing_with_media(
                id.to_string(),
                String::new(),
                oxidezap_core::MediaContent::document(
                    "application/octet-stream".to_string(),
                    Some("synthetic.bin".to_string()),
                ),
            )
        } else {
            ChatMessage::new_outgoing(id.to_string(), String::new())
        }
    }

    #[test]
    fn a_direct_chat_has_no_roster_and_keeps_its_plain_identifier() {
        english();
        let chat = direct();
        let model = ChatInfoModel::new(&chat, false, None, false, 0, false, false);
        assert_eq!(model.jid, "synthetic-peer@example.invalid");
        assert_eq!(model.display_name, "Synthetic Peer");
        assert_eq!(model.roster, RosterState::NotGroup);
        assert_eq!(
            model.kind_label(),
            Language::English.text(TextKey::ChatInfoDirect)
        );
        assert!(model.roster_label().is_none());
    }

    #[test]
    fn the_roster_is_authoritative_not_the_set_of_seen_senders() {
        english();
        let mut chat = group();
        chat.participants.insert(
            "synthetic-peer@example.invalid".to_string(),
            "Synthetic Peer".to_string(),
        );
        let members = roster(&["Synthetic One"], 2, true);
        let model = ChatInfoModel::new(&chat, false, Some(&members), false, 0, false, false);
        assert_eq!(
            model.kind_label(),
            Language::English.text(TextKey::ChatInfoGroup)
        );
        assert_eq!(
            model.roster_label().as_deref(),
            Some("You, Synthetic One and 2 others")
        );
    }

    #[test]
    fn unnamed_members_are_counted_not_named() {
        english();
        let members = roster(&[], 3, false);
        let model = ChatInfoModel::new(&group(), false, Some(&members), false, 0, false, false);
        assert_eq!(model.roster_label().as_deref(), Some("3 members"));
    }

    #[test]
    fn a_roster_without_an_answer_never_claims_the_group_is_empty() {
        english();
        let waiting = ChatInfoModel::new(&group(), false, None, true, 0, false, false);
        assert_eq!(waiting.roster, RosterState::Waiting);
        assert_eq!(
            waiting.roster_label().as_deref(),
            Some(Language::English.text(TextKey::ChatInfoMembersPending))
        );
        let unavailable = ChatInfoModel::new(&group(), false, None, false, 0, false, false);
        assert_eq!(unavailable.roster, RosterState::Unavailable);
        assert_eq!(
            unavailable.roster_label().as_deref(),
            Some(Language::English.text(TextKey::ChatInfoMembersUnavailable))
        );
    }

    #[test]
    fn the_media_count_covers_loaded_rows_with_media_only() {
        english();
        let messages: Vec<Arc<ChatMessage>> = [
            message("synthetic-1", true),
            message("synthetic-2", false),
            message("synthetic-3", true),
        ]
        .into_iter()
        .map(Arc::new)
        .collect();
        let count = loaded_media_count(&messages);
        let model = ChatInfoModel::new(&direct(), false, None, false, count, false, false);
        assert_eq!(count, 2);
        assert_eq!(model.loaded_media_count, 2);
        assert_eq!(
            Language::English.text(TextKey::ChatInfoLoadedMedia),
            "Media in loaded history"
        );
    }

    #[test]
    fn muted_and_archived_labels_state_the_current_flags() {
        english();
        let model = ChatInfoModel::new(&direct(), false, None, false, 0, true, true);
        assert_eq!(muted_label(model.muted), "Muted");
        assert_eq!(archived_label(model.archived), "Archived");
        assert_eq!(muted_label(false), "Not muted");
        assert_eq!(archived_label(false), "Not archived");
    }

    #[test]
    fn fallback_names_follow_the_chat_row_rule() {
        english();
        let chat = Chat::new("synthetic-peer@example.invalid".to_string());
        let model = ChatInfoModel::new(&chat, false, None, false, 0, false, false);
        assert_eq!(model.display_name, "Unknown chat");
        let own = ChatInfoModel::new(&chat, true, None, false, 0, false, false);
        assert_eq!(own.display_name, "Unknown chat (You)");
    }
}
