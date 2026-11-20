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
answering a group call needs no client-side work. The drawn participant grid
is still unreachable — that needs participants exposed from the library's
call registry across session → daemon → GUI, not just a flag.

### 2. Linux package staging smoke test

CI checked that deb/rpm/AppImage files exist, but nothing checked the FHS tree
they are built from. `packaging/linux/smoke-test.sh` now verifies, with only a
POSIX shell: the three binaries co-located and executable in `usr/bin`, a
desktop entry and AppStream metadata whose ids agree and have no unsubstituted
`@VERSION@`/`@DATE@`, and the seven hicolor PNGs. Wired into `build.yml` after
`package.sh` on both Linux runners. `docs/linux-packaging.md` documents it and
marks the old glibc/version figures as measurements from specific builds.

## Planned (see plan file)

1. Group video: expose participants from the library registry and route the card
   to the drawn grid. Start group calls from an existing group via
   `voip().group_call_by_id()`. **In progress.**
2. Resolve the group's display name for the ringing card (the offer only has
   the caller's push name).
3. Keyring/secret-service option for identity keys (keep one-file store default).
4. rem-scale audit for absolute `px()` literals in GUI. **In progress.**
5. Branding / app id / desktop file rename — keep crate names to minimize diff.
6. ~~Packaging smoke tests~~ ✅ (above); audit and benchmarks still open.
