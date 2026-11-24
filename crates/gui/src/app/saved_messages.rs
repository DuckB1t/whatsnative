//! The account-level message panels: saved (starred), pinned and media.
//!
//! Saved and pinned reuse the global search's hit rows, because a hit is the
//! same thing in both places: a chat, a message id and a line of text to
//! click. The media panel is a gallery over the same `MessageSearchHit`
//! projection. Its rows come from the daemon's `MediaResults`, a bounded store
//! listing opened from Storage & media settings. The optional media facts on
//! a row are read from the stored proto; a card shows a picture only when this
//! window already holds its bytes. There is no server total here and no
//! pretend preview fetched behind the reader's back.
//!
//! The pinned panel used to be derived from the `PinChanged` events this
//! window had seen and the messages it had loaded, which left a pin from a
//! different front end or a message outside the loaded page invisible. It now
//! reads the store through `ListPinnedMessages`, exactly like the saved and
//! media panels: the local flag is the source of truth, not this window's
//! event history.

use gpui::{
    App, Context, Entity, FocusHandle, ObjectFit, Styled, StyledImage as _, div, img, prelude::*,
};
use gpui_component::ActiveTheme as _;
use gpui_component::Selectable as _;
use gpui_component::Sizable as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{Icon, IconName, v_flex};
use oxidezap_ipc::MessageSearchHit;

use super::{KeyboardOwner, WhatsAppApp, search, what_went_wrong};
use crate::components::parts;
use crate::retained_image::RetainedImage;
use crate::theme::ActiveProductTheme as _;

/// How many rows each panel keeps, and what the store is asked to send.
pub(super) const LIST_LIMIT: usize = 50;

/// A kind filter for the local media gallery.
///
/// Only kinds the store can actually name are offered. A row whose proto was
/// undecodable has no kind, stays in `All`, and is never claimed to be audio
/// or a document just because its content class carries media.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum MediaFilter {
    #[default]
    All,
    Image,
    Video,
    Audio,
    Document,
    Sticker,
}

impl MediaFilter {
    /// Stable id and debug-selector suffix, never a translated string.
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Image => "images",
            Self::Video => "videos",
            Self::Audio => "audio",
            Self::Document => "documents",
            Self::Sticker => "stickers",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => crate::tr!(MediaFilterAll),
            Self::Image => crate::tr!(MediaFilterImages),
            Self::Video => crate::tr!(MediaFilterVideos),
            Self::Audio => crate::tr!(MediaFilterAudio),
            Self::Document => crate::tr!(MediaFilterDocuments),
            Self::Sticker => crate::tr!(MediaFilterStickers),
        }
    }

    /// Every filter the gallery draws, in the order it draws them.
    const ALL: [Self; 6] = [
        Self::All,
        Self::Image,
        Self::Video,
        Self::Audio,
        Self::Document,
        Self::Sticker,
    ];
}

/// One card in the local media gallery.
#[derive(Debug, Clone)]
pub(super) struct MediaGalleryRow {
    pub chat_jid: String,
    pub chat_name: String,
    pub message_id: String,
    pub snippet: String,
    pub timestamp_ms: i64,
    pub media_kind: Option<String>,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_stored_preview: bool,
    pub fetchable: bool,
}

/// Rows already grouped by chat, keeping the filter index so the thumbnail
/// collected in `Render` stays paired with its card.
struct MediaGroup<'a> {
    chat_jid: String,
    chat_name: String,
    cards: Vec<(usize, &'a MediaGalleryRow)>,
}

impl MediaGalleryRow {
    fn matches(&self, filter: MediaFilter) -> bool {
        match filter {
            MediaFilter::All => true,
            MediaFilter::Image => self.media_kind.as_deref() == Some("image"),
            MediaFilter::Video => self.media_kind.as_deref() == Some("video"),
            MediaFilter::Audio => self.media_kind.as_deref() == Some("audio"),
            MediaFilter::Document => self.media_kind.as_deref() == Some("document"),
            MediaFilter::Sticker => self.media_kind.as_deref() == Some("sticker"),
        }
    }

