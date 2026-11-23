# Our changes on top of upstream

> This is the receipt pile. Every local change is recorded here with its commit
> and the gates it passed. No mystery diffs, no "we rewrote the whole thing"
> bullshit. If a change is not in this file, ask why.

Base: `oxidezap/client` @ `ea67a2aa41279232029b5872528ec394fb491e98` (2026-11-20),
tagged `base-upstream-ea67a2aa`.

Upstream remote: `https://github.com/oxidezap/client.git`
Our remote: `https://github.com/duckb1t/whatsnative.git`

This file exists so a monthly `git fetch upstream && git rebase upstream/main` stays
reviewable. Record every local change here with its commit and reason. Small diffs only:
upstream already owns the protocol stack, daemon/IPC, native audio/video, GPUI UI,
SQLite store, plugins and Linux packaging.

## Environment / baseline (no source changes)

- Rust 1.100.0 via rustup (system 1.99.1 too old; project pins 1.100.0).
- Native deps present: ALSA, X11, Wayland, xkbcommon, fontconfig, freetype, FFmpeg.
- `cargo check -p oxidezap-core`, passed.
- `cargo build --release --bin oxidezap --bin oxidezapd --bin oxidezap-cli`, passed (24m30s).
- Binaries: `oxidezap` 46M, `oxidezapd` 31M, `oxidezap-cli` 1.5M (stripped).
- `oxidezap-cli doctor`: daemon running, DB healthy, pairing QR issued.
- GUI verified as native Wayland window (`org.oxidezap.client.local` at baseline,
  renamed to `org.whatsnative.client.local` in section 23, `xwayland: 0`).

## Local changes

### 1. Group calls keep their group identity (`group_jid`)

**Problem.** `wacore::types::call::CallAction::Offer` carries `group_jid`, but
`crates/session/src/whatsapp/mod.rs` matched it with `..` and the resulting
`oxidezap_core::IncomingCall` had nowhere to put it. Downstream,
`daemon/session_bridge/call_log.rs` derived `is_group` from `caller_jid`, which
for a group offer is the *person* who started it, so a group call was logged
as a 1:1 call in that person's chat.

**Change.**
- `core::IncomingCall` gains `group_jid: Option<String>` with
  `#[serde(default, skip_serializing_if = "Option::is_none")]`, derived inside
  `IncomingCall::new` from the library offer so no caller can omit it. Old
  daemon frames still deserialize.
- `call_log.rs` uses the group as `chat_jid` and sets `is_group` from the new
  field (still falling back to a `@g.us` suffix check). `caller_jid` remains
  the person who rang.
- 4 new tests: group derived from offer, direct offer has none, legacy frame
  reads, and a group call lands in the group's log entry.

**Not changed:** the library's generic `voip.accept()` already handles group
roster, relay and key epochs (`AcceptCall::start` checks `incoming.group`), so
answering a group call needs no client-side work. The participant grid this
flag was needed for is now reachable, see change 3.

### 2. Linux package staging smoke test

CI checked that deb/rpm/AppImage files exist, but nothing checked the FHS tree
they are built from. `packaging/linux/smoke-test.sh` now verifies, with only a
POSIX shell: the three binaries co-located and executable in `usr/bin`, a
desktop entry and AppStream metadata whose ids agree and have no unsubstituted
`@VERSION@`/`@DATE@`, and the seven hicolor PNGs. Wired into `build.yml` after
`package.sh` on both Linux runners. `docs/linux-packaging.md` documents it and
marks the old glibc/version figures as measurements from specific builds.

### 3. Group calls: rosters, an entry point and the participant grid

**Problem.** A group offer names the group but carries no participant list; the
authoritative, transaction-ordered snapshots belong to the library's call
registry. Nothing carried them past the session, and a group could be answered
but never started from a group chat.

**Change (protocol, commit `ca251b9`).**
- `core::CallParticipant`, `CallGroup` and the tuple-shaped aliases
  `CallParticipantRow` / `CallParticipantRef` carry the roster as one additive,
  defaulted `group` field on outgoing, incoming, active and history call types.
  Older daemon frames still deserialize; every group getter falls back to the
  flat `group_jid` field.
- `set_incoming` refreshes the roster of the call already on screen instead of
  ringing again, and refuses snapshots older than the one already drawn so a
  slow name lookup cannot rewind the grid.
- New `CallAction::StartGroup` (protocol version **45**) reuses the shared
  placement path and the library's roster-resolving `group_call_by_id()`.
- `CallParticipantDto` and group columns in call history + `oxidezap-cli calls
  --participants`. `DaemonEvent::CallsChanged` is boxed (serde-transparent) to
  keep the event enum small.

**Change (GUI, this commit).**
- `core::is_group_jid` is the one predicate for "this address is a group". The
  chat header gate, the optimistic outgoing card and `SessionHandle::start_call`
  all use it, including the camera-preview confirmation that re-enters with
  only the JID, no group flag to lose on that path.
- Group JIDs now show the call buttons; both toolbar and overflow menu route
  to one entry point. `SessionHandle::start_group_call` sends `StartGroup`.
- `CallState::group_roster()` feeds the existing `active_group` grid. A
  participant with no resolved name is drawn by JID. Screen-share and
  add-participant stay visibly disabled with honest tooltips.

