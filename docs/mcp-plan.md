# MCP plan, before somebody ships a fake server

> This is a plan, not a promise that a model gets to drive somebody's WhatsApp
> account. The current `oxidezap-cli mcp` prints a KDL usage spec. It does not
> speak MCP, it does not open a session, and its `--spec` flag is dead code.
> Calling that an MCP interface would be bullshit with a help screen.

## The blunt version

A real MCP server is a thin front end onto the daemon. It speaks MCP over
stdio, opens two typed line protocol connections to `oxidezapd`, and exposes a
small set of reads. No session, no store, no media and no account state in the
MCP process. The daemon still owns all of that.

Fuck Electron, fuck webview. This is a Rust binary with stdin and stdout.

Phase 0 is done as docs only. The SDK, tool names, input schemas, output data,
limits and error codes are frozen in [docs/mcp-tools.md](mcp-tools.md). The user
later approved a write scope, frozen in
[docs/mcp-write-plan.md](mcp-write-plan.md), but no MCP crate, dependency,
socket code, tool or write exists yet. The approved writes wait for repo Wave 4
multi-account.

## Decisions

- Add a separate workspace crate and binary: `oxidezap-mcp`, naming to match the
  existing `oxidezap*` crate and binary rule. Do not overload `oxidezap-cli mcp`.
  The KDL spec printer stays because hosts may already use it and because
  breaking it for no reason is stupid.
- Transport is MCP over stdio using JSON-RPC. No HTTP listener, no websocket, no
  browser tab and no friendly little web panel. A host spawns the binary and
  talks to it on pipes.
- The server freezes `rmcp 3.11.0` as the SDK. Exact pin, minimum server and
  stdio features only. No auth, HTTP, client or reqwest stack. The reasoning and
  the feature list are in `docs/mcp-tools.md`.
- One process serves one daemon profile and one local account. It is selected
  at startup by `--local-account` or `OXIDEZAP_LOCAL_ACCOUNT`. Tool calls have
  no account field. A model cannot wander into another account mid session.
- Two daemon connections are required. `IpcClient` speaks wire
  `RequestEnvelope` frames and covers seven tools. A plain `Endpoint` split
  into `Reader` and `Link` speaks legacy `Request` frames and covers contact
  search and the two privacy reads. `IpcClient` cannot carry those legacy
  frames. The handshake fixes the frame plane on each connection from the
  first hello, and a connection never mixes planes.
- The wire hello names the client, pins the local account, requires account
  selection acknowledgement, asks for no session events and sets `read_only`.
  The legacy hello binds the same account, owns no window, asks for no session
  events and asks for no call video. If no daemon is reachable, return a
  structured tool error. Do not auto start a daemon in the first version. A
  hidden process is how a read tool becomes a surprise.
- All tool results use the same shape as the CLI machine envelope: `ok`, `data`
  and structured `error` with a code and message. The MCP result puts that
  envelope in `structuredContent` and mirrors it as one text item for hosts
  that ignore `structuredContent`. No prose wrapped in JSON, no human table,
  no nonzero exit that hides the reason.
- Start read only. The wire connection sets `read_only: true`. The legacy
  connection whitelists exactly three reads: contact search and the two
  privacy fetches. The approved write tools wait for repo Wave 4 and their own
  gated slices. A model that can send messages is a liability, not a feature,
  until somebody has proved the guard rails.

## First tool set

The first version exposes a curated set of nine reads, not all 22 CLI commands.
The exact schemas and limits are in `docs/mcp-tools.md`.

| Tool | What it does | Why it is in |
| --- | --- | --- |
| `account_status` | Connection state and selected local account facts | Bootstraps the host without dumping the store |
| `list_chats` | Bounded chat summaries with a page cursor | Basic orientation |
| `get_chat` | One chat's local summary | Follow up from `list_chats`, no invented recent rows |
| `list_messages` | Bounded messages for one chat, with a cursor | No unbounded history ever |
| `search_messages` | Bounded search over local stored history | The useful read most people expect |
| `search_contacts` | Local address book only, no server directory | Avoids pretending a local hit is a server fact |
| `list_calls` | Daemon lifetime call log, capped and labelled | Reuses an honest existing surface |
| `read_privacy` | Six generic categories plus DM read receipts, read from the server | Proves account reads can be typed and bounded |
| `store_stats` | Local store counts, no content | Useful diagnostics with no message leak |

