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
- Exactly one real account privacy control exists: DM read receipts. Last seen,
  About visibility, group-add permissions, 2FA, passkeys, chat lock and
  disappearing defaults are documented gaps, not fake switches
  (`docs/roadmap.md:357-372`).
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
5. Local GIF and sticker search. Filter the existing recent items and favorites
   by message text, chat name or pack metadata. No cloud claims.
6. Attachment editing improvements that need no new codec: kind override and
   downscaling with the image crate. Cropping stays open if the picker gives us
   no crop rectangle.

### Blocked by upstream or by missing platform pieces

- Cloud GIF trending and search, sticker pack open and add, channel role
  administration, status audience editing, last seen and About visibility,
  group-add permissions, chat lock, message-request inbox, passkeys, 2FA,
  security notifications, key transparency, disappearing defaults, cross-platform
  chat transfer, Meta AI, ads, business catalogs and channel subscriptions.
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

### Wave 3, privacy and settings

Wire up any mutation that has a real public setter. Do not add decorative
toggles. Update `docs/roadmap.md` when a gap closes.

### Wave 4, accounts

Follow `docs/multi-account-todo.md` as a separate program. Do not fold it into
these waves.

### After Wave 4

Plan the MCP work. Nothing in this document pre-empts that design.

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