    fn kind_label(&self) -> &'static str {
        match self.media_kind.as_deref() {
            Some("image") => crate::tr!(MediaFilterImages),
            Some("video") => crate::tr!(MediaFilterVideos),
            Some("audio") => crate::tr!(MediaFilterAudio),
            Some("document") => crate::tr!(MediaFilterDocuments),
            Some("sticker") => crate::tr!(MediaFilterStickers),
            _ => crate::tr!(GlobalSearchMedia),
        }
    }
}

/// Which of the three panels a frame is drawing.
#[derive(Clone, Copy)]
pub(super) enum MessagePanel {
    Saved,
    Pinned,
    Media,
}

impl MessagePanel {
    fn title(self) -> &'static str {
        match self {
            Self::Saved => crate::tr!(SavedMessages),
            Self::Pinned => crate::tr!(PinnedMessages),
            Self::Media => crate::tr!(MediaMessages),
        }
    }

    fn empty(self) -> &'static str {
        match self {
            Self::Saved => crate::tr!(NoSavedMessages),
            Self::Pinned => crate::tr!(NoPinnedMessages),
            Self::Media => crate::tr!(NoMediaMessages),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Saved => "saved-messages-panel",
            Self::Pinned => "pinned-messages-panel",
            Self::Media => "media-messages-panel",
        }
    }

    /// A separate name for the rows, so a panel and its own list do not answer
    /// to the same debug selector.
    fn hit_id(self) -> &'static str {
        match self {
            Self::Saved => "saved-messages-hit",
            Self::Pinned => "pinned-messages-hit",
            Self::Media => "media-messages-hit",
        }
    }

    fn close(self, app: &Entity<WhatsAppApp>, cx: &mut App) {
        app.update(cx, |app, cx| match self {
            Self::Saved => {
                app.close_saved_messages(cx);
            }
            Self::Pinned => {
                app.close_pinned_messages(cx);
            }
            Self::Media => {
                app.close_media_messages(cx);
            }
        });
    }
}

impl WhatsAppApp {
    /// Open the saved panel and ask the store for the current rows.
    pub(super) fn open_saved_messages(&mut self, cx: &mut Context<Self>) {
        self.close_global_search(cx);
        self.close_pinned_messages(cx);
        self.close_media_messages(cx);
        self.saved_messages_panel = true;
        self.keyboard_owner = Some(KeyboardOwner::SavedMessages);
        cx.notify();
        self.request_starred(cx);
    }

    pub(super) fn close_saved_messages(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.saved_messages_panel {
            return false;
        }
        self.saved_messages_panel = false;
        self.keyboard_owner = None;
        cx.notify();
        true
    }

    /// Open the pinned panel and ask the store for the current rows.
    pub(super) fn open_pinned_messages(&mut self, cx: &mut Context<Self>) {
        self.close_global_search(cx);
        self.close_saved_messages(cx);
        self.close_media_messages(cx);
        self.pinned_messages_panel = true;
        self.keyboard_owner = Some(KeyboardOwner::PinnedMessages);
        cx.notify();
        self.request_pinned(cx);
    }

    pub(super) fn close_pinned_messages(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.pinned_messages_panel {
            return false;
        }
        self.pinned_messages_panel = false;
        self.keyboard_owner = None;
        cx.notify();
        true
    }

    /// Open the media jump list from Storage & media settings and ask the
    /// store for the current rows.
    pub(crate) fn open_media_messages(&mut self, cx: &mut Context<Self>) {
        self.close_global_search(cx);
        self.close_saved_messages(cx);
        self.close_pinned_messages(cx);
        self.media_messages_panel = true;
        self.keyboard_owner = Some(KeyboardOwner::MediaMessages);
        cx.notify();
        self.request_media(cx);
    }

    pub(super) fn close_media_messages(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.media_messages_panel {
            return false;
        }
        self.media_messages_panel = false;
        self.keyboard_owner = None;
        cx.notify();
        true
    }

