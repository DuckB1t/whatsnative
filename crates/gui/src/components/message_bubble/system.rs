//! Rows nobody typed: call records, group changes, the encryption notice.
//!
//! Centred rather than sided, because they belong to the conversation rather
//! than to either person in it. No ticks, no avatar, no author colour.

use gpui::{
    App, Entity, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::FluentBuilder as _,
};
use gpui_component::ActiveTheme as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{Icon, Sizable as _};
use oxidezap_core::{CallRecord, SystemNotice};

use crate::app::WhatsAppApp;
use crate::components::ProductIcon;
use crate::theme::Metrics;

/// A call, after the fact.
pub fn render_call_record(
    record: CallRecord,
    peer_jid: String,
    message_id: String,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let is_missed = record.is_missed();
    // A missed call is the one kind you might want to act on, so it is the
    // only one that takes a colour and offers the call back.
    let accent = if is_missed {
        cx.theme().danger
    } else {
        cx.theme().muted_foreground
    };

    div()
        .id(SharedString::from(format!("call-{message_id}")))
        .flex()
        .items_center()
        .gap(metrics.space_lg())
        .px(metrics.space_lg())
        .py(metrics.space_md())
        .rounded(metrics.radius_lg())
        .bg(cx.theme().secondary)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            Icon::new(if record.is_video {
                ProductIcon::Video
            } else {
                ProductIcon::Phone
            })
            .size(metrics.icon_small())
            .flex_shrink_0()
            .text_color(accent),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(metrics.text_secondary())
                        .text_color(cx.theme().foreground)
                        .child(crate::i18n::Language::current().call_title(&record)),
                )
                .child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(metrics.text_micro())
                        .text_color(accent)
                        .child(crate::i18n::Language::current().call_detail(&record)),
                ),
        )
        // Calling back is a command, so it is a Button rather than a click on
        // the whole row: that is what carries keyboard activation, and a row
        // that dials when you click anywhere on it is easy to hit by accident.
        .when(is_missed, |el| {
            el.child(
                Button::new(SharedString::from(format!("call-back-{message_id}")))
                    .icon(Icon::new(ProductIcon::Phone))
                    .ghost()
                    .small()
                    .tooltip(crate::tr!(CallBack))
                    .cursor_pointer()
                    .on_click(move |_, _window, cx| {
                        entity.update(cx, |app, cx| app.start_call(peer_jid.clone(), false, cx));
                    }),
            )
        })
}

/// A group change, or anything else with a sentence and no author.
pub fn render_notice(text: String, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .px(metrics.space_lg())
        .py(metrics.space_sm())
        .rounded_full()
        .bg(cx.theme().secondary)
        .border_1()
        .border_color(cx.theme().border)
        .text_size(metrics.text_small())
        .text_color(cx.theme().muted_foreground)
        .text_center()
        // Bounded like the encryption notice below it: a group's subject is
        // whoever renamed it typing, and a pill that lays out to its natural
        // width runs out past its own border.
        .max_w(metrics.call_card_width_wide())
        .child(text)
}

/// Earlier messages shared into this conversation.
///
/// Its own row rather than the group-change pill: the imported messages are
/// already ordinary rows in the timeline, so there is nothing to browse here
/// and nothing to click. The clock is the quiet glyph already used for pending
/// state; no new asset is needed to make this read as history.
pub fn render_history_notice(text: String, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap(metrics.space_sm())
        .px(metrics.space_lg())
        .py(metrics.space_sm())
        .rounded(metrics.radius_lg())
        .bg(cx.theme().secondary)
        .border_1()
        .border_color(cx.theme().border)
        .max_w(metrics.call_card_width_wide())
        .child(
            Icon::new(ProductIcon::Clock)
                .size(metrics.icon_small())
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground),
        )
        .child(
            div()
                .min_w_0()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
}

