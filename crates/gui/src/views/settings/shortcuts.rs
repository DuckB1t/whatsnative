//! The keyboard map inside Settings.
//!
//! It is a description of the bindings this binary actually installs, not a
//! wish list. Actions that have no key are named too, so an empty row means
//! "use the menu" rather than "this control does not exist". The entries live
//! beside the Settings screen instead of in a modal because Settings already
//! owns the keyboard, scrolls, and closes on Escape.

use gpui::{App, InteractiveElement, IntoElement, ParentElement, Styled, div};
use gpui_component::ActiveTheme as _;

use crate::components::parts;
use crate::theme::Metrics;

/// One line of the map. Keys are written as text rather than drawn as a chip
/// per modifier, because the secondary modifier differs by platform and this
/// panel has to say so once instead of on every row.
struct Row {
    keys: String,
    action: &'static str,
}

struct Group {
    title: &'static str,
    rows: Vec<Row>,
}

fn secondary(rest: &str) -> String {
    format!("Ctrl/⌘ {rest}")
}

fn row(keys: String, action: &'static str) -> Row {
    Row { keys, action }
}

/// Build the table on each render: the labels come from the current language,
/// so a static table would freeze the first language the process saw.
fn groups() -> Vec<Group> {
    vec![
        Group {
            title: crate::tr!(ShortcutScopeEverywhere),
            rows: vec![
                row(secondary("K"), crate::tr!(Search)),
                row(secondary("Shift F"), crate::tr!(ShortcutGlobalSearch)),
                row(secondary("Shift S"), crate::tr!(SavedMessages)),
                row(secondary("Shift P"), crate::tr!(PinnedMessages)),
                row(secondary("F"), crate::tr!(SearchInConversation)),
                row(secondary(","), crate::tr!(Settings)),
                row(secondary("/"), crate::tr!(KeyboardShortcuts)),
                row(
                    "Page Up / Page Down".into(),
                    crate::tr!(ShortcutPageHistory),
                ),
                row(secondary("Q"), crate::tr!(ShortcutQuit)),
            ],
        },
        Group {
            title: crate::tr!(ShortcutScopeChatList),
            rows: vec![
                row("↑ / ↓".into(), crate::tr!(ShortcutPreviousChat)),
                row("Space".into(), crate::tr!(ShortcutToggleCommunity)),
            ],
        },
        Group {
            title: crate::tr!(ShortcutScopeConversation),
            rows: vec![
                row("Enter".into(), crate::tr!(Send)),
                row("Shift / Ctrl Enter".into(), crate::tr!(ShortcutNewLine)),
            ],
        },
        Group {
            title: crate::tr!(ShortcutScopeCalls),
            rows: vec![
                row("Enter".into(), crate::tr!(Accept)),
                row("Escape".into(), crate::tr!(Decline)),
                row(secondary("Shift M"), crate::tr!(Mute)),
                row(secondary("Shift C"), crate::tr!(ShortcutReturnToCall)),
            ],
        },
        Group {
            title: crate::tr!(ShortcutScopeViewer),
            rows: vec![
                row("← / →".into(), crate::tr!(Previous)),
                row("Escape".into(), crate::tr!(Close)),
            ],
        },
        Group {
            title: crate::tr!(ShortcutScopeNone),
            rows: vec![
                row("—".into(), crate::tr!(Reply)),
                row("—".into(), crate::tr!(ForwardMessage)),
                row("—".into(), crate::tr!(StarMessage)),
                row("—".into(), crate::tr!(Edit)),
                row("—".into(), crate::tr!(DeleteForMe)),
                row("—".into(), crate::tr!(MarkChatUnread)),
                row("—".into(), crate::tr!(Archive)),
                row("—".into(), crate::tr!(PinChat)),
                row("—".into(), crate::tr!(MuteConversation)),
                row("—".into(), crate::tr!(Emoji)),
            ],
        },
    ]
}

/// Render a heading, then its rows. `use<>` keeps the helper from borrowing the
/// render tree's lifetime, as the virtual list callers require.
fn render_group(group: Group, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    let subtle = parts::subtle(cx);
    let border = cx.theme().border;
    let mono_font = cx.theme().mono_font_family.clone();
    div()
        .flex()
        .flex_col()
        .gap(metrics.space_sm())
        .child(
            div()
                .text_size(metrics.text_heading())
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(group.title),
        )
        .children(group.rows.into_iter().map(move |row| {
            div()
                .flex()
                .items_center()
                .gap(metrics.space_lg())
                .text_color(subtle)
                .child(
                    div()
                        .min_w(metrics.space_xxxl() * 4.0)
                        .rounded(metrics.radius_sm())
                        .border_1()
                        .border_color(border)
                        .px(metrics.space_sm())
                        .font_family(mono_font.clone())
                        .text_size(metrics.text_meta())
                        .child(row.keys),
                )
                .child(div().text_size(metrics.text_body()).child(row.action))
        }))
}

pub fn render(metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .debug_selector(|| "shortcut-map".into())
        .flex()
        .flex_col()
        .gap(metrics.space_xxl())
        .max_w(metrics.reading_width())
        .child(
            div()
                .text_size(metrics.text_secondary())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(KeyboardShortcutsIntro)),
        )
        .child(
            div()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(ShortcutSecondaryNote)),
        )
        .children(
            groups()
                .into_iter()
                .map(|group| render_group(group, metrics, cx)),
        )
}