**Gates.** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings`, `cargo test --workspace --all-features`, all
green; GUI 680 passed, 14 ignored.

**Not done:** real group calls cannot be validated without explicit consent to
place or answer one on the paired account (see the consent boundary in the
plan). Only synthetic fixtures were exercised.

### 4. Block list in Settings → Privacy

**Problem.** The account's server-owned block list was unreachable from any
front end: upstream `client.blocking()` existed, but the repo had no domain
type, IPC action or UI for it. Blocked contacts are safety state, so guessing
one from chats or contacts would be wrong.

**Change.**
- `core::BlockedContact` `{ jid, name, blocked_at }` is the domain type and the
  wire shape; `label()` falls back to the JID and `normalize_block_target()`
  validates input (bare numbers become PN JIDs; PN/LID JIDs accepted; device
  suffixes stripped; groups/newsletters/malformed input refused).
- `session/whatsapp/blocklist.rs` wraps `get_blocklist`, `block` and `unblock`,
  resolving names locally and keeping the exact server JID so Unblock sends
  the right address and namespace.
- IPC adds `ClientRequest::{Blocklist, Block, Unblock}` and a correlated
  `DaemonMessage::Blocklist { id, entries }`; **protocol version 46**.
  Block/unblock are correlated `ask`s, not `tell`s, they mutate the real
  account and must report a refusal. Mutations answer after the server IQ;
  the fetch is single-flight so a second click cannot strand a waiter.
- Settings → Privacy lists blocked JIDs with Unblock buttons, an Add-by-JID
  field and a Refresh button. The list is re-read on pane arrival and after
  each successful mutation. Errors use the existing notice surface; EN/PT
  strings added to the i18n catalog.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace test suite all
green; GUI 680 passed, 14 ignored. No real block/unblock was performed.

### 5. Username lookup in Settings → Contacts

**Problem.** Upstream's experimental `contacts().find_by_username()` was
unreachable from the front end. The repo already carried `username` and
`verified_name` fields in profile DTOs, but had no way to ask the server for
one.

**Change.**
- `core::UsernameLookupResult` is a typed answer: `Found(ResolvedUsername)`,
  `KeyRequired`, `NotFound` or locally-invalid handle. These stay separate,
  "give me the key" is not "nobody has this handle". `normalize_username()`
  strips the display-only `@` and enforces the server's 3..=35 length locally.
- `session/whatsapp/username_lookup.rs` calls upstream, converts the answer to
  domain data, and adds only the local address-book name. The verified name is
  the protocol's own field; a handle never becomes a fake business name.
- IPC adds `ClientRequest::LookupUsername { username, username_key }` and a
  correlated `DaemonMessage::UsernameLookup { id, result }`; protocol version
  **47**. The key is runtime input only, never logged or hardcoded. Failures
  use the existing `ProtocolError::Failed` path.
- A new Settings → Contacts pane holds the handle/key fields and the result
  card, keeps one lookup in flight, and lets a `KeyRequired` answer be retried
  with the typed key. EN/PT strings added.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace test suite all
green; no real lookup was performed.

### 6. Per-request failures for reactions, poll votes and plugin approval

**Problem.** Three user-visible `tell` requests had no completion: a reaction
or poll vote drawn optimistically stayed on screen after the daemon refused
it, and a refused plugin permission change was only a log line. Call paths
were deliberately excluded.

**Change.**
- `SessionHandle::{send_reaction, vote_poll, plugin_approval}` now use the
  existing `ask` + `Awaiting::Mutation` correlation and return a oneshot. No
  new IPC variants, Awaiting variants or frames routing; protocol unchanged.
- The app layer awaits each answer: on failure it restores the pre-tap
  reaction map (covers switching emoji), removes the attempted poll vote, and
  raises a `what_went_wrong(...)` notice with `Tone::Problem`. Plugin approval
  draws nothing optimistically, so its failure path only notifies.
- The daemon now awaits `SendReaction`/`VotePoll` inline in `act.rs` and maps
  task failures to `CommandOutcome::Refused(detail)` instead of dropping them.
- Typing, read receipts, history/plugin reloads, telemetry and all call paths
  remain fire-and-forget on purpose: they are ephemeral, republished state or
  outside the consent boundary.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace test suite all
green. Synthetic tests only; no real reaction or vote was sent. Session
errors are plain strings, so they read as permanent in `what_went_wrong`,
a retryable `CommandOutcome::Failed` is a separate, later change.

### 7. Core vocabulary for events, communities and channels

Prepared the domain types the next three parity features will share, with no
protocol or UI change yet:

- `core::EventCard` / `EventLocation` / `EventRsvp` / `CreatedMessageRef`, and
  an additive `ChatMessage.event` field (serde default, cleared on revoke).
- `core::Community*` request/result types, preserving partial link failures and
  warnings instead of collapsing them into one error.
- `core::Channel{PollDraft,React,VotePoll,PollCreated}`; server ids and option
  hashes stay behind the session boundary.
- 10 synthetic serde/default tests. `impl Eq` is kept on the float-bearing
  event types because `ChatMessage` and the wire enums above it derive `Eq`;
  protocol coordinates are finite by definition.

Four `ChatMessage` literals in `session/whatsapp/{convert.rs,mod.rs,tests.rs}`
were updated with `event: None`. No real account data or protocol use yet.

### 8. Native calendar events (protocol 48)

**Problem.** The repo only materialized `event_message` rows; there was no
domain card, no way to create an event and no way to RSVP. The library already
had `Events::create/respond/respond_raw`.

**Change.**
- `session/whatsapp/events.rs` creates through `EventCreationParams` and
  returns only `CreatedMessageRef`. RSVP loads the stored creation, requires
  its protobuf event, resolves the creator with the poll-vote identity rule,
  prefers `message_context_info.message_secret` with the `msg_secrets` index
  as fallback, and calls `respond_raw`. Neither the secret nor `CreatedEvent`
  crosses IPC.
- Event protobuf is projected into `core::EventCard` on both live and hydrated
  paths; no DB migration. The card renders name, description, known time
  range, location and join link, and three RSVP buttons. An answer is drawn as
  "requested" until the correlated daemon answer; a refusal takes it back.
- A create dialog in the chat header keeps every field but the name optional,
  validates numbers and half-pins locally, and surfaces failures as notices.
- IPC adds `ClientRequest::{CreateEvent, RespondEvent}` and
  `DaemonMessage::EventCreated { id, created }`; protocol version **48**.
  Creation is addressed/out-of-band; RSVP uses the existing correlated
  mutation path. No CLI wire changes.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace test suite all
green; synthetic fixtures only, no real event or RSVP was sent.

### 9. Native communities (protocol 49)

**Problem.** Groups had hierarchy badges but no way to create a community or
subgroup, link existing groups, or join a subgroup. The library's community API
can finish in part, so a bare acknowledgement was not enough.

**Change.**
- `session/whatsapp/communities.rs` wraps create/create-subgroup/link/join with
  the library's `CreateSubgroupOptions`/`LinkSubgroupOptions` so per-target
  visibility is kept. Partial cases become `Ok` with typed data: a failed
  description write still reports the created group plus a `SetDescription`
  warning; approval-pending join is a `JoinApprovalPending` warning; refused
  groups stay typed failures. Hard errors become strings.
- IPC adds four `ClientRequest` variants and a correlated
  `DaemonMessage::CommunityResult { id, result }`; protocol version **49**.
  The daemon answers out-of-band under the request id (like events) with
  `ProtocolError::Failed`/`NoSession` for hard errors. No CLI wire changes.
- GUI adds `Awaiting::Community`, four correlated `SessionHandle` methods, and
  a four-mode dialog reachable from the Communities `+` button and empty
  state. A typed result shows per-item notices and navigates to a created
  group already in the chat list; a hard failure keeps the form open with the
  daemon's reason. All strings are in the EN/PT catalog.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2594 passed). One pre-existing timing-sensitive sticker-animation test flakes
under full-suite load (passes in isolation and on clean rerun); unrelated to
this slice. Synthetic JIDs only; no real community operation was performed.

### 10. Native channel posts, polls, reactions and votes (protocol 50)

**Problem.** Channels were a read-only destination with a shared composer.
Posting was unnamed and left no durable row; reactions and poll votes went
through the E2E APIs, which reject newsletter JIDs, because the server content
id they need was never captured. The prepared `core::Channel*` types were inert.

**Change.**
- A new `messages.newsletter_server_id BIGINT` column (nullable) carries the
  server content id from live newsletter pushes
  (`InboundMessage.info.newsletter_server_id`). Zero is a real id; only SQL
  `NULL` means absent. The value is stored as `u64 as i64` and read back
  bitwise, so ids above `i64::MAX` survive. A live copy can fill a missing id
  on a stale history row without rewriting its content.
- History sync deliberately stores **no** id: a forwarded-newsletter context
  names the message in the *original* channel, not the row being stored, and
  using it could react to the wrong channel. Older history rows therefore
  return a clear "no server id yet" error until a live copy arrives.
- `session/whatsapp/ops.rs` adds `post_to_channel`, `create_channel_poll`,
  `react_to_channel` and `vote_channel_poll`. A post carries the window's
  optimistic id as its WhatsApp message id and records the outgoing row
  before sending, so the bubble and the durable row are one message. Poll
  creation records the returned proto/id after the send; the store's deferred
  ack queue absorbs an early acknowledgement. Reactions and votes load the
  stored row, require its server id, and hash the exact stored option names
  with `wacore::poll::compute_option_hash`.
- IPC adds `ClientRequest::{PostToChannel, CreateChannelPoll, ReactToChannel,
  VoteChannelPoll}` and `DaemonMessage::ChannelPollCreated { id, created }`;
  protocol version **50**. Poll creation is answered out-of-band under the
  request id; the other three use the correlated mutation path so refusals
  reach the press that made them. No CLI/wire changes and no call-path changes.
- GUI routes channel text, reactions and votes to the channel session methods;
  ordinary chats keep the E2E paths. A channel-only "Create poll" action in
  the chat header and overflow menu opens a validated form (question, 2–12
  distinct options, 0 or 1 choice) with EN/PT strings. A successful creation
  reveals the new bubble; a failure raises a notice.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2601 passed, 44 ignored). Synthetic JIDs and fixtures only; no channel post,
poll, reaction or vote was sent to the paired account.

### 11. Wire/daemon parity for protocol 50 operations

**Problem.** The new blocklist, username, event, community and channel requests
existed only on the GUI's IPC path. The CLI speaks `oxidezap-wire`, whose own
request enum did not carry them, so a terminal had no way to ask for the same
operations.

**Change.**
- `oxidezap-wire` gains the 14 requests and six typed answers, with wire-owned
  DTOs that mirror core/IPC field names and serde spellings. It still has no
  dependency on `oxidezap-core`: parity is the JSON contract, not Rust type
  identity, so the CLI dependency stays small.
- The daemon's new `session_bridge::wire_domain` converts each wire payload to
  the session's domain type (and results back) field by field. No JSON
  round-trip: a renamed field fails to compile instead of arriving as a
  default. Every request is routed out-of-band under its own request id and
  needs a live connection, including the two network reads.
- `local_id` on a channel post is preserved from the CLI too; a terminal simply
  sends `None` when it draws no optimistic bubble.

This commit adds the transport and daemon half. The CLI surface itself follows
in the next commit.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2610 passed, 44 ignored). Synthetic fixtures only; no account operation was
performed.

### 12. CLI surface for protocol 50 operations

**Problem.** The wire contract and daemon mapping made the operations possible
from a terminal, but no command reached them. A script still had to open a GUI
to block an address, answer an event, create a community or post in a channel.

**Change.**
- `contacts blocked|block|unblock|lookup` exposes the block list, both block
  mutations, and all four username lookup outcomes. Lookup's human output says
  when a `--key` retry is possible, rather than rendering `KeyRequired` as a
  generic failure.
- New `events create|respond` and `communities create|subgroup|link|join`
  families pass the same typed DTOs the GUI uses. Partial community answers
  print groups, links, failed groups and warnings; JSON mode emits the typed
  answer verbatim.
- `channels post|poll|react|vote` reaches the channel-specific session paths.
  A CLI post passes `local_id: None` because it draws no optimistic bubble;
  reaction removal is an empty `--emoji`. Polls need at least two options, and
  an event location needs both coordinates, both are refused before a socket
  is opened.
- The help-surface tests name every command and flag, so a rename cannot leave
  the documented CLI and the binary disagreeing.

No wire schema or IPC protocol changed: these are clients of the contract
landed in the previous section.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2611 passed, 44 ignored). Synthetic command lines only; no account operation
was performed.

### 13. Native call-link preview and join (protocol 51)

**Problem.** A reusable WhatsApp call link can be previewed and joined by the
library, but no front end could reach either operation. The group-call path
addresses people by JID; a call-link token is an opaque invite, and routing it
through the JID parser or a chat lookup would make it a recipient it is not.

**Change.**
- `core` adds `CallLinkMedia`, `CallLinkPreview`, and the fixed label/name pair
  for a link placement. The label is explicitly not a JID; the preview's token
  is link identity and is never logged or drawn.
- IPC is version **51**: `PreviewCallLink` is an out-of-band read answered by
  `DaemonMessage::CallLinkPreview`, and `CallAction::JoinLink` carries the
  pasted text and the front end's placeholder id. Session wrappers map the
  upstream preview metadata and join through the library's `CallLinkCall`
  builder; the token reaches only that builder, never `Jid`, `hub.chat()` or
  `is_group_jid`. A video link requires camera endpoints, no voice downgrade.
- The daemon mirrors a link join as an ordinary outgoing call under the fixed
  recipient with no group identity, and reuses the same busy/offline/device
  cleanup as `Start` and `StartGroup`. Group-call routing is unchanged.
- The GUI adds a modal reached from the chat header and overflow menu. It
  validates a full `https://call.whatsapp.com/{audio|video}/{token}` link or a
  bare token without parsing either as an address, rejects media mismatches,
  shows only the creator, media and waiting-room state, and keeps the join
  button locked until a preview has been shown and the consent box is ticked.
  EN/PT catalog entries carry the explicit live-group-call warning.
