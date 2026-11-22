//! The Calls destination: the daemon's in-memory log as a list of rows.
//!
//! This is not a conversation and not an archive. The daemon sends a bounded
//! snapshot when the pane opens and when a call state arrives, and the rows
//! here are only what that snapshot held. Tapping a row opens its chat when
//! this window has loaded one; otherwise the row stays an honest label.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, Entity, IntoElement, ParentElement, Pixels, SharedString, Size, Styled, div, size,
};
use gpui_component::ActiveTheme as _;
use gpui_component::Selectable as _;
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::v_virtual_list;
use gpui_component::{Icon, IconName};

use crate::app::WhatsAppApp;
use crate::app::call_log::{CallFilter, CallRow};
use crate::components::{EmptyState, ProductIcon, parts};
use crate::responsive::ResponsiveLayout;
use crate::theme::Metrics;

/// Everything the call list draws for one frame.
pub(crate) struct CallsListProps {
    pub rows: Vec<CallRow>,
    pub filter: CallFilter,
    pub loading: bool,
    pub error: Option<String>,
}

/// The pane beside the list. It carries the retention note so the limit is
/// visible even when the list is full.
pub(crate) struct CallsDetailProps {
    pub loading: bool,
    pub error: Option<String>,
}

pub(crate) fn render_calls_list(
    props: CallsListProps,
    entity: Entity<WhatsAppApp>,
    layout: ResponsiveLayout,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = *layout.metrics();
    let base = if layout.is_mobile() {
        div().w_full()
    } else {
        div().w(layout.sidebar_width())
    };

    base.id("calls-list-container")
        .flex()
        .flex_col()
        .h_full()
        .bg(cx.theme().sidebar)
        .when(!layout.is_mobile(), |el| {
            el.border_r_1().border_color(cx.theme().border)
        })
        .child(render_header(&props, entity.clone(), metrics, cx))
        .child(render_rows(props, entity, layout, metrics, cx))
        .child(
            div()
                .flex_shrink_0()
                .px(metrics.space_lg())
                .pb(metrics.space_lg())
                .text_size(metrics.text_meta())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(CallsNote)),
        )
}

fn render_header(
    props: &CallsListProps,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let refresh_entity = entity.clone();
    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(metrics.space_md())
        .h(metrics.sidebar_header_height() + metrics.filter_chip_height() + metrics.space_md())
        .pl(metrics.space_xl())
        .pr(metrics.space_lg())
        .pt(metrics.space_lg())
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(metrics.text_title())
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(crate::tr!(Calls)),
                )
                .child(
                    Button::new("calls-refresh")
                        .ghost()
                        .small()
                        .tooltip(crate::tr!(RefreshCalls))
                        .child(crate::tr!(RefreshCalls))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            refresh_entity.update(cx, |app, cx| app.refresh_calls(cx));
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .gap(metrics.space_sm())
                .children(CallFilter::ALL.into_iter().map(|filter| {
                    let selected = props.filter == filter;
                    let filter_entity = entity.clone();
                    Button::new(SharedString::from(format!("calls-filter-{}", filter.id())))
                        .ghost()
                        .selected(selected)
                        .h(metrics.filter_chip_height())
                        .px(metrics.space_lg())
                        .rounded_full()
                        .border_1()
                        .flex()
                        .items_center()
                        .flex_shrink_0()
                        .text_size(metrics.text_small())
                        .map(|el| {
                            if selected {
                                el.border_color(cx.theme().primary.opacity(0.45))
                                    .text_color(cx.theme().primary)
                                    .font_weight(gpui::FontWeight::MEDIUM)
                            } else {
                                el.border_color(cx.theme().border)
                                    .text_color(cx.theme().muted_foreground)
                            }
                        })
                        .child(filter.label())
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            filter_entity.update(cx, |app, cx| app.set_call_filter(filter, cx));
                        })
                })),
        )
}

