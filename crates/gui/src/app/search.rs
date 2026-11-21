//! The window's two searches.
//!
//! The sidebar's field filters *chats* by name; the header's magnifier
//! searches *messages* in the conversation on screen. They were one control
//! once — the header's magnifier said "Search in conversation" and focused
//! the sidebar's field, so it did something other than what it was labelled —
//! and keeping them in one module is what keeps that distinction written down
//! rather than rediscovered.
//!
//! The conversation's half searches what is loaded, and says so when that is
//! all it searched: the store's FTS index lives behind the daemon, and a
//! control that silently covers only part of a history is worse than one that
//! names its horizon.

use std::time::Duration;

use gpui::{
    App, Context, Entity, FocusHandle, SharedString, Styled, Task, WeakEntity, Window, div,
    prelude::*,
};
use gpui_component::ActiveTheme as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName, Sizable as _, v_flex};
use oxidezap_core::ChatMessage;
use oxidezap_ipc::MessageSearchHit;

use super::{ChatOpen, Failure, KeyboardOwner, WhatsAppApp, what_went_wrong};
use crate::theme::ActiveProductTheme as _;

/// How long a keystroke waits before the list is filtered again.
///
/// Long enough that typing a word is one filter rather than five, short
/// enough that it reads as instant. The conversation's search has no
/// equivalent because it walks messages already in memory, while this one
/// rebuilds a cache over every chat the window holds.
const LIST_DEBOUNCE_MS: u64 = 150;

/// How long the account-wide query waits before it reaches the daemon.
///
/// The index lives behind a process boundary, so every letter would otherwise
/// be a round trip and a fresh set of hits. Longer than the sidebar's filter
/// for that reason, but still short enough to feel like typing into one box.
const GLOBAL_DEBOUNCE_MS: u64 = 220;

/// How many global hits the overlay asks for, and how many it keeps.
///
/// The bound travels to the daemon rather than being applied only here: a
/// thousand hits in one frame costs more to decode than to draw, and the
/// reader cannot use more than a screenful at a time.
const GLOBAL_SEARCH_LIMIT: usize = 50;

/// Both of this window's searches, and the fields they are typed into.
///
/// Two searches rather than one, and deliberately not merged: the sidebar's
/// field filters *chats* by name and the header's searches *messages* in the
/// open conversation. They are together because they are the same kind of
/// state — a query, a box to type it in, and nothing else the window needs —
/// and because Escape has to dismiss them in an order, which is one decision
/// about two things.
///
/// An entity because the pair's methods can then say, in their signatures,
/// that typing a letter into the sidebar changes a query and no chat: none of
/// them can reach a `Context<WhatsAppApp>`. What they cannot do alone is act
/// on a result — filtering the list invalidates the window's cache, and
/// following a match scrolls the window's timeline — so both boxes are built
/// by the window, and it is the window that hears them change.
///
/// That last part is load-bearing rather than incidental. A subscription
/// belongs to the context it is registered in, and gpui hands the handler its
/// entity *leased out of the map*: a handler registered here that called back
/// into the window would return through a method that updates this entity,
/// and a second lease of one entity is a panic, not a glitch. So the boxes
/// are the window's, their events are the window's, and what arrives here is
/// the result.
pub(super) struct Search {
    /// The sidebar's field. Created the first time a window has one to build
    /// it in.
    list_input: Option<Entity<InputState>>,
    language: crate::i18n::Language,
    /// What the list is filtered by: lowercase, trimmed, and one debounce
    /// behind what is in the box.
    list_query: String,
    /// The debounce. Dropped to cancel it, which is what a later keystroke
    /// does.
    debounce: Option<Task<()>>,
    /// Searching inside the open conversation, when that is open. Separate
    /// from the field above, which filters chats by name.
    conversation: Option<ConversationSearch>,
    /// The field for it, created the first time the search is opened.
    conversation_input: Option<Entity<InputState>>,
    /// The account-wide search over stored messages. Not merged with the two
    /// above: those narrow what this window already holds, while this one
    /// asks the daemon to look through the whole store.
    global: Option<GlobalSearch>,
    /// Its field, built by the window and handed over like the others.
    global_input: Option<Entity<InputState>>,
}