- Synthetic-only tests cover serde round-trips, token/JID rejection, preview
  mapping, link placeholder identity, daemon classification/mirroring, all
  validation forms and the preview-plus-consent gate. No real link was
  previewed or joined.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2623 passed, 44 ignored). Web gates green too: shared-view ban, daemon/session
wasm library tests, stable and nightly shared-memory WebCodecs/call-audio
browser tests, plus the `xtask` and example-plugin gates. Release rebuild:
`oxidezap` 47,868,336 B, `oxidezapd` 32,799,400 B, `oxidezap-cli`
1,706,472 B (<2 MiB).

### 14. Native message actions and account-wide search (protocol 52)

**Problem.** Forward and FTS5 search worked from the CLI but not the window.
Starred messages were a read-only list with no mutation, and `@mentions` could
only be attached from a terminal. The four gaps were split across store,
session, IPC, daemon, wire, CLI and GUI, so they landed as one slice rather
than seven partial ones.

**Change.**
- `core` gains `MessageSearchHit`; IPC moves to version **52** with
  `ForwardMessage`, `SetStarred`, `SearchMessages`, an additive `mentions`
  field on `SendText`, and the `SearchResults` / `StarChanged` events.
  `SearchMessages` is a local read; the other two are correlated mutations.
- The daemon relays forwards through the existing session mutation, routes
  mentions through the extended-text context used by the wire send, and
  persists the starred flag in the existing `messages.starred` column, no
  migration. A successful star is broadcast as `StarChanged`; its own ack or
  refusal is still correlated to the press. Search hits are bounded, sorted
  newest-first and carry their query so a front end can discard late answers.
