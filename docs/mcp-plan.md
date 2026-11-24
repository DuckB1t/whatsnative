# MCP plan, before somebody ships a fake server

> This is a plan, not a promise that a model gets to drive somebody's WhatsApp
> account. The current `oxidezap-cli mcp` prints a KDL usage spec. It does not
> speak MCP, it does not open a session, and its `--spec` flag is dead code.
> Calling that an MCP interface would be bullshit with a help screen.

## The blunt version

A real MCP server should be a thin front end onto the daemon. It speaks MCP over
stdio, connects to `oxidezapd` through the same typed line protocol the CLI
uses, and exposes a small set of tools. No session, no store, no media and no
account state in the MCP process. The daemon still owns all of that.

Fuck Electron, fuck webview. This is a Rust binary with stdin and stdout.

## Decisions

- Add a separate workspace crate and binary: `oxidezap-mcp`, naming to match the
  existing `oxidezap*` crate and binary rule. Do not overload `oxidezap-cli mcp`.
  The KDL spec printer stays because hosts may already use it and because
  breaking it for no reason is stupid.
- Transport is MCP over stdio using JSON-RPC. No HTTP listener, no websocket, no
  browser tab and no friendly little web panel. A host spawns the binary and
  talks to it on pipes.
- The server connects with `IpcClient` from `crates/ipc/src/client.rs` and uses
  typed `ClientRequest` and `DaemonMessage` frames. It does not shell out to the
  CLI. Shelling out would inherit human formatting, exit codes and raw text
  refusal, then turn all of it back into JSON. That is a clown car.
- The hello handshake says this front end does not own a window. If no daemon is
  reachable, return a structured tool error. Do not auto start a daemon in the
  first version. A hidden process is how a read tool becomes a surprise.
- Account selection is explicit. Tool calls carry an account id or the process
  was started with one. No silent grab of whatever account happens to be
  selected.
- All tool results use the same shape as the CLI machine envelope: `ok`, `data`
  and structured `error` with a code and message. No prose wrapped in JSON, no
  human table, no nonzero exit that hides the reason.
- Start read only. Write tools are a later wave with their own gates. A model
  that can send messages is a liability, not a feature, until somebody has
  proved the guard rails.

## First tool set

The first version exposes a curated set of reads, not all 22 CLI commands.

| Tool | What it does | Why it is in |
| --- | --- | --- |
| `account_status` | Connection state and selected account facts | Bootstraps the host without dumping the store |
| `list_chats` | Bounded chat summaries with a page cursor | Basic orientation |
| `get_chat` | One chat's local summary and bounded recent rows | Follow up from `list_chats` |
| `list_messages` | Bounded messages for one chat, with a cursor | No unbounded history ever |
| `search_messages` | Bounded search over local history | The useful read most people expect |
| `search_contacts` | Local and server contact lookup, clearly labelled by source | Avoids pretending a local hit is a server fact |
| `list_calls` | Daemon lifetime call log, capped and labelled | Reuses an honest existing surface |
| `read_privacy` | The six protocol 63 categories plus DM read receipts | Proves account reads can be typed and bounded |
| `store_stats` | Local store counts, no content | Useful diagnostics with no message leak |

Every tool declares its account scope, its maximum result size and whether it
touches the network. A tool that reads the local store says so. A tool that asks
the server says so. A tool that can do both says which part came from where.

## Write tools, later and never casual

Writes are out of scope for the first MCP wave. That is not timidity, it is the
difference between a tool and a loaded gun.

When writes do arrive, they come in a separate wave with all of this in place:

- A process flag or environment switch, for example `--allow-writes` or
  `OXIDEZAP_MCP_ALLOW_WRITES=1`. Default stays read only.
- Per tool declaration of write intent. The daemon's exhaustive
  `ClientRequest::access()` classification and read only enforcement stay the
  backstop, not a decorative comment.
- A confirmation token for each write call. The first call returns a preview and
  a token; the second call with that token performs it. No blind "trust me"
  send.
- No pairing, logout, account add or remove, store cleanup, history backfill,
  media writes to disk, call start, or message purge in the first write set.
  Those are account and device operations, not casual agent toys.
- A local audit line for every accepted write, with request id, tool name,
  account id and outcome. No message bodies, no phone numbers, no tokens.

A future write set can cover text send, reactions, chat read state and poll
votes. Each one needs its own tests and its own honest limitation note.

## Layout

- New crate `crates/mcp` with binary `oxidezap-mcp`.
- Dependencies stay thin: `oxidezap-ipc`, `oxidezap-wire`, `oxidezap-core`,
  `serde` and whatever small MCP SDK survives review.
- Phase 0 chooses the SDK. If a maintained crate gives us `initialize`,
  `tools/list`, `tools/call` and structured errors without dragging in a web
  stack, use it. If it fights Rust 1.100 or pulls half the browser with it,
  hand roll the small JSON-RPC subset. Being clever is not worth a dependency
  swamp.
- The crate holds no session, no chat store and no media. That is the layering
  rule in `AGENTS.md`, not a suggestion.
- Reuse the fake daemon harness pattern from
  `crates/cli/tests/cli_surface.rs` so protocol tests never need a real account.

## Phases

1. **Tool contract and SDK choice.** Freeze the table above, pick the SDK,
   write the JSON schema for each tool and settle the error codes. No network
   code yet.
2. **Stdio skeleton.** `initialize`, `tools/list`, structured errors and one
   honest tool, `account_status`, against a fake daemon. Tests cover malformed
   JSON-RPC, unknown tools and missing daemon.
3. **Read tools.** Add the rest of the table with page limits, source labels and
   synthetic fixtures only. Full gates before any write discussion.
4. **Write wave, only if explicitly approved.** Reopen this plan, add the
   confirmation flow and audit rules, then implement one write tool at a time.
5. **Docs and Linux artifact.** Update `docs/cli.md`, the README and release
   notes when it actually exists. Do not advertise a plan as a product.

## Validation

- Every test uses a fake daemon and `.invalid` fixtures. No real JID, phone
  number, message, call, reaction or account.
- Tool schema tests prove every tool has a bounded result and a declared write
  flag.
- Protocol tests cover `initialize`, `tools/list`, `tools/call`, cancelled
  calls, malformed frames and structured refusals.
- Read only default is tested by trying a write tool with the switch off.
- No app, daemon or CLI launch during validation. The fake daemon is a test
  double, not the installed service.
- Repository gates still apply: fmt, clippy `-D warnings`, workspace nextest,
  doctests, xtask tests, `git diff --check`, no added em or en dashes and the
  no account leak audit.

## Non-goals

- No Electron, no webview, no browser extension and no local web server.
- No automatic daemon spawn and no pairing through MCP.
- No full mirror of the CLI command surface. A small honest tool is better than
  twenty traps.
- No sending anything to a real account during validation.
- No claim that a model can safely act on somebody's behalf just because the
  JSON validates. The user decides what gets exposed.
