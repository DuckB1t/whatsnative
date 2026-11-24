# Feature plan, plain talk

> This is the working plan for closing the gaps a normal user actually notices.
> It is not a promise to fake features Meta never exposed. Every entry says what
> we can build, what we cannot, and why. Nothing here is delivered until it has
> synthetic tests, green gates and a commit that can be reverted.

## Decisions already made

- Product name is WhatsNative. Crate and binary names stay `oxidezap*`.
- **UI target:** familiar layout, original design. The same controls go in the
  same places a person expects, but the colours, icons, spacing and artwork are
  ours. We are not copying Meta's proprietary assets or trade dress pixel by
  pixel. The user chose this explicitly on 2026-11-22.
- Validation is synthetic only. No real messages, calls, posts, votes,
  reactions, pairing, account changes or personal data. No logged-in account is
  read by a test or a screenshot generator.
- Agents implement. The coordinator reviews every diff, runs the gates, updates
  docs and commits centrally. Nothing is pushed without an explicit request.
  The user authorized pushing after each completed wave on 2026-11-22.

## Current status, updated after each wave

- [x] Wave 0, investigation and plan
- [x] Wave 1.1, dedicated Calls destination and daemon call log
- [x] Wave 1.2, chat info panel
- [x] Wave 1.3, new chat dialog
- [x] Wave 1.4, media gallery
- [x] Wave 1.5, local GIF and sticker search
- [x] Wave 2, message actions with real APIs
- [x] Wave 3, privacy and settings with real mutations
- [ ] Wave 4, multi-account V2
- [x] MCP planning

Waves 1.1 through 1.5 are implemented, reviewed, green and cleared for push
after their waves. Wave 2 is also implemented, reviewed and green: four slices,
one protocol bump, one push at the end. None of it was in v0.1.0. v0.2.0 carries it. The call log is a daemon-lifetime
ring capped at 100, and the pane says that plainly. Chat info is a mirror of
local state: identity, authoritative roster, loaded-history media count and
working mute and archive controls. It does not invent profile facts. The new
chat dialog filters loaded chats and uses the existing username lookup; it is
not a contact directory. The media gallery is a bounded local view over the
last 50 stored media rows. It shows real kind, filename, dimension and
availability facts from the stored proto, groups what it has by chat and filters
only those rows. A thumbnail is drawn only from bytes this window already holds
and can decode. No server total, no downloaded-only claim and no fake fetch
hides behind the chips. The picker search filters only the tiles already loaded:
GIFs by chat name, caption and file name, recent stickers by the chat they came
from. Favorites carry no text and pack metadata never reaches this window, so
they are honestly not searchable rather than matched on invented words. It is a
local hide, not a fetch, and the query dies when the picker closes. The
coordinator reran format, clippy, the full nextest workspace suite (2732 passed,
35 skipped), the GUI nextest suite (772 passed, 15 skipped), doctests and the
xtask gates (52 passed), then checked the publication range for account leaks
before pushing. One pre-existing daemon launch test reports as leaky under
nextest; it is unrelated to these waves. The old full libtest GUI command still
aborts in this environment with a GPUI scheduler destructor panic, so CI's
nextest run is the honest gate here.

Wave 3 is also implemented, reviewed and green. It adds one protocol bump to 63
and seven real account controls in the Privacy pane: the old DM read receipts
plus last seen, online, profile photo, About, group add and who can message me.
Every one of the six new controls uses the pinned upstream public setter,
re-reads the server before it reports success and never flips a local value on a
click. Unsupported server values stay as facts the window refuses to guess
about. The full change is in section 33 of `docs/our-changes.md`.

## What the deep investigation found

### Navigation and conversation surface

- The nav rail has exactly four destinations: Chats, Status, Channels,
  Communities (`crates/gui/src/components/nav_rail.rs:101-106`). There is no
  Calls destination, no Contacts destination and no profile destination.
- The sidebar `+` button is disabled outside Communities. The tooltip literally
  says "Starting a new conversation is not available yet"
  (`crates/gui/src/components/chat_list/mod.rs:188-211`,
  `crates/gui/src/i18n/catalog.rs:224,559`). Only the Communities form can create
  anything, and that is a community or a community subgroup.
- There is no chat, contact or group info view. The conversation header shows
  name, avatar and a status or member subtitle
  (`crates/gui/src/components/chat_header.rs:49-92,121-149,228-290`), but its
  overflow menu only offers conversation search, event, channel poll, voice
  call, video call, join call link and my group label. No View profile, no
  Contact info, no Group info, no About, no member roster, no shared media.
- Chat row context menus only mark unread, pin, mute and archive
  (`crates/gui/src/components/chat_item.rs:147-204`).