- `oxidezap-wire` gains `SetStarred` without taking a dependency on `core`;
  CLI gains `messages star|unstar` and a protocol-52 parity test covers the
  JSON spelling.
- The GUI adds Forward and Star/Remove star to the bubble menu, a native chat
  picker for forwards, a bounded `@` candidate list built only from JIDs the
  window already knows, and a global search overlay on `secondary-shift-f`.
  The sidebar filter and in-conversation search are unchanged. Stars are a
  display overlay because `ChatMessage` carries no starred field; a window
  opened after a star change has no snapshot to draw until the next event.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2631 passed, 0 failed). The pre-existing sticker-animation timing test flakes
only under heavy parallel load; it passes in isolation and in the
single-threaded suite. Web gates green: shared-view ban, wasm daemon/session
browser tests (16 and 35 passed), and the nightly wasm type-check. Synthetic
fixtures only; no message, mention, star, forward or search ran against the
paired account.

### 15. Native saved messages and message pins (protocol 53)

**Problem.** A starred message could only be listed from a terminal, and a pin
did not exist in this tree at all. The library offers no "pin forever", so the
Web feature of that name cannot be reproduced exactly; the longest duration it
exposes is 30 days.

**Change.**
- IPC moves to version **53** with `SetMessagePinned` (a correlated mutation),
  `ListStarredMessages` (a local read) and the account-wide `PinChanged` /
  `StarredResults` events. `StarredResults` reuses the search-hit shape: a
  starred list and a search answer are the same "rows to jump to" result.
- A chat-store migration adds `messages.pinned` with a false default, plus
  bounded `pinned_messages` / `set_pinned` queries. The session mutation mirrors
  `set_starred`'s group addressing, calls the library's `pin_message` /
  `unpin_message` with `PinDuration::Days30`, and commits the local flag only
  after the network accepts it, a refused pin leaves no stale mark.
- The daemon routes both frames out of band, signals `PinChanged` only on a
  successful mutation, and answers the listing with `StarredResults` plus the
  request's own ack. `oxidezap-wire` gains `SetMessagePinned` without depending
  on `core`; CLI gains `messages pin|unpin`. Protocol-53 parity, IPC serde and
  CLI-surface tests are synthetic.
- The GUI adds a saved panel on `secondary-shift-s`, a pinned panel on
  `secondary-shift-p`, and Pin/Unpin plus a pinned marker on the bubble. Both
  panels reuse the global search's hit rows and close when a hit is opened.
  The saved panel is filled by a store-wide listing with no request id, so the
  newest answer replaces its rows. There is no `ListPinnedMessages` IPC: the
  pinned panel is derived from `PinChanged` events this window has seen and
  the messages it has loaded, so older pins or unloaded rows are absent.
  EN/PT catalog entries carry the 30-day wording, nothing promises "forever".

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2641 passed, 44 ignored, 0 failed). Web gates green: shared-view ban, wasm
daemon/session browser tests (16 and 35 passed), and the nightly wasm GUI and
session type-checks. CLI release 1,682,904 B (<2 MiB). Synthetic fixtures only;
no pin, star or listing ran against the paired account.

### 16. Native chat folders and list management (protocol 54)

**Problem.** The sidebar had four filters but no way to manage the list itself:
pinning, muting, archiving and marking a chat unread existed only in the
terminal. Favorites has no public setter upstream, and labels/custom lists are
a larger data-model slice, so neither is faked with a dead control.

**Change.**
- IPC moves to version **54** with four correlated mutations, `MarkUnread`,
  `PinChat`, `MuteChat`, `ArchiveChat`, whose JSON shapes mirror the terminal's
  wire requests exactly. No new events: the store already applies the library's
  `PinUpdate`, `MuteUpdate`, `ArchiveUpdate` and `MarkChatAsReadUpdate`.
- The daemon routes each mutation out of band to a focused `Action` and calls
  the existing session wrapper. The correlated reply only says whether this
  window's optimistic change stood; the authoritative row reaches every front
  end through the store, so nothing is signalled twice.
- The GUI adds a Pinned filter chip (the chip row wraps on narrow sidebars) and
  a right-click menu on chat rows for all four operations. A small per-chat
  overlay holds only those four flags while a request is in flight; it is
  rolled back per field on a refusal and retired when a store-backed row
  agrees. Bounded mute durations are in the protocol but not exposed, the menu
  offers indefinite mute and unmute, which is what the row can name honestly.