fn render_rows(
    props: CallsListProps,
    entity: Entity<WhatsAppApp>,
    layout: ResponsiveLayout,
    metrics: Metrics,
    _cx: &App,
) -> impl IntoElement + use<> {
    if props.rows.is_empty() {
        let retry = entity.clone();
        let mut empty = if props.loading {
            EmptyState::new(crate::tr!(Loading))
        } else if let Some(error) = props.error.as_deref() {
            EmptyState::new(crate::tr!(CouldNotLoadCalls))
                .description(error.to_string())
                .action(crate::tr!(RefreshCalls), move |_, cx| {
                    retry.update(cx, |app, cx| app.refresh_calls(cx));
                })
        } else if props.filter == CallFilter::Missed {
            EmptyState::new(crate::tr!(NoMissedCalls))
        } else {
            EmptyState::new(crate::tr!(NoCallsYet))
        };
        empty = empty.icon(ProductIcon::Phone).compact(true);
        return div().flex_1().min_h_0().child(empty).into_any_element();
    }

    let rows = props.rows;
    let count = rows.len();
    let width = layout.sidebar_width();
    let height = row_height(layout, metrics);
    let sizes: Rc<Vec<Size<Pixels>>> = Rc::new(vec![size(width, height); count]);
    let row_entity = entity.clone();

    v_virtual_list(entity, "calls-list", sizes, move |_, range, _, cx| {
        range
            .map(|ix| {
                render_row(&rows[ix], height, row_entity.clone(), metrics, cx).into_any_element()
            })
            .collect()
    })
    .size_full()
    .into_any_element()
}

fn row_height(layout: ResponsiveLayout, metrics: Metrics) -> Pixels {
    layout.chat_item_height() + metrics.space_lg()
}

fn render_row(
    row: &CallRow,
    height: Pixels,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let outgoing = row.direction == crate::tr!(CallDirectionOutgoing);
    let missed = row.outcome == crate::tr!(CallResultMissed);
    let direction_ground = if outgoing {
        cx.theme().list_active
    } else {
        cx.theme().background
    };
    let outcome_ink = if missed {
        cx.theme().danger
    } else {
        cx.theme().muted_foreground
    };
    let when = row
        .duration
        .as_ref()
        .map(|duration| format!("{}: {}", crate::tr!(CallDuration), duration))
        .unwrap_or_default();
    let tap_entity = entity.clone();
    let tap_jid = row.jid.clone();
    let can_open = row.can_open_chat;

    div()
        .id(format!("call-row-{}", row.id))
        .debug_selector(|| "call-row".into())
        .w_full()
        .h(height)
        .flex()
        .items_center()
        .gap(metrics.space_md())
        .px(metrics.space_lg())
        .when(can_open, |el| {
            el.cursor_pointer().on_click(move |_, window, cx| {
                tap_entity.update(cx, |app, cx| {
                    app.open_call_chat(tap_jid.clone(), window, cx);
                });
            })
        })
        .child(
            div()
                .flex_shrink_0()
                .size(metrics.icon_button())
                .rounded_full()
                .bg(direction_ground)
                .flex()
                .items_center()
                .justify_center()
                .text_color(if outgoing {
                    cx.theme().foreground
                } else {
                    cx.theme().muted_foreground
                })
                .child(
                    Icon::new(if outgoing {
                        IconName::ArrowUp
                    } else {
                        IconName::ArrowDown
                    })
                    .size(metrics.icon_small()),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(metrics.space_xxs())
                .child(
                    parts::one_line()
                        .text_size(metrics.text_body())
                        .text_color(cx.theme().foreground)
                        .child(row.name.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(metrics.space_xs())
                        .child(
                            Icon::new(if row.kind == crate::tr!(VideoCall) {
                                ProductIcon::Video
                            } else {
                                ProductIcon::Phone
                            })
                            .size(metrics.icon_small())
                            .flex_shrink_0()
                            .text_color(outcome_ink),
                        )
                        .child(
                            parts::one_line()
                                .text_size(metrics.text_small())
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} · {}", row.direction, row.kind)),
                        ),
                ),
        )
        .child(
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .items_end()
                .gap(metrics.space_xxs())
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(outcome_ink)
                        .child(row.outcome.clone()),
                )
                .child(
                    div()
                        .text_size(metrics.text_meta())
                        .text_color(cx.theme().muted_foreground)
                        .child(when),
                )
                .child(
                    div()
                        .text_size(metrics.text_meta())
                        .text_color(cx.theme().muted_foreground)
                        .child(row.time.clone()),
                ),
        )
}

pub(crate) fn render_calls_detail(
    props: CallsDetailProps,
    entity: Entity<WhatsAppApp>,
    _cx: &App,
) -> impl IntoElement + use<> {
    let retry = entity;
    let description = props
        .error
        .as_deref()
        .unwrap_or(crate::tr!(CallsNote))
        .to_string();
    let title = if props.loading {
        crate::tr!(Loading)
    } else if props.error.is_some() {
        crate::tr!(CouldNotLoadCalls)
    } else {
        crate::tr!(Calls)
    };
    EmptyState::new(title)
        .description(description)
        .icon(ProductIcon::Phone)
        .action(crate::tr!(RefreshCalls), move |_, cx| {
            retry.update(cx, |app, cx| app.refresh_calls(cx));
        })
}
