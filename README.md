# WhatsNative

A native WhatsApp client for Linux, macOS and Windows, written in Rust with
[GPUI](https://github.com/zed-industries/zed). No Electron, no WebView, no
website in a window: the interface is drawn by the GPU, and the same codebase
also builds a WebAssembly front end.

**Not affiliated with, endorsed by, or connected to WhatsApp or Meta.**

WhatsNative is a fork of [oxidezap/client](https://github.com/oxidezap/client).
It keeps upstream's crate names and binary names (`oxidezap`, `oxidezapd`,
`oxidezap-cli`) so the dependency graph and packaging stay intact; the product
is branded WhatsNative. All protocol, Signal, media and call machinery comes
from [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust). See
[Credits](#credits).

## Status

Working: pairing, chats with durable local history, media send and receive,
1:1 voice and video calls, communities, channels, statuses, global and
in-conversation search, saved and pinned messages, block list, calendar events,
call-link previews, keyboard shortcuts, plugins, and a browser front end.
Group calls are implemented and draw a participant grid but have not been
validated against a real call. See [Known limitations](#known-limitations) and
[docs/roadmap.md](docs/roadmap.md) for the rest.

## Architecture

One WhatsApp session per user lives in a background daemon, `oxidezapd`. It
owns the connection, the SQLite store and the media cache, and serves front
ends over a per-user local transport (a Unix socket, or a named pipe on
Windows). `oxidezap` is the GPUI desktop window; it holds no session and starts
the daemon if none is running. `oxidezap-cli` is a scriptable front end onto
the same daemon. Any number of windows can attach to one session.

| Crate | Role |
| --- | --- |
| `oxidezap-core` | Domain types and the daemon's wire format |
| `oxidezap-chat-store` | SQLite history with FTS5 full-text search |
| `oxidezap-session` | Connection, event stream, sends, store hydration |
| `oxidezap-daemon` | `oxidezapd`: session, socket, tray, plugins |
| `oxidezap-ipc` | Protocol and transport shared by every front end |
| `oxidezap-gui` | `oxidezap`: GPUI window (also builds to wasm) |
| `oxidezap-cli` | `oxidezap-cli`: scriptable front end |
| `oxidezap-wire` | Lightweight DTOs for external scripts |
| `oxidezap-audio` / `oxidezap-video` | Capture, playback, codecs |
| `oxidezap-plugin-*` | wasm ABI, host and Rust SDK |

Details: [docs/architecture.md](docs/architecture.md),
[docs/gotchas.md](docs/gotchas.md).

## Build and run

Rust 1.100.0 is pinned by `rust-toolchain.toml`. On Linux you also need ALSA,
X11/Wayland and fontconfig development headers:

```bash
sudo apt install libasound2-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libxcb1-dev libfontconfig1-dev libfreetype6-dev

cargo build --release --bin oxidezap --bin oxidezapd --bin oxidezap-cli
./target/release/oxidezap
```

The window starts the daemon beside itself, so keep the three binaries in the
same directory. The first build compiles the whole GPUI tree and takes a while.

Releases are published at
[github.com/duckb1t/whatsnative/releases](https://github.com/duckb1t/whatsnative/releases).
Each archive contains the three binaries, plus an optional plugin bundle and a
static web bundle. Linux packages (`.deb`, `.rpm`, `.AppImage`) are described
in [docs/linux-packaging.md](docs/linux-packaging.md).

## Documentation

- [docs/our-changes.md](docs/our-changes.md) — what this fork changed, slice by
  slice, with the gates each one passed.
- [docs/roadmap.md](docs/roadmap.md) — known gaps and why they are not faked.
- [docs/building.md](docs/building.md) — web, plugin and profiling builds.
- [docs/cli.md](docs/cli.md) — the scriptable front end.
- [docs/localization.md](docs/localization.md) — English and Portuguese UI.
- [AGENTS.md](AGENTS.md) — the decisions that hold the codebase together.

## Data and privacy

State lives in one SQLite file under the platform data directory
(`~/.local/share/oxidezap/whatsapp.db` on Linux): device identity, Signal state
and chat history together. Deleting it unlinks the device and discards local
history. No account data, message content or paired identity is included in
this repository; every JID and phone number in the source and tests is a
synthetic fixture. The optional web bridge is off by default and requires a
bearer token; see the [web notes](docs/web.md) before enabling it.

## Plugins

Plugins are `.wasm` modules installed under Settings → Plugins (or beside
`oxidezapd` on a desktop). There is no WASI: the `oxidezap` import module is a
plugin's entire outside world, so it cannot read the disk or open a socket.
Anything that touches the account — sending, marking read, typing — must be
approved separately and can be withdrawn. Start from `examples/template` or
read [docs/plugin-abi.md](docs/plugin-abi.md).

## Known limitations

- Group calls are not live-validated; 1:1 calls are.
- Several server-side or upstream-only features are deliberately not drawn:
  Meta AI/Private Processing, ads, channel subscriptions, business catalogs,
  passkeys, cross-platform chat transfer, story-audience mutation, arbitrary
  member tags, status read receipts, channel role administration, and more.
  They are listed in [docs/roadmap.md](docs/roadmap.md) rather than shipped as
  controls that do nothing.
- A browser tab holding its own session shares one plugin agent; attaching to
  `oxidezapd` gives each plugin a thread.

## Credits

This fork stands on a lot of other people's work:

- [oxidezap/client](https://github.com/oxidezap/client) — the original native
  client this project forks (MIT, copyright João Lucas de Oliveira Lopes).
- [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) — protocol,
  Signal, media and VoIP implementation (MIT).
- [Zed's GPUI](https://github.com/zed-industries/zed) and
  [gpui-component](https://github.com/longbridge/gpui-component) — the UI
  toolkit and component library.
- [SQLite](https://www.sqlite.org/), [rusqlite](https://github.com/rusqlite/rusqlite),
  [diesel](https://diesel.rs/) and the Rust community crates in `Cargo.lock`.
- [Noto Color Emoji](https://github.com/googlefonts/noto-emoji) for the bundled
  emoji font.

The rest of the dependency list, with versions and licenses, is in
`Cargo.lock` and the vendored crates under `vendor/`.

## License

MIT. The upstream copyright notice is preserved in [LICENSE](LICENSE).