/// Which row a system notice takes, decided without a window so the routing
/// can be tested. The sentence is resolved here as well, so the timeline and
/// the chat list cannot disagree about what a notice says.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NoticeRow {
    Call(CallRecord),
    /// The centred pill: a group change or another sentence without a row.
    Notice(String),
    /// Shared history, which gets its own non-interactive row.
    History(String),
}

impl NoticeRow {
    fn of(notice: &SystemNotice, language: crate::i18n::Language) -> Self {
        match notice {
            SystemNotice::Call(record) => Self::Call(*record),
            SystemNotice::GroupMembership(event) => Self::Notice(language.membership(event)),
            SystemNotice::GroupChanged(text) => Self::Notice(text.clone()),
            SystemNotice::History {
                count,
                receivers,
                text,
            } => Self::History(language.history_notice(*count, *receivers, text.as_deref())),
        }
    }
}

/// The standing notice at the head of every conversation.
///
/// Not a stored message: it is a property of the conversation rather than
/// something that happened in it, so it is drawn as the first row rather than
/// fabricated into the history.
pub fn render_encryption_notice(metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_start()
        .gap(metrics.space_md())
        .px(metrics.space_lg())
        .py(metrics.space_md())
        .rounded(metrics.radius_lg())
        .bg(cx.theme().secondary)
        .border_1()
        .border_color(cx.theme().border)
        .max_w(metrics.call_card_width_wide())
        .child(
            Icon::new(ProductIcon::Lock)
                .size(metrics.icon_small())
                .flex_shrink_0()
                .text_color(cx.theme().success),
        )
        .child(
            // Shrinkable, so the sentence wraps inside the pill instead of
            // laying out to its natural width and running out past the border.
            div()
                .flex_1()
                .min_w_0()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(
                    MessagesAreEndToEndEncryptedNobodyOutsideThisChatCanReadThem
                )),
        )
}

/// Route a notice to the row that draws it.
pub fn render_system_row(
    notice: &SystemNotice,
    peer_jid: String,
    message_id: String,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> gpui::AnyElement {
    match NoticeRow::of(notice, crate::i18n::Language::current()) {
        NoticeRow::Call(record) => {
            render_call_record(record, peer_jid, message_id, entity, metrics, cx).into_any_element()
        }
        NoticeRow::Notice(text) => render_notice(text, metrics, cx).into_any_element(),
        NoticeRow::History(text) => render_history_notice(text, metrics, cx).into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::{NoticeRow, SystemNotice};
    use crate::i18n::Language;

    #[test]
    fn shared_history_takes_its_own_row_not_the_group_change_pill() {
        let group = "155555500001@g.us";
        let notice = SystemNotice::History {
            count: Some(4),
            receivers: Some(2),
            text: None,
        };
        assert_eq!(
            NoticeRow::of(&notice, Language::English),
            NoticeRow::History("Shared 4 earlier messages with 2 group members".into())
        );
        assert_eq!(
            NoticeRow::of(&notice, Language::Portuguese),
            NoticeRow::History("Compartilhou 4 mensagens anteriores com 2 membros do grupo".into())
        );

        // The same notice on a message in a synthetic group still takes the
        // history row, while a real group change stays on the generic pill.
        let mut message = oxidezap_core::fixtures::message("synthetic-history", group, "");
        message.system = Some(notice);
        let system = message.system.as_ref().expect("the notice we just set");
        assert!(matches!(
            NoticeRow::of(system, Language::English),
            NoticeRow::History(_)
        ));
        assert!(matches!(
            NoticeRow::of(
                &SystemNotice::GroupChanged("Group membership changed".into()),
                Language::English
            ),
            NoticeRow::Notice(_)
        ));
    }

    #[test]
    fn a_legacy_history_row_keeps_its_sentence_on_the_history_row() {
        let legacy = "Shared 9 earlier messages with 3 group members (legacy)";
        let notice = SystemNotice::History {
            count: None,
            receivers: None,
            text: Some(legacy.into()),
        };
        for language in Language::ALL {
            assert_eq!(
                NoticeRow::of(&notice, language),
                NoticeRow::History(legacy.into())
            );
        }
    }
}