- Settings -> Contacts is a one-off username lookup, not an address book
  (`crates/gui/src/views/settings/panes.rs:439-544`).
- Other panels, search, saved, pinned, media and forward picker, are centered
  overlays. There is no right-side detail panel.

### Calls

- There is no dedicated Calls tab. Call history lives as per-conversation system
  rows, with a missed-call preview in the chat list
  (`crates/gui/src/components/message_bubble/system.rs:19-92`,
  `crates/gui/src/app/calls_ctl.rs:1373-1429`,
  `crates/gui/src/app/chat_row.rs:273-297`).
- The daemon has a structured `CallLog` and a CLI command, `oxidezap-cli calls`.
  The log is in memory only, finalized calls since daemon start, capped at 100
  (`crates/daemon/src/session_bridge/call_log.rs:1-7,18-20,188-202`).
  The GUI never asks for it. `ClientRequest::ListCalls` does not exist on the GUI
  protocol; the wire request at `crates/wire/src/request.rs:382-385` is the CLI
  path only. A GUI Calls view needs a new typed request and answer across ipc,
  daemon and gui. That is a real change, not a CSS trick.
- Group calls are drawn and routed but never live tested. 1:1 calls are the only
  validated ones. Screen share and add participant stay visibly disabled.

### GIFs, stickers, emoji, reactions

- A composer picker already exists with Emoji, Stickers, Favorites and GIFs tabs
  (`crates/gui/src/components/composer_picker.rs:11-27,221-332`). It is backed by
  local history and synced favorites, not a cloud catalog. There is no GIF
  search, no trending GIFs and no sticker search anywhere in code or catalog.
- Animated WebP stickers decode and play properly
  (`crates/gui/src/components/message_bubble/media.rs:465-489`,
  `crates/gui/src/components/sticker.rs:8-123`). GIF-flagged videos play through
  the video player. Raw animated GIF or APNG sent as a photo is refused; the
  sender is told to use Document
  (`crates/session/src/whatsapp/outgoing.rs:223-238`).
- Sticker pack messages are a text projection only. Opening or adding a pack is
  unsupported (`docs/channel-message-types.md:9-11`).
- Emoji search with skin tones exists (`crates/gui/src/components/emoji_picker.rs:45-213`).
- Reactions exist as a fixed six emoji quick strip and context menu
  (`crates/gui/src/app/timeline_ctl.rs:311-315`), with optimistic send and
  rollback. They are only synthetically tested, never sent to a live account.
- The attach menu has exactly Document and Photos and Videos
  (`crates/gui/src/components/input_area_view.rs:805-840`). Cropping, kind
  override, poster frames and downscaling are open gaps in `docs/roadmap.md`.

### Other honest gaps

- Media is a local jump list, not a gallery, with no thumbnails, filters or
  kind grouping (`docs/roadmap.md:646-653`).
- Account level privacy now has seven real controls: DM read receipts plus last
  seen, online, profile photo, About, group add and who can message me. Security
  notifications, 2FA, passkeys, chat lock, linked-device removal, disappearing
  defaults and the message-request inbox remain documented gaps, not fake
  switches (`docs/roadmap.md:362-378`).
- The GUI is fixed to `AccountId::LEGACY`. Multi-account V2 has its own tracker
  in `docs/multi-account-todo.md` and is not mixed into this plan.

## Feasibility buckets

### Buildable with what is in this tree

1. Dedicated Calls destination and call log view. Add a GUI protocol request,
   daemon handler and view. Show outcome, direction, video flag, duration,
   time and group. Tapping a row opens the related chat. The log stays honestly
   labeled as since daemon start, capped at 100.
2. Chat info panel. A right-side detail sheet built from known chat state:
   avatar, name, JID, group member roster from the existing members request,
   media count from known messages, mute and archived state. No invented
   profile fields. Clicking the header identity or a chat row opens it.
3. New chat dialog. Start with known chats plus the existing username lookup.
   If upstream has no create-chat-by-JID mutation, the dialog opens an existing
   chat or resolves a username before sending, and says so. No fake contact
   directory.
4. Real media gallery. Upgrade the jump list to a gallery with thumbnails,
   filters by image, video, audio and document, grouped by chat, reusing the
   existing media cache. Still local only.
5. Local GIF and sticker search. Filter the existing recent GIFs by chat name,
   caption and file name, and recent stickers by the chat they came from. No
   cloud claims. Favorites carry no text and pack metadata is not projected, so
   they stay unsearchable rather than matched on invented words.
6. Attachment editing improvements that need no new codec: kind override and
   downscaling with the image crate. Cropping stays open if the picker gives us
   no crop rectangle.
 7. Six server owned privacy visibility controls. Use the public upstream read
    and write IQ pair, re-read after every set and show unsupported values
    honestly. This shipped in Wave 3, protocol 63.