Every tool declares its account scope, its maximum result size and whether it
touches the network. A tool that reads the local store says so. A tool that asks
the server says so. `read_privacy` is the only tool that asks the server.

The request itself gets an ordinary ack, but contact and privacy data arrive
first as account-wide frames with no request id. The MCP reader collects those
frames instead of pretending the ack carried data. It does not claim a unique
correlation the protocol does not give it.

`database_bytes` from `store_stats` is the whole shared SQLite file for every
local account in that profile. The tool does not label it as one account's
share. Media bytes and file counts are account scoped.

## Write tools, approved and never casual

The first MCP build stays read only. That is not timidity, it is the difference
between a tool and a loaded gun. The user approved text send, reactions, poll
votes, status posts, channel posts, call start and end, privacy and read receipt
changes, and account selection. The exact list and the hard exclusions are in
[docs/mcp-write-plan.md](mcp-write-plan.md). No logout tool exists.

Writes come after repo Wave 4 multi-account and in separate gated slices with
all of this in place:

- A process flag or environment switch, `--allow-writes` or
  `OXIDEZAP_MCP_ALLOW_WRITES=1`. Default stays read only.
- Per tool declaration of write intent. The daemon's exhaustive
  `ClientRequest::access()` classification and read only enforcement stay the
  backstop, not a decorative comment.
- A two call confirmation token for each write call. The first call returns a
  preview and a token; the second call with that token performs it. No blind
  "trust me" send.
- No pairing, logout, account add or remove, reset, store cleanup, history
  backfill, media writes to disk or message purge in the write set. Those are
  account and device operations, not casual agent toys.
- A local audit line for every accepted write, with request id, tool name,
  account id and outcome. No message bodies, phone numbers, JIDs, tokens or
  account paths.

`select_account` rebinds the process to an existing local account. It never
creates, removes, logs out or resets one. A confirmed switch ends any active
call on the old account and restores the old binding if either hello fails.
Calls land last, after Wave 4 has a real global call coordinator.

## Layout

- New crate `crates/mcp` with binary `oxidezap-mcp`.
- Dependencies stay thin: `oxidezap-ipc`, `oxidezap-wire`, `oxidezap-core`,
  `serde`, `serde_json` and `rmcp` with the exact feature set in
  `docs/mcp-tools.md`.
- The crate holds no session, no chat store and no media. That is the layering
  rule in `AGENTS.md`, not a suggestion.
- Reuse the fake daemon harness pattern from
  `crates/cli/tests/cli_surface.rs` so protocol tests never need a real account.

## Phases

0. **Tool contract and SDK choice.** Done in docs. `docs/mcp-tools.md` freezes
   the nine tools, schemas, limits, transport contract and error codes. No
   network code yet.
1. **Stdio skeleton.** `initialize`, `tools/list`, structured errors and one
   honest tool, `account_status`, against a fake daemon. Tests cover malformed
   JSON-RPC, unknown tools and missing daemon.
2. **Wire read tools.** Add the six other wire-backed reads with page limits
   and synthetic fixtures only. Full gates before any legacy tool lands.
3. **Legacy read tools.** Add contact search and privacy reads with the
   unaddressed frame collector, then run full gates again.
4. **Write wave, after repo Wave 4.** The scope is approved in
   `docs/mcp-write-plan.md`. Add the switch, confirmation flow and audit rules,
   then implement one write tool at a time in the order in that file.
5. **Docs and Linux artifact.** Update `docs/cli.md`, the README and release
   notes when it actually exists. Do not advertise a plan as a product.

## Validation

- Every test uses a fake daemon and `.invalid` fixtures. No real JID, phone
  number, message, call, reaction or account.
- Tool schema tests prove every tool has a bounded result and a declared read
  flag.
- Protocol tests cover `initialize`, `tools/list`, `tools/call`, cancelled
  calls, malformed frames and structured refusals.
- Legacy tests cover unaddressed frames arriving before acks, duplicate frames
  and a refused unknown request.
- Read only default is tested by trying a write tool with the switch off.
- Write tests prove preview tokens are single use, bound to tool, account and
  payload, and that a failed account switch restores the old binding. No test
  logs out, pairs, resets or removes an account.
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
