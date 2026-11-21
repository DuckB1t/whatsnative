# Our changes on top of upstream

Base: `oxidezap/client` @ `ea67a2aa41279232029b5872528ec394fb491e98` (2026-11-20),
tagged `base-upstream-ea67a2aa`.

Upstream remote: `https://github.com/oxidezap/client.git`
Our remote: `https://github.com/duckb1t/whatsapp-native.git`

This file exists so a monthly `git fetch upstream && git rebase upstream/main` stays
reviewable. Record every local change here with its commit and reason. Small diffs only:
upstream already owns the protocol stack, daemon/IPC, native audio/video, GPUI UI,
SQLite store, plugins and Linux packaging.

## Environment / baseline (no source changes)

- Rust 1.100.0 via rustup (system 1.99.1 too old; project pins 1.100.0).
- Native deps present: ALSA, X11, Wayland, xkbcommon, fontconfig, freetype, FFmpeg.
- `cargo check -p oxidezap-core` — passed.
- `cargo build --release --bin oxidezap --bin oxidezapd --bin oxidezap-cli` — passed (24m30s).
- Binaries: `oxidezap` 46M, `oxidezapd` 31M, `oxidezap-cli` 1.5M (stripped).
- `oxidezap-cli doctor`: daemon running, DB healthy, pairing QR issued.
- GUI verified as native Wayland window (`org.oxidezap.client.local`, `xwayland: 0`).

## Local changes

### 1. Group calls keep their group identity (`group_jid`)

**Problem.** `wacore::types::call::CallAction::Offer` carries `group_jid`, but
`crates/session/src/whatsapp/mod.rs` matched it with `..` and the resulting
`oxidezap_core::IncomingCall` had nowhere to put it. Downstream,
`daemon/session_bridge/call_log.rs` derived `is_group` from `caller_jid` — which
for a group offer is the *person* who started it — so a group call was logged
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
flag was needed for is now reachable — see change 3.

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
  only the JID — no group flag to lose on that path.
- Group JIDs now show the call buttons; both toolbar and overflow menu route
  to one entry point. `SessionHandle::start_group_call` sends `StartGroup`.
- `CallState::group_roster()` feeds the existing `active_group` grid. A
  participant with no resolved name is drawn by JID. Screen-share and
  add-participant stay visibly disabled with honest tooltips.

**Gates.** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
--all-features -- -D warnings`, `cargo test --workspace --all-features` — all
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
  Block/unblock are correlated `ask`s, not `tell`s — they mutate the real
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
  `KeyRequired`, `NotFound` or locally-invalid handle. These stay separate —
  "give me the key" is not "nobody has this handle". `normalize_username()`
  strips the display-only `@` and enforces the server's 3..=35 length locally.
- `session/whatsapp/username_lookup.rs` calls upstream, converts the answer to
  domain data, and adds only the local address-book name. The verified name is
  the protocol's own field; a handle never becomes a fake business name.
- IPC adds `ClientRequest::LookupUsername { username, username_key }` and a
  correlated `DaemonMessage::UsernameLookup { id, result }`; protocol version
  **47**. The key is runtime input only — never logged or hardcoded. Failures
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
errors are plain strings, so they read as permanent in `what_went_wrong` —
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

## Planned (see plan file)

1. ~~Group call plumbing and grid~~ ✅ (above).
2. Resolve the group's display name for the ringing card (the offer only has
   the caller's push name).
3. Keyring/secret-service option for identity keys (keep one-file store default).
4. ~~rem-scale audit~~ ✅ Audited 116 bare `px()` calls in GUI components/views:
   all design sizes already use `metrics.*()`. The remaining ones are decoded
   media geometry, scale-invariant zero/hairline thresholds, or test fixtures —
   no changes needed. GUI tests: 680 passed, 14 ignored.
5. Branding / app id / desktop file rename — keep crate names to minimize diff.
6. ~~Packaging smoke tests~~ ✅ (above); audit and benchmarks still open.
