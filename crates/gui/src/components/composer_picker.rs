//! The chat composer's emoji, sticker and GIF popover.

use super::*;
use gpui::{SharedString, img};
use gpui_component::scroll::Scrollbar;
use gpui_component::{Selectable as _, popover::Popover};
use oxidezap_core::{MediaContent, MediaType};

use crate::retained_image::RetainedImage;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickerTab {
    #[default]
    Emoji,
    Stickers,
    Favorites,
    Gifs,
}

impl PickerTab {
    pub(crate) fn matches(self, media: &MediaContent) -> bool {
        match self {
            Self::Emoji | Self::Favorites => false,
            Self::Stickers => media.media_type == MediaType::Sticker,
            Self::Gifs => media.media_type == MediaType::Video && media.is_animated,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Emoji => "picker-tab-emoji",
            Self::Stickers => "picker-tab-stickers",
            Self::Favorites => "picker-tab-favorites",
            Self::Gifs => "picker-tab-gifs",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Emoji => crate::tr!(Emoji),
            Self::Stickers => crate::tr!(Stickers),
            Self::Favorites => crate::tr!(StickerFavorites),
            Self::Gifs => crate::tr!(Gifs),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerRequest {
    pub source: Option<String>,
    pub message_id: String,
    pub target: String,
    pub generation: u64,
}

#[derive(Clone)]
pub struct PickerItem {
    pub request: PickerRequest,
    pub preview: Option<RetainedImage>,
    pub decode_as_sticker: bool,
    pub description: String,
    /// The real text this tile can be searched by: chat name, message text,
    /// caption and file name where the window actually holds them. GUI-only;
    /// it never crosses the wire and is never invented for a tile whose text
    /// is not loaded.
    pub search_text: String,
}

/// Whether a tile's loaded text contains every word of the query. The query
/// words arrive lowercased; the text is folded here. A tile whose text is not
/// loaded matches no query, which is the honest answer for favorites rather
/// than an invented match.
fn tile_matches(text: &str, words: &[&str]) -> bool {
    if words.is_empty() {
        return true;
    }
    let text = text.to_lowercase();
    words.iter().all(|word| text.contains(word))
}

impl InputAreaView {
    pub(crate) fn set_picker_items(
        &mut self,
        tab: PickerTab,
        items: Vec<PickerItem>,
        cx: &mut Context<Self>,
    ) {
        if self.picker_tab == tab {
            self.picker_status = None;
            self.picker_items = items;
            self.refilter_picker();
            cx.notify();
        }
    }

    pub(crate) fn set_picker_status(
        &mut self,
        tab: PickerTab,
        status: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.picker_tab == tab {
            self.picker_status = status;
            cx.notify();
        }
    }

    pub(crate) fn has_picker_item(&self, request: &PickerRequest) -> bool {
        self.picker_items
            .iter()
            .any(|item| &item.request == request)
    }

    /// Close the popover and forget what it showed. For every close the
    /// owner makes itself: the popover reports only the ones it starts, and a
    /// tab and a list left from the previous chat would offer that chat's
    /// stickers to this one.
    pub(super) fn close_picker(&mut self) {
        self.emoji_open = false;
        self.picker_items.clear();
        self.picker_status = None;
        self.picker_tab = PickerTab::Emoji;
        self.reset_picker_search();
    }

    /// Forget the media query and schedule the field's text to be cleared on
    /// the next paint. Clearing an `InputState` needs a window, which the
    /// close paths do not all hold, so the text reset is deferred to the
    /// render pass rather than threaded through every caller.
    fn reset_picker_search(&mut self) {
        self.picker_query.clear();
        self.picker_search_pending_clear = true;
        self.refilter_picker();
    }

    /// Recompute which loaded tiles match the query. Runs on change, never on
    /// a paint pass. An empty query shows everything the picker holds; the
    /// filter only ever hides rows already in hand, never asks for more.
    pub(super) fn refilter_picker(&mut self) {
        if self.picker_query.is_empty() {
            self.picker_visible = (0..self.picker_items.len()).collect();
            return;
        }
        let words: Vec<&str> = self.picker_query.split_whitespace().collect();
        self.picker_visible = self
            .picker_items
            .iter()
            .enumerate()
            .filter(|(_, item)| tile_matches(&item.search_text, &words))
            .map(|(index, _)| index)
            .collect();
    }

    /// A tile whose picture cannot be had — the media has expired on
    /// WhatsApp's side — is removed rather than left as a word in a box.
    pub(crate) fn drop_picker_item(&mut self, request: &PickerRequest, cx: &mut Context<Self>) {
        let before = self.picker_items.len();
        self.picker_items.retain(|item| &item.request != request);
        if self.picker_items.len() != before {
            self.refilter_picker();
            cx.notify();
        }
    }

    pub(crate) fn set_picker_preview(
        &mut self,
        request: &PickerRequest,
        preview: RetainedImage,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self
            .picker_items
            .iter_mut()
            .find(|item| &item.request == request)
        {
            item.preview = Some(preview);
            item.decode_as_sticker = true;
            cx.notify();
        }
    }

    pub(super) fn render_picker(
        &self,
        control: gpui::Pixels,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let composer = cx.entity();
        let closing = composer.clone();
        let emoji_picker = self.emoji_picker.clone();
        Popover::new("composer-emojis")
            .anchor(gpui::Anchor::BottomRight)
            .open(self.emoji_open)
            .track_focus(&self.emoji_picker.read(cx).focus_handle(cx))
            .trigger(
                parts::icon_button(
                    "emoji",
                    Icon::new(ProductIcon::Smile),
                    crate::tr!(EmojiStickersAndGifs),
                    control,
                )
                .disabled(self.edit_pending)
                .debug_selector(|| "emoji-trigger".into()),
            )
            .on_open_change(move |open: &bool, _, cx| {
                closing.update(cx, |view, cx| {
                    if *open && !view.edit_pending && !view.is_recording {
                        view.emoji_open = true;
                    } else {
                        view.close_picker();
                    }
                    cx.notify();
                });
            })
            .content(move |_, window, cx| {
                let popover = cx.entity();
                // A close or tab switch scheduled the search text to be
                // cleared; do it here where a window is in hand, before the
                // view is read for the paint.
                if composer.read(cx).picker_search_pending_clear {
                    let search = composer.read(cx).picker_search.clone();
                    search.update(cx, |input, cx| input.set_value("", window, cx));
                    composer.update(cx, |view, _| view.picker_search_pending_clear = false);
                }
                let view = composer.read(cx);
                let tab = view.picker_tab;
                let editing = view.editing;
                let query_active = !view.picker_query.is_empty();
                let items: Vec<PickerItem> = view
                    .picker_visible
                    .iter()
                    .filter_map(|&index| view.picker_items.get(index).cloned())
                    .collect();
                let status = view.picker_status.clone();
                let search = view.picker_search.clone();
                let metrics = cx.product().metrics;
                // As large as the window is: a fixed size is right at one
                // resolution and a postage stamp at the next. Two fifths of the
                // width and three fifths of the height, kept between what is
                // usable and what is absurd.
                let viewport = window.viewport_size();
                let cell = metrics.icon_button();
                let width = (viewport.width * 0.4)
                    .clamp(cell * 12.0, cell * 30.0)
                    .min(viewport.width - metrics.space_xl());
                let height = (viewport.height * 0.62)
                    .max(cell * 9.0)
                    .min(viewport.height - metrics.space_xl());
                // The header, search, tones and tabs take about five cells.
                // Cells sized to the popup rather than to a button, so what is
                // being picked is large enough to pick: about eleven across.
                const EMOJI_ACROSS: usize = 11;
                let emoji_cell = width / EMOJI_ACROSS as f32;
                emoji_picker.update(cx, |picker, cx| {
                    if picker.fit(
                        EMOJI_ACROSS,
                        ((height - cell * 5.0) / emoji_cell).floor(),
                        emoji_cell,
                    ) {
                        cx.notify();
                    }
                });
                let scroll = window
                    .use_keyed_state(
                        SharedString::from(format!("picker-scroll-{}", tab.id())),
                        cx,
                        |_, _| gpui::ScrollHandle::new(),
                    )
                    .read(cx)
                    .clone();
                let columns = ((width / (cell * 4.0)).floor() as usize).clamp(3, 8);
                let title = match tab {
                    PickerTab::Emoji => None,
                    PickerTab::Stickers => Some(crate::tr!(RecentStickers)),
                    PickerTab::Favorites => Some(crate::tr!(StickerFavorites)),
                    PickerTab::Gifs => Some(crate::tr!(RecentGifs)),
                };
                let item_label = if tab == PickerTab::Gifs {
                    crate::tr!(Gif)
                } else {
                    crate::tr!(StickerText)
                };

                div()
                    .w(width)
                    .flex()
                    .flex_col()
                    .gap(metrics.space_sm())
                    .child(
                        div().flex().children(
                            [
                                PickerTab::Emoji,
                                PickerTab::Stickers,
                                PickerTab::Favorites,
                                PickerTab::Gifs,
                            ]
                            .into_iter()
                            .map(|choice| {
                                let composer = composer.clone();
                                Button::new(choice.id())
                                    .debug_selector(move || choice.id().into())
                                    .ghost()
                                    .selected(choice == tab)
                                    .disabled(editing && choice != PickerTab::Emoji)
                                    .label(choice.label())
                                    .on_click(move |_, _, cx| {
                                        composer.update(cx, |view, cx| {
                                            if view.picker_tab == choice {
                                                return;
                                            }
                                            view.picker_tab = choice;
                                            view.picker_items.clear();
                                            view.picker_status = None;
                                            view.reset_picker_search();
                                            if choice != PickerTab::Emoji {
                                                cx.emit(InputAreaEvent::LoadPickerMedia(choice));
                                            }
                                            cx.notify();
                                        });
                                    })
                            }),
                        ),
                    )
                    .children(
                        matches!(tab, PickerTab::Stickers | PickerTab::Gifs).then(|| {
                            div()
                                .id("picker-search")
                                .debug_selector(|| "picker-search".into())
                                .child(Input::new(&search).w_full())
                        }),
                    )
                    .children(title.map(|title| div().child(title)))
                    .child(if tab == PickerTab::Emoji {
                        emoji_picker.clone().into_any_element()
                    } else if items.is_empty() {
                        div()
                            .child(if query_active {
                                crate::tr!(NoPickerMediaMatches).to_owned()
                            } else {
                                status.unwrap_or_else(|| {
                                    match tab {
                                        PickerTab::Favorites => crate::tr!(NoStickerFavorites),
                                        PickerTab::Gifs => crate::tr!(NoRecentGifs),
                                        _ => crate::tr!(NoRecentStickers),
                                    }
                                    .to_owned()
                                })
                            })
                            .into_any_element()
                    } else {
                        let edge =
                            ((width - metrics.space_lg()) / columns as f32) - metrics.space_xs();
                        let content = div()
                            .id("picker-media-grid")
                            .debug_selector(|| "picker-media-grid".into())
                            .h(height)
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .flex()
                            .flex_col()
                            .gap(metrics.space_xs())
                            .children(items.chunks(columns).map(|row| {
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .gap(metrics.space_xs())
                                    .children(row.iter().map(|item| {
                                        let request = item.request.clone();
                                        let composer = composer.clone();
                                        let popover = popover.clone();
                                        let id: SharedString =
                                            format!("picker-{}", request.message_id).into();
                                        Button::new(id.clone())
                                            .debug_selector({
                                                let id = id.clone();
                                                move || id.to_string()
                                            })
                                            .ghost()
                                            .w(edge)
                                            .h(edge)
                                            .tooltip(format!("{item_label} · {}", item.description))
                                            .child(match &item.preview {
                                                Some(preview) if item.decode_as_sticker => {
                                                    crate::components::sticker::sticker(
                                                        id,
                                                        preview.clone(),
                                                    )
                                                    .into_any_element()
                                                }
                                                Some(preview) => img(preview.source(false))
                                                    .w_full()
                                                    .h_full()
                                                    .object_fit(gpui::ObjectFit::Contain)
                                                    .into_any_element(),
                                                None => div().child(item_label).into_any_element(),
                                            })
                                            .on_click(move |_, window, cx| {
                                                popover.update(cx, |state, cx| {
                                                    state.dismiss(window, cx)
                                                });
                                                composer.update(cx, |_, cx| {
                                                    cx.emit(InputAreaEvent::SendPickerMedia(
                                                        request.clone(),
                                                    ));
                                                });
                                            })
                                    }))
                            }));
                        div()
                            .relative()
                            .child(content)
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .bottom_0()
                                    .child(Scrollbar::vertical(&scroll)),
                            )
                            .into_any_element()
                    })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext, px, size};
    use std::{cell::RefCell, rc::Rc};

    #[gpui::test]
    fn composer_picker_sends_only_the_clicked_recent_media(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
        struct Harness(Entity<InputAreaView>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(self.0.clone())
            }
        }

        let mut input = None;
        let window = cx.open_window(size(px(640.), px(400.)), |window, cx| {
            let view = cx.new(|cx| InputAreaView::new(window, cx));
            input = Some(view.clone());
            Harness(view)
        });
        let input = input.unwrap();
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let sent = Rc::new(RefCell::new(Vec::new()));
        let observed = sent.clone();
        let events = cx.update(|_, cx| {
            let input_view = input.clone();
            cx.subscribe(&input, move |_, event: &InputAreaEvent, cx| match event {
                InputAreaEvent::LoadPickerMedia(tab) => {
                    let request = PickerRequest {
                        source: (*tab != PickerTab::Favorites)
                            .then(|| "15550001111@s.whatsapp.net".into()),
                        message_id: tab.id().into(),
                        target: "15550002222@s.whatsapp.net".into(),
                        generation: 1,
                    };
                    input_view.update(cx, |view, cx| {
                        view.set_picker_items(
                            *tab,
                            (0..if *tab == PickerTab::Stickers { 60 } else { 1 })
                                .map(|index| {
                                    let mut request = request.clone();
                                    if index != 0 {
                                        request.message_id = format!("sticker-{index}");
                                    }
                                    PickerItem {
                                        request,
                                        preview: None,
                                        decode_as_sticker: false,
                                        description: "Test chat".into(),
                                        search_text: "Test chat".into(),
                                    }
                                })
                                .collect(),
                            cx,
                        );
                    });
                }
                InputAreaEvent::SendPickerMedia(request) => {
                    observed.borrow_mut().push(request.clone())
                }
                _ => {}
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));

        for (index, tab) in [PickerTab::Stickers, PickerTab::Favorites, PickerTab::Gifs]
            .into_iter()
            .enumerate()
        {
            for selector in ["emoji-trigger", tab.id()] {
                let bounds = cx.debug_bounds(selector).expect(selector);
                cx.simulate_click(bounds.center(), Modifiers::default());
                cx.run_until_parked();
                cx.update(|window, cx| window.draw(cx).clear(cx));
            }
            let selector = match tab {
                PickerTab::Stickers => "picker-picker-tab-stickers",
                PickerTab::Favorites => "picker-picker-tab-favorites",
                PickerTab::Gifs => "picker-picker-tab-gifs",
                PickerTab::Emoji => unreachable!(),
            };
            let bounds = cx.debug_bounds(selector).expect(selector);
            if tab == PickerTab::Stickers {
                let grid = cx.debug_bounds("picker-media-grid").unwrap();
                assert!(grid.size.width > px(300.));
                cx.simulate_event(gpui::ScrollEvent {
                    position: grid.center(),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-90.))),
                    ..Default::default()
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let moved = cx.debug_bounds(selector).unwrap();
                assert!(
                    moved.origin.y < bounds.origin.y,
                    "sticker tiles should scroll"
                );
                cx.simulate_event(gpui::ScrollEvent {
                    position: grid.center(),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(90.))),
                    ..Default::default()
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
            }
            assert_eq!(sent.borrow().len(), index);
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        assert_eq!(
            sent.borrow()
                .iter()
                .map(|request| request.message_id.as_str())
                .collect::<Vec<_>>(),
            [
                PickerTab::Stickers.id(),
                PickerTab::Favorites.id(),
                PickerTab::Gifs.id()
            ]
        );
        drop(events);
    }

    #[test]
    fn tile_matching_is_case_insensitive_and_requires_every_word() {
        // Every word must be present; order does not matter.
        assert!(tile_matches("Birthday Cat Party", &["cat", "birthday"]));
        assert!(tile_matches("Birthday Cat Party", &["party"]));
        // Case is folded on the text; the words arrive lowercased.
        assert!(tile_matches("CAT meme", &["cat"]));
        // A missing word hides the tile.
        assert!(!tile_matches("Birthday party", &["cat"]));
        assert!(!tile_matches("Cat meme", &["cat", "dog"]));
        // A tile with no loaded text matches no query: the honest favorites
        // answer, not an invented match.
        assert!(!tile_matches("", &["cat"]));
        // No words is the empty query: everything shows.
        assert!(tile_matches("", &[]));
        assert!(tile_matches("anything", &[]));
    }

    /// Open the picker on the stickers tab with three tiles of distinct text.
    /// Returns the subscription handle so the caller can drop it.
    fn open_stickers_with_three_tiles(
        cx: &mut VisualTestContext,
        input: &Entity<InputAreaView>,
    ) -> gpui::Subscription {
        let events = cx.update(|_, cx| {
            let input_view = input.clone();
            cx.subscribe(input, move |_, event: &InputAreaEvent, cx| {
                if let InputAreaEvent::LoadPickerMedia(PickerTab::Stickers) = event {
                    let texts = ["Birthday party", "Cat meme", "birthday cat"];
                    let items = texts
                        .iter()
                        .enumerate()
                        .map(|(index, text)| PickerItem {
                            request: PickerRequest {
                                source: Some("15550001111@s.whatsapp.net".into()),
                                message_id: format!("tile-{index}"),
                                target: "15550002222@s.whatsapp.net".into(),
                                generation: 1,
                            },
                            preview: None,
                            decode_as_sticker: false,
                            description: (*text).into(),
                            search_text: (*text).into(),
                        })
                        .collect();
                    input_view.update(cx, |view, cx| {
                        view.set_picker_items(PickerTab::Stickers, items, cx)
                    });
                }
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for selector in ["emoji-trigger", "picker-tab-stickers"] {
            let bounds = cx.debug_bounds(selector).expect(selector);
            cx.simulate_click(bounds.center(), Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        events
    }

    #[gpui::test]
    fn picker_search_filters_loaded_tiles_as_you_type(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
        struct Harness(Entity<InputAreaView>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(self.0.clone())
            }
        }
        let mut input = None;
        let window = cx.open_window(size(px(640.), px(400.)), |window, cx| {
            let view = cx.new(|cx| InputAreaView::new(window, cx));
            input = Some(view.clone());
            Harness(view)
        });
        let input = input.unwrap();
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let events = open_stickers_with_three_tiles(&mut cx, &input);

        assert!(cx.debug_bounds("picker-search").is_some());
        assert!(cx.debug_bounds("picker-media-grid").is_some());
        assert_eq!(cx.update(|_, cx| input.read(cx).picker_visible.len()), 3);

        // Focus the search field and type; the subscription refilters on
        // change, never on a paint pass.
        cx.update(|window, cx| {
            let handle = input.read(cx).picker_search.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        });
        cx.simulate_input("cat");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let (query, visible) = cx.update(|_, cx| {
            let view = input.read(cx);
            (view.picker_query.clone(), view.picker_visible.clone())
        });
        assert_eq!(query, "cat");
        // "Cat meme" and "birthday cat" match; "Birthday party" does not.
        assert_eq!(visible, vec![1, 2]);
        assert!(cx.debug_bounds("picker-media-grid").is_some());

        // A query nothing loaded matches is an honest empty state, not a fetch.
        cx.simulate_input("zzz");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let visible = cx.update(|_, cx| input.read(cx).picker_visible.clone());
        assert!(visible.is_empty());
        assert!(cx.debug_bounds("picker-media-grid").is_none());
        drop(events);
    }

    #[gpui::test]
    fn picker_search_only_offers_on_stickers_and_gifs(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
        struct Harness(Entity<InputAreaView>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(self.0.clone())
            }
        }
        let mut input = None;
        let window = cx.open_window(size(px(640.), px(400.)), |window, cx| {
            let view = cx.new(|cx| InputAreaView::new(window, cx));
            input = Some(view.clone());
            Harness(view)
        });
        let input = input.unwrap();
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx.debug_bounds("emoji-trigger").expect("emoji-trigger");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // Emoji has its own search; the media box is not drawn there.
        assert!(cx.debug_bounds("picker-search").is_none());
        // Favorites carry no text source, so offering a search would lie.
        let bounds = cx.debug_bounds("picker-tab-favorites").expect("favorites");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("picker-search").is_none());
        // GIFs do have loaded text, so the box is there.
        let bounds = cx.debug_bounds("picker-tab-gifs").expect("gifs");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("picker-search").is_some());
        drop(input);
    }

    #[gpui::test]
    fn closing_the_picker_forgets_the_media_query(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::theme::init(cx);
        });
        struct Harness(Entity<InputAreaView>);
        impl Render for Harness {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(self.0.clone())
            }
        }
        let mut input = None;
        let window = cx.open_window(size(px(640.), px(400.)), |window, cx| {
            let view = cx.new(|cx| InputAreaView::new(window, cx));
            input = Some(view.clone());
            Harness(view)
        });
        let input = input.unwrap();
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let events = open_stickers_with_three_tiles(&mut cx, &input);
        cx.update(|window, cx| {
            let handle = input.read(cx).picker_search.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        });
        cx.simulate_input("cat");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.update(|_, cx| input.read(cx).picker_query.clone()),
            "cat"
        );

        // Close by clicking outside the popover; the query is forgotten at
        // once and the field's text is scheduled to clear on the next paint.
        cx.simulate_click(gpui::point(px(10.), px(10.)), Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let (query, pending) = cx.update(|_, cx| {
            let view = input.read(cx);
            (view.picker_query.clone(), view.picker_search_pending_clear)
        });
        assert!(query.is_empty());
        assert!(pending);

        // Reopen: the deferred clear runs on paint and the field is empty.
        let bounds = cx.debug_bounds("emoji-trigger").expect("emoji-trigger");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let text = cx.update(|_, cx| input.read(cx).picker_search.read(cx).text().to_string());
        assert!(text.is_empty());
        assert!(!cx.update(|_, cx| input.read(cx).picker_search_pending_clear));
        drop(events);
    }
}