    /// The listing is a local read: the ack only says it ran. The rows arrive
    /// separately as `FromDaemon::StarredResults`, like a search answer.
    fn request_starred(&mut self, cx: &mut Context<Self>) {
        let Some(waiting) = self
            .client()
            .as_ref()
            .map(|client| client.list_starred_messages(LIST_LIMIT))
        else {
            self.notify_user(
                what_went_wrong(
                    crate::tr!(CouldNotLoadSavedMessages),
                    &super::Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                ),
                super::notices::Tone::Problem,
                cx,
            );
            return;
        };
        cx.spawn(async move |entity, cx| {
            let failure = match waiting.await {
                Ok(Ok(())) => None,
                Ok(Err(failure)) => Some(failure),
                Err(_) => Some(super::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            let _ = entity.update(cx, |app, cx| {
                if let Some(failure) = failure {
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotLoadSavedMessages), &failure),
                        super::notices::Tone::Problem,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// The pinned listing is a local read: the ack only says it ran. The rows
    /// arrive separately as `FromDaemon::PinnedResults`, like a search answer,
    /// so a pin made by another front end turns up here too.
    fn request_pinned(&mut self, cx: &mut Context<Self>) {
        let Some(waiting) = self
            .client()
            .as_ref()
            .map(|client| client.list_pinned_messages(LIST_LIMIT))
        else {
            self.notify_user(
                what_went_wrong(
                    crate::tr!(CouldNotLoadPinnedMessages),
                    &super::Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                ),
                super::notices::Tone::Problem,
                cx,
            );
            return;
        };
        cx.spawn(async move |entity, cx| {
            let failure = match waiting.await {
                Ok(Ok(())) => None,
                Ok(Err(failure)) => Some(failure),
                Err(_) => Some(super::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            let _ = entity.update(cx, |app, cx| {
                if let Some(failure) = failure {
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotLoadPinnedMessages), &failure),
                        super::notices::Tone::Problem,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// The media listing is a local read too: the rows arrive separately as
    /// `FromDaemon::MediaResults`, like every other jump list.
    fn request_media(&mut self, cx: &mut Context<Self>) {
        let Some(waiting) = self
            .client()
            .as_ref()
            .map(|client| client.list_media_messages(LIST_LIMIT))
        else {
            self.notify_user(
                what_went_wrong(
                    crate::tr!(CouldNotLoadMediaMessages),
                    &super::Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                ),
                super::notices::Tone::Problem,
                cx,
            );
            return;
        };
        cx.spawn(async move |entity, cx| {
            let failure = match waiting.await {
                Ok(Ok(())) => None,
                Ok(Err(failure)) => Some(failure),
                Err(_) => Some(super::Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            let _ = entity.update(cx, |app, cx| {
                if let Some(failure) = failure {
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotLoadMediaMessages), &failure),
                        super::notices::Tone::Problem,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    fn rows_for_hits<'a>(
        &self,
        hits: impl IntoIterator<Item = &'a MessageSearchHit>,
    ) -> Vec<search::SearchResultRow> {
        hits.into_iter()
            .map(|hit| search::SearchResultRow {
                chat_jid: hit.chat_jid.clone().into(),
                chat_name: self
                    .chat_named(&hit.chat_jid)
                    .map(|chat| chat.name.clone())
                    .unwrap_or_else(|| hit.chat_jid.clone())
                    .into(),
                message_id: hit.message_id.clone().into(),
                snippet: hit.snippet.clone().into(),
                timestamp_ms: hit.timestamp_ms,
                has_media: hit.has_media,
            })
            .collect()
    }

    /// Rows for the saved panel, from the latest store-wide listing.
    pub(super) fn saved_rows(&self) -> Vec<search::SearchResultRow> {
        self.rows_for_hits(self.saved_messages.iter())
    }

    /// Change the gallery's kind filter. Only the local view changes; no new
    /// request goes out and no claim is made about rows outside this listing.
    pub(super) fn set_media_filter(&mut self, filter: MediaFilter, cx: &mut Context<Self>) {
        if self.media_filter != filter {
            self.media_filter = filter;
            cx.notify();
        }
    }

    /// Counts for the filter chips, from the rows this window actually has.
    pub(super) fn media_filter_counts(&self) -> [usize; 6] {
        let rows = self.all_media_rows();
        let mut counts = [0usize; 6];
        for (index, filter) in MediaFilter::ALL.iter().enumerate() {
            counts[index] = rows.iter().filter(|row| row.matches(*filter)).count();
        }
        counts
    }

    fn all_media_rows(&self) -> Vec<MediaGalleryRow> {
        self.media_messages
            .iter()
            .map(|hit| MediaGalleryRow {
                chat_jid: hit.chat_jid.clone(),
                chat_name: self
                    .chat_named(&hit.chat_jid)
                    .map(|chat| chat.name.clone())
                    .unwrap_or_else(|| hit.chat_jid.clone()),
                message_id: hit.message_id.clone(),
                snippet: hit.snippet.clone(),
                timestamp_ms: hit.timestamp_ms,
                media_kind: hit.media_kind.clone(),
                file_name: hit.file_name.clone(),
                mime_type: hit.mime_type.clone(),
                width: hit.width,
                height: hit.height,
                has_stored_preview: hit.has_stored_preview,
                fetchable: hit.fetchable,
            })
            .collect()
    }

    /// Cards for the gallery after the reader's kind filter. Newest first is
    /// already guaranteed by the listing, so no sort is repeated here.
    pub(super) fn media_rows(&self) -> Vec<MediaGalleryRow> {
        self.all_media_rows()
            .into_iter()
            .filter(|row| row.matches(self.media_filter))
            .collect()
    }

    /// Rows for the pinned panel, newest first, straight from the store's
    /// listing. A chat not yet loaded appears by its address rather than being
    /// dropped, which is the whole point of asking the store.
    pub(super) fn pinned_rows(&self) -> Vec<search::SearchResultRow> {
        self.rows_for_hits(self.pinned_results.iter())
    }
}

/// The panel shell, shared by the saved and pinned lists.
pub(super) fn render_panel(
    panel: MessagePanel,
    rows: &[search::SearchResultRow],
    focus: &FocusHandle,
    app: &Entity<WhatsAppApp>,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let close_app = app.clone();
    let result_rows = search::render_hit_rows(rows, app, panel.hit_id(), cx);
    div()
        .id(panel.id())
        .debug_selector(|| panel.id().into())
        .track_focus(focus)
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .pt(metrics.space_xxl())
        .bg(parts::scrim(cx).opacity(0.92))
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .occlude()
        .child(
            v_flex()
                .w(metrics.reading_width())
                .max_w_full()
                .rounded(metrics.radius_xl())
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .p(metrics.space_lg())
                .gap(metrics.space_md())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(metrics.space_sm())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(metrics.space_sm())
                                .child(
                                    Icon::new(gpui_kit_assets::IconName::Bookmark)
                                        .with_size(metrics.icon())
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .child(
                                    div()
                                        .text_size(metrics.text_heading())
                                        .text_color(cx.theme().foreground)
                                        .child(panel.title()),
                                ),
                        )
                        .child(
                            Button::new(format!("{}-close", panel.id()))
                                .icon(IconName::Close)
                                .ghost()
                                .tooltip(crate::tr!(Close))
                                .on_click(move |_, _, cx| panel.close(&close_app, cx)),
                        ),
                )
                .child(div().flex_1().min_h_0().overflow_y_scrollbar().children(
                    if rows.is_empty() {
                        vec![
                            div()
                                .text_size(metrics.text_secondary())
                                .text_color(cx.theme().muted_foreground)
                                .child(panel.empty())
                                .into_any_element(),
                        ]
                    } else {
                        result_rows
                    },
                )),
        )
}

/// One card. The picture is only ever a decoded thumbnail this window already
/// owns; otherwise the kind label carries the card. Clicking always opens the
/// stored message, which is the same jump the old text row offered.
fn render_gallery_card(
    row: &MediaGalleryRow,
    thumb: Option<RetainedImage>,
    app: &Entity<WhatsAppApp>,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let click_app = app.clone();
    let chat_jid = row.chat_jid.clone();
    let message_id = row.message_id.clone();
    let kind = row.kind_label();
    let title = row.file_name.clone().unwrap_or_else(|| row.snippet.clone());
    let time = chrono::DateTime::from_timestamp_millis(row.timestamp_ms)
        .map(|when| crate::utils::format_time_local(&when))
        .unwrap_or_default();
    let dimensions = match (row.width, row.height) {
        (Some(width), Some(height)) => format!(" · {width} × {height}"),
        _ => String::new(),
    };
    let availability = if row.fetchable {
        crate::tr!(MediaFetchable)
    } else if row.has_stored_preview {
        crate::tr!(MediaPreviewInStore)
    } else {
        crate::tr!(MediaNoLocalPreview)
    };
    let mime = row.mime_type.as_deref().unwrap_or_default();
    let mime_suffix = if mime.is_empty() {
        String::new()
    } else {
        format!(" · {mime}")
    };
    let preview = thumb
        .map(|image| {
            img(image.source(false))
                .w(metrics.media_thumb())
                .h(metrics.media_thumb())
                .object_fit(ObjectFit::Contain)
                .rounded(metrics.radius_md())
                .debug_selector(|| "media-gallery-thumb".into())
                .into_any_element()
        })
        .unwrap_or_else(|| {
            div()
                .w(metrics.media_thumb())
                .h(metrics.media_thumb())
                .rounded(metrics.radius_md())
                .bg(cx.theme().muted)
                .flex()
                .items_center()
                .justify_center()
                .text_size(metrics.text_micro())
                .text_color(cx.theme().muted_foreground)
                .child(kind)
                .into_any_element()
        });

    div()
        .id(format!("media-gallery-{chat_jid}-{message_id}"))
        .debug_selector(|| "media-messages-hit".into())
        .w(metrics.media_card_width())
        .cursor_pointer()
        .rounded(metrics.radius_md())
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .p(metrics.space_sm())
        .gap(metrics.space_xs())
        .hover(|el| el.bg(cx.theme().list_hover))
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            click_app.update(cx, |app, cx| {
                app.open_search_hit(&chat_jid, &message_id, window, cx);
            });
        })
        .child(preview)
        .child(
            div()
                .text_size(metrics.text_micro())
                .text_color(cx.theme().muted_foreground)
                .child(row.kind_label()),
        )
        .child(
            div()
                .text_size(metrics.text_secondary())
                .text_color(cx.theme().foreground)
                .child(title),
        )
        .child(
            div()
                .text_size(metrics.text_micro())
                .text_color(cx.theme().muted_foreground)
                .child(format!("{time}{dimensions}{mime_suffix} · {availability}")),
        )
}

/// The local media gallery, opened from Storage & media.
///
/// Grouping is computed from the returned rows, not asked of the daemon. A
/// filter only hides rows already in hand; it never implies the rest of the
/// account has been searched for that kind.
pub(super) fn render_media_gallery(
    rows: &[MediaGalleryRow],
    thumbs: &[Option<RetainedImage>],
    counts: &[usize; 6],
    active: MediaFilter,
    focus: &FocusHandle,
    app: &Entity<WhatsAppApp>,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let close_app = app.clone();
    let filter_apps: Vec<_> = MediaFilter::ALL
        .iter()
        .map(|filter| (*filter, app.clone()))
        .collect();
    let chips = filter_apps
        .into_iter()
        .enumerate()
        .map(|(index, (filter, chip_app))| {
            div()
                .debug_selector(|| format!("media-filter-{}", filter.as_str()))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new(format!("media-filter-{}", filter.as_str()))
                        .label(format!("{} · {}", filter.label(), counts[index]))
                        .small()
                        .outline()
                        .selected(filter == active)
                        .on_click(move |_, _, cx| {
                            chip_app.update(cx, |app, cx| app.set_media_filter(filter, cx));
                        }),
                )
        });

    let mut groups: Vec<MediaGroup<'_>> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        match groups
            .iter_mut()
            .find(|group| group.chat_jid == row.chat_jid)
        {
            Some(group) => group.cards.push((index, row)),
            None => groups.push(MediaGroup {
                chat_jid: row.chat_jid.clone(),
                chat_name: row.chat_name.clone(),
                cards: vec![(index, row)],
            }),
        }
    }
    let groups = groups.into_iter().map(|group| {
        div()
            .debug_selector(|| "media-gallery-chat".into())
            .gap(metrics.space_sm())
            .child(
                div()
                    .text_size(metrics.text_strong())
                    .text_color(cx.theme().foreground)
                    .child(format!("{} · {}", group.chat_name, group.chat_jid)),
            )
            .child(div().flex().flex_wrap().gap(metrics.space_md()).children(
                group.cards.into_iter().map(|(index, row)| {
                    render_gallery_card(row, thumbs.get(index).cloned().flatten(), app, cx)
                        .into_any_element()
                }),
            ))
    });

    div()
        .id("media-messages-panel")
        .debug_selector(|| "media-messages-panel".into())
        .track_focus(focus)
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .pt(metrics.space_xxl())
        .bg(parts::scrim(cx).opacity(0.92))
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .occlude()
        .child(
            v_flex()
                .w(metrics.reading_width())
                .max_w_full()
                .rounded(metrics.radius_xl())
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .p(metrics.space_lg())
                .gap(metrics.space_md())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(metrics.space_sm())
                        .child(
                            div()
                                .text_size(metrics.text_heading())
                                .text_color(cx.theme().foreground)
                                .child(crate::tr!(MediaMessages)),
                        )
                        .child(
                            div()
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_mouse_up(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .child(
                                    Button::new("media-gallery-close")
                                        .icon(IconName::Close)
                                        .ghost()
                                        .tooltip(crate::tr!(Close))
                                        .on_click(move |_, _, cx| {
                                            close_app.update(cx, |app, cx| {
                                                app.close_media_messages(cx);
                                            });
                                        }),
                                ),
                        ),
                )
                .child(
                    div()
                        .text_size(metrics.text_micro())
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::tr!(MediaListGuidance)),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(metrics.space_xs())
                        .children(chips),
                )
                .child(div().flex_1().min_h_0().overflow_y_scrollbar().children(
                    if rows.is_empty() {
                        vec![
                            div()
                                .text_size(metrics.text_secondary())
                                .text_color(cx.theme().muted_foreground)
                                .child(if active == MediaFilter::All {
                                    MessagePanel::Media.empty()
                                } else {
                                    crate::tr!(NoMediaMatches)
                                })
                                .into_any_element(),
                        ]
                    } else {
                        groups.map(|group| group.into_any_element()).collect()
                    },
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::body::tests::connected_app_fixture;
    use chrono::TimeZone as _;
    use oxidezap_core::{Chat, ChatMessage, MediaContent};
    use std::sync::Arc;

    const CHAT: &str = "peer@example.invalid";

    fn message(id: &str, content: &str, timestamp_ms: i64) -> ChatMessage {
        let mut message = ChatMessage::new_incoming(id.into(), CHAT.into(), content.into());
        message.timestamp = chrono::Utc.timestamp_millis_opt(timestamp_ms).unwrap();
        message
    }

    fn saved_hit(id: &str, timestamp_ms: i64) -> MessageSearchHit {
        MessageSearchHit {
            chat_jid: CHAT.into(),
            message_id: id.into(),
            snippet: "a synthetic saved message".into(),
            sender_jid: None,
            timestamp_ms,
            has_media: false,
            ..Default::default()
        }
    }

    /// A panel row is a place to jump to, exactly like a search hit. Both
    /// panels get their own debug selector so a test can tell them apart.
    #[gpui::test]
    fn saved_and_pinned_panels_show_rows_and_open_the_hit(cx: &mut gpui::TestAppContext) {
        let (mut cx, app) = connected_app_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.add_message_to_chat(
                    CHAT,
                    message("SAVED-1", "saved line", 1_700_000_000_000),
                    cx
                ));
                assert!(app.add_message_to_chat(
                    CHAT,
                    message("PINNED-1", "pinned line", 1_700_000_001_000),
                    cx
                ));
                app.adopt_starred_results(vec![saved_hit("SAVED-1", 1_700_000_000_000)], cx);
                app.adopt_pinned_results(vec![saved_hit("PINNED-1", 1_700_000_001_000)], cx);
                app.open_saved_messages(cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("saved-messages-panel").is_some());
        let saved_row = cx
            .debug_bounds("saved-messages-hit")
            .expect("a saved row is drawn");
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(app.saved_messages_panel);
            assert!(!app.pinned_messages_panel);
            assert_eq!(app.keyboard_owner, Some(KeyboardOwner::SavedMessages));
            assert_eq!(app.saved_rows().len(), 1);
            assert_eq!(app.saved_rows()[0].message_id, "SAVED-1");
        });

        cx.simulate_click(saved_row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(!app.saved_messages_panel);
            assert_ne!(app.keyboard_owner, Some(KeyboardOwner::SavedMessages));
            assert_eq!(
                app.focused_message
                    .as_ref()
                    .map(|(jid, id)| (jid.as_str(), id.as_str())),
                Some((CHAT, "SAVED-1"))
            );
            assert_eq!(app.selected_chat.as_deref(), Some(CHAT));
        });

        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.open_pinned_messages(cx));
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("pinned-messages-panel").is_some());
        let pinned_row = cx
            .debug_bounds("pinned-messages-hit")
            .expect("a pinned row is drawn");
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(app.pinned_messages_panel);
            assert!(!app.saved_messages_panel);
            assert_eq!(app.keyboard_owner, Some(KeyboardOwner::PinnedMessages));
            assert_eq!(app.pinned_rows().len(), 1);
            assert_eq!(app.pinned_rows()[0].message_id, "PINNED-1");
        });

        cx.simulate_click(pinned_row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(!app.pinned_messages_panel);
            assert_eq!(
                app.focused_message
                    .as_ref()
                    .map(|(jid, id)| (jid.as_str(), id.as_str())),
                Some((CHAT, "PINNED-1"))
            );
        });
    }

    /// A gallery card is another place to click through to a message. The
    /// card is synthetic, and so is the address.
    #[gpui::test]
    fn media_panel_shows_rows_and_opens_the_hit(cx: &mut gpui::TestAppContext) {
        const MEDIA_CHAT: &str = "155555500001@g.us";
        let (mut cx, app) = connected_app_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                app.chats.push(Arc::new(Chat::new(MEDIA_CHAT.into())));
                let mut row = ChatMessage::new_incoming(
                    "SYNTHETIC-MEDIA-1".into(),
                    MEDIA_CHAT.into(),
                    "[image]".into(),
                );
                row.timestamp = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
                assert!(app.add_message_to_chat(MEDIA_CHAT, row, cx));
                app.adopt_media_results(
                    vec![MessageSearchHit {
                        chat_jid: MEDIA_CHAT.into(),
                        message_id: "SYNTHETIC-MEDIA-1".into(),
                        snippet: "[image]".into(),
                        sender_jid: None,
                        timestamp_ms: 1_700_000_000_000,
                        has_media: true,
                        ..Default::default()
                    }],
                    cx,
                );
                app.open_media_messages(cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("media-messages-panel").is_some());
        let row = cx
            .debug_bounds("media-messages-hit")
            .expect("a media row is drawn");
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(app.media_messages_panel);
            assert!(!app.saved_messages_panel);
            assert!(!app.pinned_messages_panel);
            assert_eq!(app.keyboard_owner, Some(KeyboardOwner::MediaMessages));
            assert_eq!(app.media_rows().len(), 1);
            assert_eq!(app.media_rows()[0].message_id, "SYNTHETIC-MEDIA-1");
        });

        cx.simulate_click(row.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.read(|cx| {
            let app = app.read(cx);
            assert!(!app.media_messages_panel);
            assert_ne!(app.keyboard_owner, Some(KeyboardOwner::MediaMessages));
            assert_eq!(
                app.focused_message
                    .as_ref()
                    .map(|(jid, id)| (jid.as_str(), id.as_str())),
                Some((MEDIA_CHAT, "SYNTHETIC-MEDIA-1"))
            );
            assert_eq!(app.selected_chat.as_deref(), Some(MEDIA_CHAT));
        });
    }

    /// The gallery only claims what the listing carries. Filters hide rows in
    /// hand, grouping is computed from those rows, and an unknown kind stays
    /// under All instead of being guessed.
    #[gpui::test]
    fn media_gallery_filters_and_groups_synthetic_rows(cx: &mut gpui::TestAppContext) {
        const A: &str = "peer-a@example.invalid";
        const B: &str = "peer-b@example.invalid";
        let (mut cx, app) = connected_app_fixture(cx);
        cx.update(|_window, cx| {
            app.update(cx, |app, cx| {
                let mut a = Chat::new(A.into());
                a.name = A.into();
                let mut b = Chat::new(B.into());
                b.name = B.into();
                app.chats.push(Arc::new(a));
                app.chats.push(Arc::new(b));

                let mut png = Vec::new();
                image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    1,
                    1,
                    image::Rgba([20, 40, 60, 255]),
                ))
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .expect("a synthetic 1x1 png encodes");
                let mut loaded = message("SYNTHETIC-IMAGE", "a caption", 3);
                loaded.media = Some(MediaContent::image(
                    Arc::new(png),
                    "image/png".into(),
                    false,
                ));
                assert!(app.add_message_to_chat(A, loaded, cx));
                app.adopt_media_results(
                    vec![
                        MessageSearchHit {
                            chat_jid: A.into(),
                            message_id: "SYNTHETIC-IMAGE".into(),
                            snippet: "a caption".into(),
                            timestamp_ms: 3,
                            has_media: true,
                            media_kind: Some("image".into()),
                            file_name: Some("picture.jpg".into()),
                            has_stored_preview: true,
                            ..Default::default()
                        },
                        MessageSearchHit {
                            chat_jid: B.into(),
                            message_id: "SYNTHETIC-VIDEO".into(),
                            snippet: "[video]".into(),
                            timestamp_ms: 2,
                            has_media: true,
                            media_kind: Some("video".into()),
                            width: Some(640),
                            height: Some(360),
                            fetchable: true,
                            ..Default::default()
                        },
                        MessageSearchHit {
                            chat_jid: B.into(),
                            message_id: "SYNTHETIC-UNKNOWN".into(),
                            snippet: "[media]".into(),
                            timestamp_ms: 1,
                            has_media: true,
                            ..Default::default()
                        },
                    ],
                    cx,
                );
                app.open_media_messages(cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(cx.debug_bounds("media-messages-panel").is_some());
        assert!(cx.debug_bounds("media-gallery-chat").is_some());
        assert!(cx.debug_bounds("media-filter-images").is_some());
        assert!(
            cx.debug_bounds("media-gallery-thumb").is_some(),
            "the message in this window has real png bytes, so the card can draw them"
        );
        cx.read(|cx| {
            let app = app.read(cx);
            assert_eq!(app.media_rows().len(), 3);
            assert_eq!(app.media_filter_counts(), [3, 1, 1, 0, 0, 0]);
            assert_eq!(app.media_rows()[0].chat_name, A);
            assert_eq!(app.media_rows()[1].chat_name, B);
        });

        let videos = cx
            .debug_bounds("media-filter-videos")
            .expect("the video chip is drawn");
        cx.simulate_click(videos.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.read(|cx| {
            let app = app.read(cx);
            assert_eq!(app.media_filter, MediaFilter::Video);
            assert_eq!(app.media_rows().len(), 1);
            assert_eq!(app.media_rows()[0].message_id, "SYNTHETIC-VIDEO");
            assert_eq!(app.media_rows()[0].width, Some(640));
        });

        cx.update(|_window, cx| {
            app.update(cx, |app, cx| app.set_media_filter(MediaFilter::All, cx));
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.read(|cx| {
            let app = app.read(cx);
            assert_eq!(app.media_rows().len(), 3);
        });
    }
}