- No store migration: `Chat` already carries `pinned_at`, `muted_until`,
  `archived` and the unread flags. All new tests use synthetic JIDs.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2649 passed, 44 ignored, 0 failed). Web gates green: shared-view ban, wasm
daemon/session browser tests (16 and 35 passed), and the nightly wasm GUI and
session type-checks. CLI release unchanged at 1,682,904 B (<2 MiB). No real
chat state was changed on the paired account.

### 17. Shared group history keeps its own meaning (protocol 55)

**Problem.** Receiving a deliberately shared group history was already complete
in the store and session (see `shared-group-history.md`), but the client side
erased what it was: hydration relabelled the notice as a generic group change,
the window showed the store's hardcoded English sentence in every locale, and
the legacy wire DTO called it a call.

**Change.**
- IPC moves to version **55**. `SystemNotice::History` carries the notice's own
  message count and receiver count inside the existing session event, no new
  IPC frame. Hydration reads them from the stored notice proto; rows written
  before metadata was projected keep their sentence in a `text` field and show
  it verbatim. The store's text projection is unchanged for previews and FTS.
- The GUI gives the notice its own non-interactive system row with a clock
  glyph, and the same localized EN/PT sentence is used in the chat-list preview.
  Portuguese agrees both the message and the member with their own count, so
  the sentence has four plurality forms plus a numberless fallback. There is no
  "view history" control: the imported messages are already ordinary timeline
  rows and the bundle envelope is consumed, so no distinct batch remains.
- The legacy wire DTO now classifies notice kinds explicitly: only real call
  records are `"call"`; history, group changes and membership are `"system"`.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2656 passed, 44 ignored, 0 failed). Web gates green: shared-view ban, wasm
daemon/session browser tests (16 and 35 passed), and the nightly wasm GUI and
session type-checks. xtask (52 passed) and the plugin-host ignored test pass;
CLI release unchanged at 1,682,904 B (<2 MiB). No real chat state was changed.

### 18. My own group member label (protocol 56)

**Problem.** The group roster, roles, topic, description and permissions were
already available in the native client, but the one group mutation the upstream
library exposes for a member label had no front-end surface. Arbitrary
per-member tags are not actually possible: upstream's mutation takes a group
JID and a label but no target member, and `participant_label` is read-only
server metadata with no live change action. Group status has classification and
history support but no public sender. Those remain documented gaps, not dead UI.

**Change.**
- IPC moves to version **56** with `SetGroupMemberLabel { group_jid, label }`, a
  correlated mutation where the empty string clears. It keeps the label
  verbatim, including `""`; the GUI trims whitespace before deciding that a
  box means clear. No new event: upstream has no label-change push, so other
  front ends see a label only when they refresh group metadata.
- The session wrapper parses the JID and calls upstream
  `Groups::update_member_label`, which sends an end-to-end encrypted protocol
  message and rejects non-group addresses itself. The daemon routes the frame
  out of band, requires a request id, and keeps it in the network branch.
- The GUI adds "My group label…" to the group overflow menu, gated on the group
  JID and the connection. Its dialog has a 64-character local bound, rejects
  newlines, offers explicit **Save label** / **Clear label** buttons, and
  correlates the answer so a cancelled form's reply cannot raise a notice over
  its replacement. EN/PT strings cover the whole flow. No member picker was
  added, because the API has no target.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2665 passed, 44 ignored, 0 failed). Web gates green: shared-view ban, wasm
daemon/session browser tests (16 and 35 passed), and the nightly wasm GUI and
session type-checks. xtask (52 passed) and the plugin-host ignored test pass;
CLI release unchanged at 1,682,904 B (<2 MiB). No real group label was changed.

### 19. Text-status styling in the native composer (protocol 57)

**Problem.** The composer, audience selection, feed and viewer were already
native, but a text status was always sent with the same hardcoded green
background and system font. Upstream's `Status::send_text` accepts both a
background and a typed font; the client never surfaced either. The wire/CLI
status path has no style field and keeps its original look.

**Change.**
- IPC moves to version **57** with an optional `style` on `PublishStatus`.
  `StatusFont` is a written enum (eight protocol values, not integers,
  upstream silently drops unknown numbers at encode time) and `StatusStyle`
  carries the `0xAARRGGBB` background. Absence still means the legacy
  green/system post; the GUI always sends an explicit style.
- The style type lives in `core`, so the session's font mapping and both front
  ends use one vocabulary. The daemon forwards the field verbatim through an
  addressed, out-of-band mutation; the wire path passes `None` explicitly.
- The GUI offers six named backgrounds (including the legacy green) and all
  eight fonts as compact dropdowns in the text composer only. Selecting media
  hides them, and a pending post ignores late changes. The swatch colour is
  recipient-visible content metadata rather than theme chrome; the picker
  itself still takes its colours from `cx.theme()`. EN/PT names every option.
- No dead controls for upstream-blocked features: story-audience mutation has
  no public setter, and read receipts, group-status posting, camera capture,
  TTL/view-once and status mentions have no public path. Those stay documented
  gaps. Local watched marking remains local, not a sent receipt.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2675 passed, 44 ignored, 0 failed). Web gates green: shared-view ban, wasm
daemon/session browser tests (16 and 35 passed), and the nightly wasm GUI and
session type-checks. xtask (52 passed) and the plugin-host ignored test pass;
CLI release unchanged at 1,682,904 B (<2 MiB). No status was posted.

### 20. Unlinking community subgroups (protocol 58)

**Problem.** The community dialog could create and link subgroups, but not
detach one. Upstream exposes `Community::unlink_subgroups` publicly and reports
per-group failures; nothing in the client used it. The rest of the advertised
"channel admin" surface is not honest to draw: channel roles and admin metadata
are dropped at every boundary in this stack, pin/reaction-settings/admin-invite
operations exist only as raw MEX IQ types, and pinned posts are not parsed or
fetched. Those stay documented gaps rather than dead buttons.

**Change.**
- IPC moves to version **58** with one addressed request,
  `UnlinkCommunitySubgroups { community_jid, groups, remove_orphan_members }`.
  The destructive orphan-member flag defaults to false on the wire and has no
  GUI checkbox: turning it on removes members left without a group.
- The result uses the existing `CommunityResult` frame and the GUI's existing
  correlated community answer path. `CommunityOperationResult` gains
  `unlinked_jids`, defaulted so older frames still decode; the frame is now
  boxed because a fifth result vector pushed it past the shared size budget.
  Partial success is reported as linked/unlinked counts plus one notice per
  refused group, not collapsed into an acknowledgement.