### Blocked by upstream or by missing platform pieces

- Cloud GIF trending and search, sticker pack open and add, channel role
  administration, status audience editing, chat lock, message-request inbox,
  passkeys, 2FA, security notifications, key transparency, disappearing defaults,
  cross-platform chat transfer, Meta AI, ads, business catalogs and channel
  subscriptions. Last seen, online, profile photo, About, group add and who can
  message me are no longer blocked; protocol 63 reaches them through public
  upstream setters.
- Animated GIF as a video message and video poster frames need an H.264 encoder.
  Nothing in this tree writes H.264.
- Group call live validation needs real calls, which this plan forbids.
- Anything requiring a real account mutation stays synthetic-only validated and
  is labeled as such.

### Explicitly out of scope

- Pixel-perfect copy of WhatsApp's proprietary interface or assets.
- Sending anything to a real account, pairing, voting, reacting or posting.
- Pushing without an explicit request.

## Waves and acceptance

### Wave 0, investigation and plan

This document plus roadmap and changelog updates. No product code.

### Wave 1, daily drivers

Calls destination, chat info panel, new chat dialog, media gallery, local GIF
and sticker search. Done when each surface is reachable from the app, has
synthetic fixtures and tests, uses `cx.theme()` tokens and rem sizing, and the
workspace gates are green.

### Wave 2, message actions

Pinned and starred hydration gaps, forwarding target discovery where upstream
allows, mention candidate completeness, full emoji reaction picker, attachment
kind override and downscaling. Done only for items with a real API.

Status: done in four slices, reviewed and gated, with one end-of-wave push and
then the v0.2.0 release. Slice A put durable star and pin flags on `ChatMessage` and made
the overlay tri-state. Slice B opened the full emoji picker and fixed mention
candidates. Slice C added the send kind override and local photo downscaling.
Slice D bumped protocol 62 for the store-backed pinned listing and local contact
search. The full story and limits are in section 31 of `docs/our-changes.md`.

Acceptance: each action reaches a real upstream or store API, uses synthetic
fixtures only, says on screen what it can and cannot do, and adds no server-wide
claim. Inbound `PinInChatMessage`, forever pins, cloud GIF search, sticker pack
open/add and a server directory stay out because this tree has no honest path
to them.

### Wave 3, privacy and settings

Wave 3 does not pretend every row in a messenger's privacy screen is reachable
from this tree. Protocol 63 covers the categories the pinned upstream can both
read with `fetch_privacy_settings` and set with the plain `set_privacy_setting`
IQ: last seen, online, profile photo, About text, group add and who can message
me. A successful set re-reads the server before the window updates, the same
rule protocol 60 already uses for DM read receipts. `contact_blacklist` is shown
as unsupported when the server reports it, because setting that value needs a
separate disallowed-list update this wave does not offer.

Security notifications, 2FA/PIN, passkeys and linked-device removal stay gaps:
there is no public setter in this dependency. Disappearing defaults and chat
lock also stay gaps for now, because a write-only control with no honest read
path is a decorative toggle with extra network traffic.

- Slice A: core DTOs and protocol 63 frames.
- Slice B: session adapter and daemon handlers.
- Slice C: settings pane controls, i18n and synthetic tests.
- Slice D: docs, full gates and one push.

Acceptance: every control shows a fresh server answer or says it is unknown,
every mutation reaches a real public API, no unsupported value is guessed,
and `docs/roadmap.md` is updated when a gap closes.

Status: shipped in Wave 3. Protocol 63 is the only bump. The controls exist, the
server value is the only source of truth, and the full gate run is in section 33
of `docs/our-changes.md`.

### Wave 4, accounts

Follow `docs/multi-account-todo.md` as a separate program. Do not fold it into
these waves.

### After Wave 4

The MCP plan is in `docs/mcp-plan.md`. It is a plan, not a shipped server.
Implementation needs an explicit go ahead and keeps the first tool set read only.

## Process rules for every wave

- Read the relevant document under `docs/` before changing the code it
  describes, as `AGENTS.md` requires.
- One agent per slice, coordinator reviews and commits. Slices that touch the
  same files are sequenced, not run in parallel.
- Synthetic fixtures only. No JID, phone number or message from a real account.
- `cargo +1.100.0 fmt --all -- --check`,
  `cargo +1.100.0 clippy --workspace --all-targets --all-features -- -D warnings`
  and the full workspace test suite must pass before a commit.
- Update `docs/our-changes.md` with the slice number, the problem, the change,
  the gates and the honest limitations.
- Repeat the no-account-leak audit before any public push.