impl Search {
    pub(super) fn new() -> Self {
        Self {
            list_input: None,
            language: crate::i18n::Language::current(),
            list_query: String::new(),
            debounce: None,
            conversation: None,
            conversation_input: None,
            global: None,
            global_input: None,
        }
    }

    pub(super) fn refresh_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let language = crate::i18n::Language::current();
        if self.language == language {
            return;
        }
        self.language = language;
        if let Some(input) = &self.list_input {
            input.update(cx, |input, cx| {
                input.set_placeholder(crate::tr!(SearchChats), window, cx)
            });
        }
        if let Some(input) = &self.conversation_input {
            input.update(cx, |input, cx| {
                input.set_placeholder(crate::tr!(SearchInThisConversation), window, cx)
            });
        }
        if let Some(input) = &self.global_input {
            input.update(cx, |input, cx| {
                input.set_placeholder(crate::tr!(GlobalSearchPlaceholder), window, cx)
            });
        }
    }

    pub(super) fn list_input(&self) -> Option<&Entity<InputState>> {
        self.list_input.as_ref()
    }

    /// What the chat list is filtered by, lowercased and trimmed.
    pub(super) fn list_query(&self) -> &str {
        &self.list_query
    }

    /// Whether a search is currently narrowing the list, which is what makes
    /// an empty list mean "no matches" rather than "no chats".
    pub(super) fn narrowing_the_list(&self) -> bool {
        !self.list_query.is_empty()
    }

    pub(super) fn conversation(&self) -> Option<&ConversationSearch> {
        self.conversation.as_ref()
    }

    pub(super) fn conversation_input(&self) -> Option<&Entity<InputState>> {
        self.conversation_input.as_ref()
    }

    /// Put the caret in the sidebar's field.
    ///
    /// The field itself is built by the window — see
    /// [`WhatsAppApp::ensure_chat_search_input`] for why — so this is only
    /// ever asked once there is one.
    pub(super) fn focus_list_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.list_input {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    /// Keep the box the window built for the sidebar.
    pub(super) fn adopt_list_input(&mut self, input: Entity<InputState>) {
        self.list_input = Some(input);
    }

    /// Take what was typed, and say what to do about it now.
    ///
    /// `true` means the list is already filtered differently and the window's
    /// cache is stale; `false` means a debounce is running and the answer
    /// comes later, through the same route.
    ///
    /// Emptying is immediate on purpose: clearing a search is somebody asking
    /// for their list back, and making them wait for a timer to notice is the
    /// one case where the debounce is felt.
    pub(super) fn set_list_query(
        &mut self,
        query: String,
        app: WeakEntity<WhatsAppApp>,
        cx: &mut Context<Self>,
    ) -> bool {
        // Whatever was queued was for an earlier keystroke.
        self.debounce = None;
        if query.is_empty() {
            self.list_query.clear();
            cx.notify();
            return true;
        }
        let trimmed = query.trim().to_lowercase();
        self.debounce = Some(cx.spawn(async move |me: WeakEntity<Self>, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(LIST_DEBOUNCE_MS))
                .await;
            let _ = me.update(cx, |search, cx| {
                search.list_query = trimmed;
                cx.notify();
            });
            let _ = app.update(cx, |app, cx| {
                app.invalidate_chat_cache();
                cx.notify();
            });
        }));
        false
    }

    /// Empty the field and the query behind it.
    ///
    /// The box is set as well as the query, because this is reached from
    /// Escape and from the button as well as from typing — and a query
    /// cleared under text still on screen is a list that disagrees with its
    /// own field.
    ///
    /// Dropping the debounce is the other half of that, and it is a fix
    /// rather than tidying: the timer armed by the last keystroke used to
    /// outlive the clear, land a hundred milliseconds later, and filter the
    /// list by a word that was no longer in the box — with nothing on screen
    /// to say why the chats had gone.
    pub(super) fn clear_list_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.list_input {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.debounce = None;
        self.list_query.clear();
        cx.notify();
    }

    /// Everything a departing account typed into either field.
    pub(super) fn forget(&mut self, cx: &mut Context<Self>) {
        self.debounce = None;
        self.list_query.clear();
        self.conversation = None;
        self.global = None;
        cx.notify();
    }

    /// Open the search over `jid`.
    ///
    /// The box is the window's to build, and is handed over on the first
    /// opening; every later one reuses it.
    pub(super) fn open_conversation(
        &mut self,
        jid: String,
        input: Option<Entity<InputState>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.conversation = Some(ConversationSearch::new(jid));
        if let Some(input) = input {
            self.conversation_input = Some(input);
        }
        if let Some(input) = &self.conversation_input {
            input.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn close_conversation(&mut self, cx: &mut Context<Self>) -> bool {
        if self.conversation.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// Re-run the conversation's query over `messages`, and say which message
    /// the timeline should be showing.
    ///
    /// The messages come from above because a chat is the window's. Nothing
    /// is cloned to get them here: the search is refreshed in place, which is
    /// what the borrow used to make impossible — holding one field mutably
    /// while reading another forced a copy of the entire message vector, on
    /// every keystroke, in the longest conversation the reader has.
    pub(super) fn refresh_conversation(
        &mut self,
        query: &str,
        messages: &[ChatMessage],
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let search = self.conversation.as_mut()?;
        search.refresh(query, messages);
        cx.notify();
        search.current_match().map(str::to_string)
    }

    /// Walk the matches. `forward` is down the timeline, the way reading goes.
    pub(super) fn step_conversation(
        &mut self,
        forward: bool,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let target = self
            .conversation
            .as_mut()?
            .step(forward)
            .map(str::to_string);
        if target.is_some() {
            cx.notify();
        }
        target
    }

    /// Whether an open search is about `jid` and has matches that a change
    /// to that conversation could have made wrong.
    pub(super) fn stale_for(&self, jid: &str) -> bool {
        self.conversation
            .as_ref()
            .is_some_and(|search| search.jid == jid && !search.query.is_empty())
    }

    /// Re-run the open search over the messages it is about.
    ///
    /// The matches used to be rebuilt only when the *query* changed, so a
    /// message arriving, a history merge, an edit or a revoke left the count
    /// and the navigation describing a conversation that had moved on — with
    /// no way to correct it but to retype the query.
    pub(super) fn refresh_open(&mut self, messages: &[ChatMessage], cx: &mut Context<Self>) {
        let Some(search) = self.conversation.as_mut() else {
            return;
        };
        let query = search.query.clone();
        search.refresh(&query, messages);
        cx.notify();
    }

    /// Close a search that was typed for a different conversation.
    ///
    /// A search belongs to the conversation it was typed for, so leaving that
    /// conversation closes it rather than carrying a query into a chat it
    /// says nothing about.
    pub(super) fn close_unless_about(&mut self, jid: &str, cx: &mut Context<Self>) {
        if self
            .conversation
            .as_ref()
            .is_some_and(|search| search.jid != jid)
        {
            self.conversation = None;
            cx.notify();
        }
    }

    /// Which chat the open search belongs to, if one is open.
    pub(super) fn conversation_jid(&self) -> Option<&str> {
        self.conversation.as_ref().map(|search| search.jid.as_str())
    }

    /// The account-wide search, while its overlay is up.
    pub(super) fn global(&self) -> Option<&GlobalSearch> {
        self.global.as_ref()
    }

    pub(super) fn global_input(&self) -> Option<&Entity<InputState>> {
        self.global_input.as_ref()
    }

    /// Open the overlay, adopting the field the window built for it.
    pub(super) fn open_global(
        &mut self,
        input: Option<Entity<InputState>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = input {
            self.global_input = Some(input);
        }
        if self.global.is_none() {
            self.global = Some(GlobalSearch::default());
        }
        if let Some(input) = &self.global_input {
            input.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn close_global(&mut self, cx: &mut Context<Self>) -> bool {
        if self.global.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// Set the query the daemon will be asked about.
    ///
    /// The hits on screen belong to the old question, so they go with it: an
    /// answer carrying the previous query arrives later and is dropped by
    /// [`Self::adopt_global_results`].
    pub(super) fn set_global_query(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(global) = self.global.as_mut() else {
            return;
        };
        if global.query == query {
            return;
        }
        global.query = query;
        global.results.clear();
        global.pending = false;
        cx.notify();
    }

    /// Adopt hits only if they answer the query now in the box.
    ///
    /// The daemon broadcasts results without a request id, so the query is the
    /// whole correlation. A late answer to a query that has since changed is
    /// somebody else's answer as far as this overlay is concerned: showing it
    /// would put messages under a question the reader did not ask.
    pub(super) fn adopt_global_results(
        &mut self,
        query: String,
        results: Vec<MessageSearchHit>,
        cx: &mut Context<Self>,
    ) {
        let Some(global) = self.global.as_mut() else {
            return;
        };
        if global.query != query {
            return;
        }
        global.pending = false;
        global.results = {
            let mut results = results;
            results.sort_unstable_by_key(|hit| std::cmp::Reverse(hit.timestamp_ms));
            results.truncate(GLOBAL_SEARCH_LIMIT);
            results
        };
        cx.notify();
    }

    pub(super) fn mark_global_pending(&mut self, cx: &mut Context<Self>) {
        if let Some(global) = self.global.as_mut() {
            global.pending = true;
            cx.notify();
        }
    }
}

/// The stored-history search, while its overlay is up.
#[derive(Default, Clone)]
pub struct GlobalSearch {
    /// Exactly what was sent to the daemon. Trimming happens before this, so
    /// the echoed query can be compared without guessing about whitespace.
    pub query: String,
    /// Newest first, and never more than [`GLOBAL_SEARCH_LIMIT`].
    pub results: Vec<MessageSearchHit>,
    /// Whether a request is out for the current query. Cleared by an answer
    /// whose query matches, or by changing the query.
    pub pending: bool,
}

/// One conversation's search, while it is open.
pub struct ConversationSearch {
    /// The chat being searched. Switching chats closes the search rather than
    /// carrying a query into a conversation it was never typed for.
    pub jid: String,
    /// Lowercased and trimmed, like the chat-list query.
    pub query: String,
    /// Ids of the matching messages, newest last — timeline order, so
    /// "next" walks the way the eye does.
    pub matches: Vec<String>,
    /// Which match is current, when there are any.
    pub current: usize,
}

impl ConversationSearch {
    pub fn new(jid: String) -> Self {
        Self {
            jid,
            query: String::new(),
            matches: Vec::new(),
            current: 0,
        }
    }

    /// The id the timeline should be showing.
    pub fn current_match(&self) -> Option<&str> {
        self.matches.get(self.current).map(String::as_str)
    }

    /// "3 of 12", or what to say instead.
    ///
    /// The empty answer names its horizon. A conversation holds the messages
    /// this window has loaded — one page, unless the reader has asked for
    /// more — while the store behind the daemon holds the rest and an FTS
    /// index over it. "No matches" is a claim about the whole history that
    /// this search is in no position to make; "in the loaded messages" is
    /// what it actually looked at.
    pub fn status(&self) -> Option<String> {
        if self.query.is_empty() {
            return None;
        }
        if self.matches.is_empty() {
            return Some(crate::tr!(NoMatchesInTheLoadedMessages).to_string());
        }
        Some(crate::tr!(
            match_count,
            self.current + 1,
            self.matches.len()
        ))
    }

    pub fn has_matches(&self) -> bool {
        !self.matches.is_empty()
    }

    /// Re-run the query over `messages`.
    ///
    /// Keeps the reader where they were when it can: narrowing a query
    /// usually leaves the current hit in the result, and jumping them back to
    /// the top of the conversation for a typed character is disorienting.
    pub fn refresh(&mut self, query: &str, messages: &[ChatMessage]) {
        let previous = self.current_match().map(str::to_string);
        self.query = query.trim().to_lowercase();
        self.matches = if self.query.is_empty() {
            Vec::new()
        } else {
            messages
                .iter()
                .filter(|message| matches(message, &self.query))
                .map(|message| message.id.clone())
                .collect()
        };
        self.current = previous
            .and_then(|id| self.matches.iter().position(|candidate| *candidate == id))
            // Otherwise the newest match, which is the one nearest where the
            // reader already is.
            .unwrap_or(self.matches.len().saturating_sub(1));
    }

    /// Step to the next match, wrapping. Returns the id to jump to.
    pub fn step(&mut self, forward: bool) -> Option<&str> {
        if self.matches.is_empty() {
            return None;
        }
        let last = self.matches.len() - 1;
        self.current = if forward {
            if self.current >= last {
                0
            } else {
                self.current + 1
            }
        } else if self.current == 0 {
            last
        } else {
            self.current - 1
        };
        self.current_match()
    }
}

/// Whether a message answers `query`, which is already lowercased.
///
/// Captions and file names count: a photo is often remembered by what was
/// said about it, and a document by what it was called.
fn matches(message: &ChatMessage, query: &str) -> bool {
    if message.content.to_lowercase().contains(query) {
        return true;
    }
    message.media.as_ref().is_some_and(|media| {
        media
            .file_name
            .as_ref()
            .is_some_and(|name| name.to_lowercase().contains(query))
    })
}

/// One result row, gathered where the chats are so the render helper never
/// has to reach back into the app it is already drawn inside.
pub(super) struct SearchResultRow {
    pub chat_jid: SharedString,
    pub chat_name: SharedString,
    pub message_id: SharedString,
    pub snippet: SharedString,
    pub timestamp_ms: i64,
    pub has_media: bool,
}

impl WhatsAppApp {
    /// Open the account-wide search over the daemon's stored index.
    pub(super) fn open_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Built and subscribed here, like the other two boxes: a handler owned
        // by `Search` would be handed that entity leased out of the map, and
        // every answer it has goes back through methods on the window.
        let input = self.search.read(cx).global_input().is_none().then(|| {
            use gpui_component::input::InputEvent;

            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder(crate::tr!(GlobalSearchPlaceholder))
            });
            cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
                if let InputEvent::Change = event {
                    let query = input.read(cx).value().to_string();
                    this.set_global_query(query, cx);
                }
            })
            .detach();
            input
        });
        self.search
            .update(cx, |search, cx| search.open_global(input, window, cx));
        self.keyboard_owner = Some(KeyboardOwner::GlobalSearch);
        cx.notify();
    }

    pub(super) fn close_global_search(&mut self, cx: &mut Context<Self>) -> bool {
        self.global_search_debounce = None;
        let closed = self.search.update(cx, |search, cx| search.close_global(cx));
        if closed {
            self.keyboard_owner = None;
            cx.notify();
        }
        closed
    }

    /// The debounced query is sent from here because only the window owns the
    /// session client. The box itself stays a dumb field.
    fn set_global_query(&mut self, query: String, cx: &mut Context<Self>) {
        let query = query.trim().to_string();
        self.search
            .update(cx, |search, cx| search.set_global_query(query.clone(), cx));
        self.global_search_debounce = None;
        if query.is_empty() {
            return;
        }
        let weak = cx.entity().downgrade();
        self.global_search_debounce = Some(cx.spawn(async move |_, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(GLOBAL_DEBOUNCE_MS))
                .await;
            let _ = weak.update(cx, |app, cx| app.request_global_search(cx));
        }));
    }

    /// Ask the daemon for the current query. The ack only says the request ran;
    /// the hits arrive separately as a broadcast carrying the query.
    fn request_global_search(&mut self, cx: &mut Context<Self>) {
        let Some(query) = self.search.read(cx).global().map(|g| g.query.clone()) else {
            return;
        };
        if query.is_empty() {
            return;
        }
        let Some(waiting) = self
            .client
            .as_ref()
            .map(|client| client.search_messages(query.clone(), None, false, GLOBAL_SEARCH_LIMIT))
        else {
            self.notify_user(
                what_went_wrong(
                    crate::tr!(CouldNotSearchMessages),
                    &Failure::worth_retrying(crate::tr!(DaemonDisconnectedBeforeAnswer)),
                ),
                crate::app::notices::Tone::Problem,
                cx,
            );
            return;
        };
        self.search
            .update(cx, |search, cx| search.mark_global_pending(cx));
        cx.spawn(async move |entity, cx| {
            let failure = match waiting.await {
                Ok(Ok(())) => None,
                Ok(Err(failure)) => Some(failure),
                Err(_) => Some(Failure::worth_retrying(crate::tr!(
                    DaemonDisconnectedBeforeAnswer
                ))),
            };
            let _ = entity.update(cx, |app, cx| {
                if let Some(failure) = failure {
                    app.notify_user(
                        what_went_wrong(crate::tr!(CouldNotSearchMessages), &failure),
                        crate::app::notices::Tone::Problem,
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// Adopt an account-wide answer, dropping one that belongs to an older
    /// query. The correlation lives in `Search`, not here, so no other caller
    /// can accidentally bypass it.
    pub(super) fn adopt_search_results(
        &mut self,
        query: String,
        results: Vec<MessageSearchHit>,
        cx: &mut Context<Self>,
    ) {
        self.search.update(cx, |search, cx| {
            search.adopt_global_results(query, results, cx)
        });
    }

    /// Whether this row is the one a search hit asked to see.
    pub(crate) fn is_focused_message(&self, chat_jid: &str, message_id: &str) -> bool {
        self.focused_message
            .as_ref()
            .is_some_and(|focused| focused.0 == chat_jid && focused.1 == message_id)
    }

    /// Close the overlay and take the reader to the hit they clicked.
    pub(super) fn open_search_hit(
        &mut self,
        chat_jid: &str,
        message_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_global_search(cx);
        self.focused_message = Some((chat_jid.to_string(), message_id.to_string()));
        self.select_chat(chat_jid.to_string(), ChatOpen::ToPreview, window, cx);
        // A hit outside the loaded page has no row to outline. The chat is
        // still open, which is the most that can honestly be done without a
        // protocol for paging to an arbitrary id.
        self.jump_to_message(message_id, cx);
        cx.notify();
    }
}

/// The floating, account-wide search. Kept out of the sidebar and the
/// conversation header: both of those search a horizon this one does not.
pub(super) fn render_global_search(
    input: &Entity<InputState>,
    focus: &FocusHandle,
    global: &GlobalSearch,
    rows: &[SearchResultRow],
    app: &Entity<WhatsAppApp>,
    cx: &App,
) -> impl IntoElement + use<> {
    let metrics = cx.product().metrics;
    let status = if global.query.is_empty() {
        crate::tr!(GlobalSearchPlaceholder).to_string()
    } else if global.pending {
        crate::tr!(GlobalSearchSearching).to_string()
    } else if rows.is_empty() {
        crate::tr!(GlobalSearchNoResults).to_string()
    } else {
        format!("{} · {}", crate::tr!(GlobalSearchResults), rows.len())
    };
    let close_app = app.clone();
    let result_rows: Vec<_> = rows
        .iter()
        .map(|row| {
            let hit_app = app.clone();
            let chat_jid = row.chat_jid.clone();
            let message_id = row.message_id.clone();
            let chat_name = row.chat_name.clone();
            let jid_label = row.chat_jid.clone();
            let snippet = row.snippet.clone();
            let time = chrono::DateTime::from_timestamp_millis(row.timestamp_ms)
                .map(|when| crate::utils::format_time_local(&when))
                .unwrap_or_default();
            let has_media = row.has_media;
            div()
                .id(format!("global-search-hit-{}-{message_id}", row.chat_jid))
                .debug_selector(|| "global-search-hit".into())
                .w_full()
                .cursor_pointer()
                .rounded(metrics.radius_md())
                .px(metrics.space_md())
                .py(metrics.space_sm())
                .hover(|el| el.bg(cx.theme().list_hover))
                .on_click(move |_, window, cx| {
                    hit_app.update(cx, |app, cx| {
                        app.open_search_hit(&chat_jid, &message_id, window, cx);
                    });
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(metrics.space_sm())
                        .child(
                            div()
                                .text_size(metrics.text_strong())
                                .text_color(cx.theme().foreground)
                                .child(chat_name),
                        )
                        .child(
                            div()
                                .text_size(metrics.text_micro())
                                .text_color(cx.theme().muted_foreground)
                                .child(jid_label),
                        ),
                )
                .child(
                    div()
                        .text_size(metrics.text_secondary())
                        .text_color(cx.theme().foreground)
                        .child(snippet),
                )
                .child(
                    div()
                        .mt(metrics.space_xxs())
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(metrics.space_sm())
                        .child(
                            div()
                                .text_size(metrics.text_micro())
                                .text_color(cx.theme().muted_foreground)
                                .child(if has_media {
                                    SharedString::from(format!(
                                        "{} · {}",
                                        crate::tr!(GlobalSearchMedia),
                                        time
                                    ))
                                } else {
                                    SharedString::from(time)
                                }),
                        ),
                )
        })
        .collect();
    div()
        .id("global-search-overlay")
        .debug_selector(|| "global-search-overlay".into())
        .track_focus(focus)
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .pt(metrics.space_xxl())
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
                        .gap(metrics.space_sm())
                        .child(
                            Icon::new(IconName::Search)
                                .with_size(metrics.icon())
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(Input::new(input).w_full())
                        .child(
                            Button::new("global-search-close")
                                .icon(IconName::Close)
                                .ghost()
                                .tooltip(crate::tr!(Close))
                                .on_click(move |_, _, cx| {
                                    close_app.update(cx, |app, cx| {
                                        app.close_global_search(cx);
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
                .child(v_flex().gap(metrics.space_xs()).children(result_rows)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxidezap_core::fixtures;

    /// A [`Search`] with the conversation half open over `jid`, and no window
    /// behind it. What the tests below drive is the part that decides — when
    /// an open search has gone stale, and what a departing account leaves.
    fn searching(jid: &str, query: &str) -> Search {
        let mut search = Search::new();
        let mut open = ConversationSearch::new(jid.to_string());
        open.refresh(query, &history());
        search.conversation = Some(open);
        search
    }

    /// Every path that invalidates a chat's timeline asks this, on every
    /// chat: the answer has to be cheap and it has to be about *this*
    /// conversation, or a message arriving in one chat would re-run the
    /// search open over another.
    #[test]
    fn only_the_searched_conversation_goes_stale() {
        let search = searching("chat", "invoice");

        assert!(search.stale_for("chat"));
        assert!(!search.stale_for("someone-else"));
    }

    /// An empty query has no matches to go stale, and re-running it on every
    /// message that arrives is a walk over the whole conversation for a
    /// result that is already empty.
    #[test]
    fn a_search_nobody_has_typed_into_yet_is_never_stale() {
        let search = searching("chat", "   ");

        assert!(!search.stale_for("chat"));
    }

    /// The list filter is the one thing here that changes what the sidebar
    /// draws, and an empty list means "no matches" only while it is set.
    #[test]
    fn the_list_is_narrowed_only_while_a_query_is_held() {
        let mut search = Search::new();
        assert!(!search.narrowing_the_list());

        search.list_query = "invoice".to_string();

        assert!(search.narrowing_the_list());
    }

    fn history() -> Vec<ChatMessage> {
        vec![
            fixtures::message("1", fixtures::PEER, "the invoice is attached"),
            fixtures::message("2", fixtures::PEER, "thanks"),
            fixtures::message("3", fixtures::PEER, "Invoice again, sorry"),
        ]
    }

    #[test]
    fn matching_ignores_case_and_keeps_timeline_order() {
        let mut search = ConversationSearch::new("chat".into());
        search.refresh("INVOICE", &history());
        assert_eq!(search.matches, vec!["1".to_string(), "3".to_string()]);
    }

    #[test]
    fn the_newest_match_is_the_one_shown_first() {
        let mut search = ConversationSearch::new("chat".into());
        search.refresh("invoice", &history());
        assert_eq!(search.current_match(), Some("3"));
        assert_eq!(search.status().as_deref(), Some("2 of 2"));
    }

    #[test]
    fn stepping_wraps_in_both_directions() {
        let mut search = ConversationSearch::new("chat".into());
        search.refresh("invoice", &history());
        assert_eq!(search.step(true), Some("1"), "forward from the last wraps");
        assert_eq!(search.step(false), Some("3"), "and back again");
    }

    /// Typing one more character must not throw the reader back to the top.
    #[test]
    fn narrowing_a_query_keeps_the_current_hit() {
        let history = history();
        let mut search = ConversationSearch::new("chat".into());
        search.refresh("invoice", &history);
        search.step(true);
        assert_eq!(search.current_match(), Some("1"));

        search.refresh("invoice is", &history);
        assert_eq!(search.current_match(), Some("1"));
    }

    #[test]
    fn an_empty_query_says_nothing_rather_than_no_matches() {
        let mut search = ConversationSearch::new("chat".into());
        search.refresh("   ", &history());
        assert!(search.status().is_none());
        assert!(!search.has_matches());
    }

    #[test]
    fn a_query_nothing_answers_says_so() {
        let mut search = ConversationSearch::new("chat".into());
        search.refresh("receipt", &history());
        assert_eq!(
            search.status().as_deref(),
            Some("No matches in the loaded messages")
        );
    }

    fn hit(id: &str, timestamp_ms: i64) -> MessageSearchHit {
        MessageSearchHit {
            chat_jid: "559900000001@s.whatsapp.net".into(),
            message_id: id.into(),
            snippet: "a synthetic hit".into(),
            sender_jid: None,
            timestamp_ms,
            has_media: false,
        }
    }

    /// Build a `Search` with only its global half open; the window would do
    /// this in production, and the tests here only drive the answer's rules.
    fn global_search(query: &str) -> Search {
        let mut search = Search::new();
        search.global = Some(GlobalSearch {
            query: query.into(),
            results: Vec::new(),
            pending: true,
        });
        search
    }

    #[gpui::test]
    fn global_hits_are_bounded_and_newest_first(cx: &mut gpui::TestAppContext) {
        let search = cx.update(|cx| cx.new(|_| global_search("invoice")));
        let hits: Vec<_> = (0..GLOBAL_SEARCH_LIMIT + 4)
            .map(|i| hit(&format!("id-{i}"), i as i64))
            .collect();
        cx.update(|cx| {
            search.update(cx, |search, cx| {
                search.adopt_global_results("invoice".into(), hits, cx)
            });
        });

        let global = cx.update(|cx| search.read(cx).global().unwrap().clone());
        assert!(!global.pending);
        assert_eq!(global.results.len(), GLOBAL_SEARCH_LIMIT);
        assert_eq!(global.results[0].message_id, "id-53");
        assert_eq!(
            global.results.last().unwrap().message_id,
            "id-4",
            "the oldest hits are the ones dropped"
        );
    }

    #[gpui::test]
    fn a_global_answer_for_an_old_query_is_dropped(cx: &mut gpui::TestAppContext) {
        let search = cx.update(|cx| cx.new(|_| global_search("invoice")));
        cx.update(|cx| {
            search.update(cx, |search, cx| {
                search.adopt_global_results("receipt".into(), vec![hit("old", 1)], cx)
            });
        });

        let global = cx.update(|cx| search.read(cx).global().unwrap().clone());
        assert!(global.results.is_empty());
        assert!(global.pending, "the live question is still waiting");

        cx.update(|cx| {
            search.update(cx, |search, cx| {
                search.set_global_query("receipt".into(), cx)
            });
        });
        let global = cx.update(|cx| search.read(cx).global().unwrap().clone());
        assert_eq!(global.query, "receipt");
        assert!(!global.pending);
    }
}