- The session wrapper validates `@g.us` addresses and calls upstream
  `unlink_subgroups`. The daemon requires a request id and dispatches the
  mutation out of band, like link/join. The wire mapping deliberately does not
  copy `unlinked_jids`: no wire command can unlink in this slice.
- The community dialog gains an **Unlink groups** mode beside Link. It reuses
  the community and group boxes, relabels the group box, keeps the two modes'
  text separate, and reports what was detached. EN/PT strings cover the whole
  flow. No channel settings, post pinning or role controls were invented.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2679 passed, 44 ignored, 0 failed). Web gates green: shared-view ban, wasm
daemon/session browser tests (16 and 35 passed), and the nightly wasm GUI and
session type-checks. xtask (52 passed) and the plugin-host ignored test pass;
CLI release unchanged at 1,682,904 B (<2 MiB). No community or group was
changed; all fixtures are synthetic `@g.us` addresses.

### 21. Keyboard shortcut map (client only, no protocol bump)

**Problem.** The client installs a real keyboard map, `Ctrl/⌘ K`, `Shift F`,
`Enter`, page keys and the rest, but nothing in the UI listed it. Users had to
discover every binding by trying it, and actions with no binding were
indistinguishable from missing features.

**Change.** A new **Keyboard shortcuts** section in Settings draws the bindings
the binary actually installs, grouped by scope (Everywhere, Chat list,
Conversation, Calls, Media viewer) and followed by an explicit **No key
assigned** group naming the actions that stay in a menu. `Ctrl/⌘ /` opens
Settings straight onto that section; a synthetic GPUI test proves the chord
lands there and the panel renders. No protocol, IPC, daemon or session code
changed: a help table is not a wire frame, and bumping the version would have
broken handshakes for no reason. EN/PT strings cover every heading and note.

**Gates.** fmt and GUI clippy (`-D warnings`) green; the shortcut chord test
passes. Full gates and the commit follow in the central review.

### 22. Stored media messages jump list (protocol 59)

**Problem.** Storage totals and cache clearing already worked, but there was no
way to get back to a picture, video or document once it had scrolled out of a
chat. A full media tab is not honest in this stack: upstream exposes no gallery,
cache accounting or thumbnail API, and the local search hit carries only a
boolean `has_media`.

**Change.** Protocol **59** adds `ListMediaMessages { limit }`, answered by a
dedicated `MediaResults` frame so it cannot be mistaken for a query-correlated
search answer or replace the saved panel's rows. It is a local read on both
sides: the daemon routes it out of band without needing a connection, and the
session projects `pending_media_messages(None, bound)` into the existing
`MessageSearchHit` shape (50 default, clamp 1–100, newest first, `has_media`
set). Storage & media gains a short guidance note and an **Open media messages**
button; the resulting overlay reuses the saved/pinned hit rows and jumps to the
chosen message, closing the other panels. No `Destination::Media`, thumbnails,
kind filters, cache-state claims or wire/CLI commands were added. Older peers
cannot attach across the new version. Wave 1.4 keeps the same frame and version;
it grows the hit with optional media facts and turns the panel into a gallery,
see section 29.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2686 passed, 44 ignored, 0 failed). Doctests, CLI 1,682,904 B (<2 MiB),
wasm daemon/session tests 16+35, nightly wasm GUI/session checks, xtask 52,
plugin-host ignored test and the shared-view ban all pass. All rows and
addresses in tests are synthetic.

### 23. WhatsNative branding and fork identity

**Problem.** The tree still carried upstream's product name, desktop/app ids,
repository URLs and package maintainer. A public fork under `duckb1t` must not
present itself as `oxidezap/client`, and a second installed app must not fight
over the same Wayland app id or icon.

**Change.** The display name is now **WhatsNative** in the window, menus, macOS
tray, CLI, Settings → About, package metadata and EN/PT strings. The repo URL is
`https://github.com/duckb1t/whatsnative` in `Cargo.toml`, AppStream, nfpm and
the README. Desktop id is `org.whatsnative.client.local` (the `.local` suffix is
kept deliberately: the project owns no DNS domain); AppStream/Flatpak id is
`io.github.duckb1t.whatsnative`, reverse-DNS on the GitHub owner. The master
icon, seven hicolor PNGs, desktop file, metainfo template and Flatpak manifest
were renamed to match, and every packaging script, smoke test, release checker
and workflow was updated with them. The deb/rpm maintainer defaults to
`DuckB1t <DuckB1t@users.noreply.github.com>`.

Crate names and binary names stay `oxidezap-*` / `oxidezap*`, this is a fork of
upstream's code, and renaming 400+ dependency references would buy nothing.
Upstream authorship is preserved in `LICENSE`, `Cargo.toml` authors, the
README's credits, and `packaging/ICONS.md` (the icon artwork is inherited).
`AGENTS.md` still says "oxidezap" at the top for the same reason: it describes
the inherited architecture, not the product name.

**Gates.** Shell syntax, `apprun-test.sh`, `apprun-cli-test.sh`,
`verified-download-test.sh`, `desktop-file-validate`, `appstreamcli validate`,
workflow YAML and `git diff --check` all pass, alongside the full workspace
suite below.

### 24. Account-level DM read receipts (protocol 60)

**Problem.** Privacy and security had no honest control: one upstream setting
is both readable and settable through public APIs, but nothing exposed it, and
drawing the other categories would be a dead surface.

**Change.** Protocol **60** adds `FetchPrivacySettings`, `SetReadReceipts`,
`PrivacySettings` (account-wide, no id) and `PrivacyResult` (correlated). The
core DTO has `All`, `None` and `Unknown`: an absent or unrecognised wire value is
never drawn as "off". The session calls upstream's public
`fetch_privacy_settings` / `set_privacy_setting` with
`PrivacyCategory::ReadReceipts`. Because a SET IQ answers with only a `dhash`
and no push event follows, the session applies `SetReadReceiptsDisabled` to the
device snapshot, flushes, then re-reads the server and refuses to claim success
unless the fresh value matches what was sent. The daemon publishes that fresh
value before the correlated result; the window does not flip the switch on
click. Settings → Privacy gains a "DM read receipts" group with a real switch
only for a known value, explicit loading/unavailable states, EN/PT strings and
failure notices; the value resets on account forget/switch. No wire or CLI
parity, no other privacy category, no live-account exercise. Older peers cannot
attach across the new version.

**Gates.** fmt, clippy (`-D warnings`) and the full workspace suite green
(2698 passed, 44 ignored, 0 failed). Doctests, CLI 1,682,904 B (<2 MiB), wasm
daemon/session tests 16+35, nightly wasm GUI/session checks, xtask 52,
plugin-host ignored tests and the shared-view ban all pass. All ids, values and
addresses in tests are synthetic.

### 25. Own logo, loud README, honest renders and release artifacts

**Problem.** The inherited icon was basically a WhatsApp bubble with the corners
filed off. The README was polite to the point of useless, the screenshots were
a washed-out light theme, and the repo description still pointed at upstream's
homepage. Nobody is fooled by that stuff.

**Change.** Replaced the artwork with `packaging/whatsnative-logo.svg`, a
hand-written vector mark: an angular W cut from cyan and violet shards on an
indigo tile, no bubble, no handset, no WhatsApp green. `packaging/ICONS.md`
records the provenance, and `packaging/generate-icons.py` plus
`packaging/linux/icons.sh` regenerated the master PNG, the web favicons, the
Apple touch icon and all seven hicolor sizes from the same source. The window
embeds the new master, so the GUI binary had to be rebuilt.

The README now leads with "FUCK ELECTRON. FUCK WEBVIEW.", says plainly what
works and what does not, and carries a vector hero rendered to PNG for reliable
GitHub display. The three screenshots are real headless GPUI renders in the
shipped Tokyo Night theme, with invented names, JIDs and messages. The
`#[ignore]`d generator in `crates/gui/src/app/mod.rs` installs the default theme
instead of reading anyone's local `theme.json`, and `AccountSelection::load()`
is defaulted under `cfg!(test)`, so no logged-in account is read. Every markdown
file and the EN/PT catalog strings were scrubbed of em dashes.

Repository description, homepage, topics and Issues were set on
`DuckB1t/whatsnative`. The `v0.1.0` release carries source, an x86_64 AppImage
and a Linux tarball, updated when the icon changed.

**Gates.** SVG parses and renders, every local README link resolves, the PNGs
were checked with image statistics and OCR for real rendered content, and no
sensitive pattern appears in any image. fmt, clippy `-D warnings` and the GUI
suite stay green. The AppImage is built with `packaging/linux/package.sh` at the
pinned tool versions, and the tarball follows `.github/workflows/build.yml`.

### 26. Calls view for the daemon's live log (protocol 61)

**Problem.** The daemon already kept a bounded ring of finalized calls and the
CLI could print it, but the window had no Calls destination. Calls only showed
up as system rows inside conversations, so there was no way to see what this
daemon had observed without leaving the GUI.

**Change.** Protocol **61** adds `ClientRequest::ListCalls` and
`DaemonMessage::Calls`. The answer is addressed under the request id and goes
only to the front end that asked; the wire and CLI `calls` request is untouched.
The daemon ring now lives on `StateHub`, with the session bridge as its only
writer, so a GUI read does not need a session command. The window asks when
Calls opens, when a fresh attachment arrives, on explicit refresh, and when a
call state changes while the pane is visible.

A fifth rail destination uses the Phone icon and opens a dedicated Calls pane.
Rows show an incoming or outgoing arrow, a voice or video icon, a resolved peer
or group name with a synthetic-safe JID fallback, outcome, duration and time.
A row opens its chat when that chat is loaded; when it is not, the row stays a
label instead of pretending to navigate. All and Missed filters, honest
loading, empty and error states, and EN/PT strings are included.

**Limitation.** This is not durable history. The daemon keeps finalized calls
from its own lifetime only, capped at 100, and a restart empties the log. The
pane states this exactly: "Calls since this daemon started. Last 100 only."
No server history is fetched and nothing is written to disk. No paired-device
exercise was performed; every fixture uses `.invalid` addresses.

**Gates.** `cargo +1.100.0 fmt --all`,
`cargo +1.100.0 clippy --workspace --all-targets --all-features -- -D warnings`
and `cargo +1.100.0 test --workspace --all-features -- --test-threads=1` are
green (2705 passed, 45 ignored, 0 failed). The coordinator reran those three
plus the xtask format, clippy and test gates with the same result.
`git diff --check` is clean. No em dashes or real identifiers are present in
the new lines.

### 27. Chat info sheet from state the window already owns (client only)

**Problem.** The conversation header carried an avatar and a name but offered
no route to details. There was no contact or group info view: no raw JID, no
member list, no honest media total and no place to see or change mute and
archive without leaving the chat. Drawing a profile card would have invented
About text, last seen, phone labels or privacy facts this stack never receives.

**Change.** The header identity is now an accessible button, and a labelled info
action sits in the toolbar and the overflow menu. A chat row's context menu also
offers Contact info or Group info. Opening the sheet selects that chat first,
asks for the group roster through the existing `GroupMembers` request and keeps
only a JID in app state. Selecting a different chat, clearing the account or
losing the chat closes the sheet. On desktop it is a right-side sheet inside the
chat area, so the list and timeline stay usable; on a phone it owns the chat
area and has a labelled Back control.

The sheet reuses `Avatar`, `chat_row::display_name` and the header's
`members_line` roster formatting. Group size comes only from `GroupRoster`,
never from `Chat::participants`, which records senders the window has seen. A
roster that is still in flight says the connection has not answered; a group
with no answer says the list is unavailable. Neither claims zero members. Media
is a count of messages in the loaded timeline only and is labelled that way.
Mute and archive rows call `set_chat_muted` and `set_chat_archived`. No new IPC
request was added and the Calls protocol was not touched. Review kept the
timeline build inside the Chats branch, so Calls and Status frames do not pay
for a hidden conversation, and the desktop Close button has an accessible name.

**Limitation.** This is not server-backed profile data. There is no About, last
seen, phone label, contact photo beyond the existing media cache, privacy fact
or whole-store media total. The roster is whatever the connection last told
this window, and the media count moves as history loads. Nothing here was
exercised against a paired account; fixtures use `.invalid` addresses only.

**Gates.** `cargo +1.100.0 fmt --all -- --check`,
`cargo +1.100.0 clippy --workspace --all-targets --all-features -- -D warnings`
and `cargo +1.100.0 test --workspace --all-features -- --test-threads=1` are
green (2717 passed, 45 ignored, 0 failed). The xtask format, clippy and test
gates are green (52 passed). The example plugin workspaces and the plugin host's
ignored wasm-loading tests are green too. `git diff --check` is clean. The
no-account-leak audit on the full publication range found no em dashes,
phone-shaped fixtures, real JID namespaces, account paths or secrets. No real
identifiers are present in the new lines.

### 28. New chat dialog from loaded chats and username lookup (client only)

**Problem.** The Chats sidebar had a `+` button that was disabled with a
tooltip saying starting a conversation was unavailable. The only username
lookup lived in Settings and described an account; it did not offer a way to
reach a conversation. The GUI protocol has no create-chat-by-JID request, so a
dialog that claimed to start any address would have invented a capability.

**Change.** The Chats `+` now opens a dedicated modal. Communities keep their
existing creation form, and the other destinations keep the disabled button.
The dialog lists only conversations already in `WhatsAppApp.chats`, filtered
case-insensitively by `chat_row::display_name` and the raw JID. A row calls
`has_chat` before `select_chat(..., ChatOpen::ToCompose, ...)` and closes the
dialog. No row is fabricated for an address the window does not hold.

The second section reuses `ClientRequest::LookupUsername` and
`normalize_username`. It keeps its own query, username, key, result, loading and
error state rather than borrowing the Settings fields. Found, KeyRequired,
NotFound, InvalidUsername, loading and lookup failure each have their own
sentence. A Found answer shows the JID and only the phone JID, username, display
name and verified name the protocol supplied. It gets an Open chat action only
when `has_chat` matches that address; otherwise it stays protocol data with a
plain explanation and no send affordance.

The dialog has its own `KeyboardOwner` and `KeyboardSurfaces` entry, so one
Escape closes this surface and leaves a chat info sheet or anything underneath
alone. Closing it, clearing the account or losing the session drops the dialog
and marks any outstanding lookup answer as unwanted, so a late reply cannot
appear in Settings or the next account.

**Limitation.** This is not a contact directory and it cannot create a chat.
There is no address-book request, no create-chat mutation, no server-backed
profile fetch and no protocol version change. A username that resolves to an
address this window has not loaded can be read but not opened here. Nothing was
exercised against a paired account; the tests use invented `.invalid` addresses
and no real lookup, message, pairing or account change.

**Gates.** `cargo +1.100.0 fmt --all -- --check`,
`cargo +1.100.0 clippy --workspace --all-targets --all-features -- -D warnings`,
`cargo +1.100.0 nextest run --workspace --all-features`, and
`cargo +1.100.0 test --workspace --all-features --doc` are green
(2727 passed, 35 skipped in nextest; 767 passed, 15 skipped in oxidezap-gui
nextest). The xtask fmt, clippy and test gates are green (52 passed).
`git diff --check` is clean. The libtest full GUI single-command run aborted in
this environment with a `gpui-pre-scheduler` async-io destructor panic
(`SIGABRT`) before a final summary; CI uses nextest, which passed here. No em
dashes or real identifiers are present in the new lines.

### 29. Local media gallery from stored message facts (Wave 1.4)

**Problem.** Protocol 59 gave Storage & media a bounded jump list, and that was
it. The row carried only `has_media`, so the overlay could not say what kind of
file it was, could not filter, and could not draw a preview without lying about
where the bytes came from. A full cloud gallery is still not possible here:
upstream exposes no gallery, cache accounting or thumbnail API.

**Change.**
- `MessageSearchHit` gains seven optional, serde-defaulted media facts: kind,
  file name, MIME type, width, height, stored-preview-byte presence and
  download-metadata presence. The first six say what the stored proto proves.
  The last two are deliberately presence flags, not a promise that a decode or
  a download will work. Old JSON still decodes and re-serializes to the old
  line; no new frame or protocol bump was needed.
- `search_hit_projection` is now the one place that turns a stored row into a
  hit. It reads media from the stored proto only when the row is not revoked and
  its content class carries media. A missing or undecodable proto keeps the old
  shape instead of guessing a kind from a media flag.
- Storage & media now opens a gallery over the same 50-row listing. Cards show
  the stored facts, group by chat, and offer chips for all, images, videos,
  audio, documents and stickers. The chips count and hide only rows this window
  already has. A new metric pair sizes the cards and thumbnails, and the EN/PT
  strings say the gallery is stored history with no server-wide total.
- A thumbnail is collected from messages already loaded in this window and is
  decoded only when the bytes are a still the local decoder understands.
  Anything else gets a kind card. The overlay now blocks clicks behind it, so
  opening a card cannot also select a chat under the scrim.
- Synthetic tests cover the projection's real image facts, the old frame shape,
  filter counts, chat grouping, a real local PNG thumbnail and click-through to
  the stored message. The row keeps its `media-messages-hit` debug selector.

**Not changed.** No server request, no media bytes over IPC, no bulk retry, no
downloaded-only filter, no account-wide total and no new destination. Nothing
was run against a paired account. Every address in the new fixtures is
`.invalid`, and the synthetic image is a 1x1 PNG made in the test.

**Gates.** `cargo +1.100.0 fmt --all -- --check`,
`cargo +1.100.0 clippy --workspace --all-targets --all-features -- -D warnings`,
`cargo +1.100.0 nextest run --workspace --all-features` (2728 passed, 35
skipped), `cargo +1.100.0 nextest run -p oxidezap-gui --all-features` (768
passed, 15 skipped) and `cargo +1.100.0 test --workspace --all-features --doc`
are green. The xtask fmt, clippy and test gates are green (52 passed).
`git diff --check` is clean. The full libtest GUI command still aborts in this
environment with the `gpui-pre-scheduler` destructor panic, so the CI-aligned
nextest result is the one recorded here. The publication audit found no em
dashes, phone-shaped fixtures, real JID namespaces, account paths or secrets in
the new lines.

## Planned (see plan file)

1. ~~Group call plumbing and grid~~ ✅ (above).
2. Resolve the group's display name for the ringing card (the offer only has
   the caller's push name).
3. Keyring/secret-service option for identity keys (keep one-file store default).
4. ~~rem-scale audit~~ ✅ Audited 116 bare `px()` calls in GUI components/views:
   all design sizes already use `metrics.*()`. The remaining ones are decoded
   media geometry, scale-invariant zero/hairline thresholds, or test fixtures,
   no changes needed. GUI tests: 680 passed, 14 ignored.
5. ~~Branding / app id / desktop file rename~~ ✅ (section 23); crate names kept
   to minimize diff.
6. ~~Packaging smoke tests~~ ✅ (above); audit and benchmarks still open.
