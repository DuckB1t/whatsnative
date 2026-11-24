# MCP tool contracts, frozen in Phase 0

> No MCP server ships. This file freezes the tool shapes so the next wave cannot
> quietly invent a tool, widen a limit, or pretend a local read came from the
> server. If a contract changes, this file changes first and the reason goes in
> `docs/our-changes.md`.

The server name is still `oxidezap-mcp`, a native Rust stdio JSON-RPC process.
It holds no session, no store, no media and no account state. The daemon owns
all of that. Fuck electron, fuck webview.

## Ground rules

- There are exactly nine tools. No tenth tool without a new plan entry.
- One process serves one daemon profile and one local account. The account is
  fixed at startup by `--local-account <positive-id>` or
  `OXIDEZAP_LOCAL_ACCOUNT`. There is no account field on any tool call. A
  missing or invalid account is a startup failure, not a default grab.
- Every tool is a read. The wire connection hello sets `read_only: true`. The
  legacy connection has no read-only hello flag, so it whitelists exactly three
  request names: `search_contacts`, `fetch_privacy_settings` and
  `fetch_privacy_categories`. Anything else is a code bug, not a tool.
- Every envelope is capped at 1 MiB of UTF-8 JSON after serialization. A bigger
  result fails with `response_too_large`. Nothing is silently truncated.
- Every array limit below is enforced before the daemon request is sent. A
  number that is negative, fractional, zero, or above the cap is an
  `invalid_argument` error. Daemon clamps are not the tool contract.
- Page cursors are opaque strings. A tool never parses, builds or rewrites one.
- A field emitted as `null` is present and null. A field omitted by the daemon
  is absent. The schemas say which is which.

## Envelope and MCP wrapper

The tool payload is the CLI machine envelope:

```json
{ "ok": true, "data": { } }
```

A tool failure is:

```json
{ "ok": false, "error": { "code": "not_found", "message": "no such chat" } }
```

The MCP `tools/call` result carries that envelope in `structuredContent`.
It also puts one text item in `content` with the same JSON, for hosts that
ignore `structuredContent`. Failures set `isError: true`. Success sets
`isError: false` or leaves it out. Example:

```json
{
  "content": [
    { "type": "text", "text": "{\"ok\":true,\"data\":{\"calls\":[]}}" }
  ],
  "structuredContent": { "ok": true, "data": { "calls": [] } },
  "isError": false
}
```

No tool puts a human table, a markdown paragraph, or a raw daemon frame in
`content`.

## Transport contract

Two sockets are opened at startup. Both bind the same local account. If either
hello fails, both sockets close and the failure code is cached. The process
still answers `tools/list`, and every `tools/call` fails with that startup code
in the envelope. No tool runs against a half attached connection.

1. **Wire plane.** `IpcClient` from `crates/ipc/src/client.rs` speaks
   `RequestEnvelope` and `ResponseEnvelope` from `crates/wire`. The first frame
   is `hello` with `client_name: "oxidezap-mcp"`, `local_account`,
   `account_selection: true`, `read_only: true` and `session_events: false`.
   The ack is `account_selected`, and its `local_account` is the value reported
   by `account_status`. Seven tools use this plane.
2. **Legacy plane.** A separate `Endpoint` is split into `Reader` and `Link`.
   `Link::send_line` writes legacy `Request` frames with an `id`. The reader
   decodes legacy `DaemonMessage` frames. The first frame is `hello` with
   `scope: { "scope": "account", "account": <id> }`, `owns_window: false`,
   `session_events: false` and `call_video: false`. The `hello` frame is the
   only non-tool request the writer sends. Three tools use this plane.

`IpcClient` cannot carry the legacy frames. It parses wire envelopes only.
Calling it for privacy or contact search would produce parse loops or silent
drops, so the MCP process must not try. The daemon's handshake fixes the frame
plane from the first hello. A connection cannot mix planes and cannot be
rebound to another plane after a refusal.

`IpcClient::request` is synchronous. The MCP SDK is tokio async. A real
implementation puts blocking wire calls on a dedicated worker behind a channel
and never calls `request` directly in an async task. The legacy reader also
runs on a dedicated worker. No web thread rules apply here, because this is a
native process, but blocking the MCP runtime is still a bug.

### Unaddressed legacy answers

Legacy `search_contacts`, `fetch_privacy_categories` and
`fetch_privacy_settings` get a correlated ack, and publish their data first as
account-wide frames with no request id:

| Request | Data frame |
| --- | --- |
| `search_contacts` | `contact_results` with `results` |
| `fetch_privacy_categories` | `privacy_categories` with `settings` |
| `fetch_privacy_settings` | `privacy_settings` with `read_receipts` |

The data frame arrives before the ack. A collector subscribes before it sends,
keeps frames until the matching ack or an error, and takes the first matching
data frame. These frames are account state, not a private pipe. Another front
end asking at the same time can cause another identical frame. The collector
does not claim a total or a unique correlation that the protocol does not
give it. Tests cover duplicate and interleaved frames.

Read privacy performs both network fetches and returns one object. Either
fetch failing fails the tool. There is no partial privacy result and no
guessed value. `unknown` is a real reported value, not an error.

## SDK choice

Frozen: `rmcp` exactly `3.11.0`, Apache-2.0, edition 2024, MSRV 1.88. The
workspace toolchain is 1.100.0, so this is not a squeeze.

Use `default-features = false` with `server`, `macros` and `transport-io`.
That gives server tools, schema generation from `schemars`, the tool macros and
server stdio transport. Do not enable `auth`, `client`, `server-side-http`,
`transport-child-process`, `skills-fs`, `reqwest` or any TLS feature. No HTTP
listener, no OAuth stack and no outbound fetch belong in this binary.

This is a dependency decision only. Phase 0 adds no crate, no manifest edit
and no lockfile change.

## Tool index

| Tool | Plane | Daemon action | Source | Row cap | Network |
| --- | --- | --- | --- | --- | --- |
| `account_status` | wire | `get_status` plus wire hello ack | daemon connection state | 1 | no |
| `list_chats` | wire | `list_chats` | local chat list | 100 | no |
| `get_chat` | wire | `get_chat` | one local chat summary | 1 | no |
| `list_messages` | wire | `list_messages` | local stored history | 200 | no |
| `search_messages` | wire | `search_messages` | local full text index | 100 | no |
| `search_contacts` | legacy | `search_contacts` | local address book only | 100 | no |
| `list_calls` | wire | `list_calls` | daemon lifetime call log, capped at 100 | 100 | no |
| `read_privacy` | legacy | two privacy fetches | server account settings | 7 | yes |
| `store_stats` | wire | `get_storage_usage` | local store and media cache | 1 | no |

`get_chat` returns one chat summary. It does not return recent messages, even
though the old plan said that. A caller that wants rows calls
`list_messages`. `search_contacts` searches the local address book. It never
asks the server directory, and a miss means not in this book. `database_bytes`
in `store_stats` is the whole shared SQLite file, not one account's share.
`media_bytes` and `media_files` are account scoped.

## Schema conventions

All input and output objects set `additionalProperties: false` unless stated.
All schemas use JSON Schema 2020-12. `$defs` below are shared definitions and
each tool schema is published with that whole block merged in. Every output
object is wrapped in the success envelope from above.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$defs": {
    "chat": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "jid", "name", "unread_count", "manually_unread", "is_group",
        "is_pinned", "is_muted", "is_archived", "last_message_ts",
        "last_message_preview"
      ],
      "properties": {
        "jid": { "type": "string" },
        "name": { "type": "string" },
        "unread_count": { "type": "integer", "minimum": 0 },
        "manually_unread": { "type": "boolean" },
        "is_group": { "type": "boolean" },
        "is_pinned": { "type": "boolean" },
        "is_muted": { "type": "boolean" },
        "is_archived": { "type": "boolean" },
        "last_message_ts": { "type": ["integer", "null"] },
        "last_message_preview": { "type": ["string", "null"] }
      }
    },
    "media": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "file_sha256", "mime_type", "filename", "size_bytes",
        "is_downloaded", "local_path"
      ],
      "properties": {
        "file_sha256": { "type": "string" },
        "mime_type": { "type": "string" },
        "filename": { "type": ["string", "null"] },
        "size_bytes": { "type": "integer", "minimum": 0 },
        "is_downloaded": { "type": "boolean" },
        "local_path": { "type": ["string", "null"] }
      }
    },
    "reaction": {
      "type": "object",
      "additionalProperties": false,
      "required": ["sender_jid", "emoji", "timestamp_ms"],
      "properties": {
        "sender_jid": { "type": "string" },
        "emoji": { "type": "string" },
        "timestamp_ms": { "type": "integer" }
      }
    },
    "message_poll": {
      "type": "object",
      "additionalProperties": false,
      "required": ["question", "options", "selectable_count"],
      "properties": {
        "question": { "type": "string" },
        "options": { "type": "array", "items": { "type": "string" } },
        "selectable_count": { "type": "integer", "minimum": 0 }
      }
    },
    "message": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "id", "chat_jid", "sender_jid", "from_me", "timestamp_ms", "text",
        "kind", "status", "is_starred", "is_pinned", "reply_to_id", "media",
        "reactions"
      ],
      "properties": {
        "id": { "type": "string" },
        "chat_jid": { "type": "string" },
        "sender_jid": { "type": "string" },
        "from_me": { "type": "boolean" },
        "timestamp_ms": { "type": "integer" },
        "text": { "type": ["string", "null"] },
        "kind": { "type": "string" },
        "status": { "type": "string" },
        "is_starred": { "type": "boolean" },
        "is_pinned": { "type": "boolean" },
        "reply_to_id": { "type": ["string", "null"] },
        "media": { "anyOf": [{ "$ref": "#/$defs/media" }, { "type": "null" }] },
        "reactions": {
          "type": "array",
          "items": { "$ref": "#/$defs/reaction" }
        },
        "poll": { "$ref": "#/$defs/message_poll" }
      }
    },
    "call_participant": {
      "type": "object",
      "additionalProperties": false,
      "required": ["jid"],
      "properties": {
        "jid": { "type": "string" },
        "name": { "type": "string" },
        "state": { "type": "string" },
        "participant_type": { "type": "string" },
        "error": { "type": "integer", "minimum": 0 }
      }
    },
    "call": {
      "type": "object",
      "additionalProperties": false,
      "required": [
        "id", "chat_jid", "caller_jid", "timestamp_ms", "is_video",
        "is_group", "duration_seconds", "outcome"
      ],
      "properties": {
        "id": { "type": "string" },
        "chat_jid": { "type": "string" },
        "caller_jid": { "type": "string" },
        "timestamp_ms": { "type": "integer" },
        "is_video": { "type": "boolean" },
        "is_group": { "type": "boolean" },
        "group_jid": { "type": "string" },
        "participants": {
          "type": "array",
          "items": { "$ref": "#/$defs/call_participant" }
        },
        "group_transaction_id": { "type": "integer", "minimum": 0 },
        "group_media": { "type": "string" },
        "duration_seconds": { "type": ["integer", "null"], "minimum": 0 },
        "outcome": { "type": "string" }
      }
    },
    "contact": {
      "type": "object",
      "additionalProperties": false,
      "required": ["jid"],
      "properties": {
        "jid": { "type": "string" },
        "name": { "type": "string" },
        "push_name": { "type": "string" },
        "phone": { "type": "string" },
        "is_business": { "type": "boolean" },
        "alias": { "type": "string" },
        "tags": { "type": "array", "items": { "type": "string" } }
      }
    },
    "privacy_category": {
      "type": "string",
      "enum": [
        "last_seen", "online", "profile_photo", "about", "group_add",
        "messages", "read_receipts", "unknown"
      ]
    },
    "privacy_value": {
      "type": "string",
      "enum": [
        "all", "contacts", "none", "match_last_seen", "contact_blacklist",
        "unknown"
      ]
    },
    "privacy_setting": {
      "type": "object",
      "additionalProperties": false,
      "required": ["category", "value"],
      "properties": {
        "category": { "$ref": "#/$defs/privacy_category" },
        "value": { "$ref": "#/$defs/privacy_value" }
      }
    }
  }
}
```

`kind` and `status` are daemon strings, not tool enums. A new daemon label is
data, not a tool error. Privacy category and value are enums because the core
types map them and keep `unknown` for anything unrecognised.

## 1. `account_status`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {}
}
```

Output `data`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": [
    "local_account", "state", "phone", "name", "jid", "lid", "qr_ascii",
    "pair_code", "pair_expires_at_ms"
  ],
  "properties": {
    "local_account": { "type": "integer", "minimum": 1 },
    "state": { "type": "string" },
    "phone": { "type": ["string", "null"] },
    "name": { "type": ["string", "null"] },
    "jid": { "type": ["string", "null"] },
    "lid": { "type": ["string", "null"] },
    "qr_ascii": { "type": ["string", "null"] },
    "pair_code": { "type": ["string", "null"] },
    "pair_expires_at_ms": { "type": ["integer", "null"] }
  }
}
```

`local_account` comes from the wire hello ack. The rest comes from
`get_status`. This tool never starts a daemon and never starts pairing.

## 2. `list_chats`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 },
    "offset": { "type": "string", "minLength": 1 },
    "query": { "type": "string", "minLength": 1 },
    "archived": { "type": "boolean", "default": false }
  }
}
```

Output `data`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["chats", "next_cursor"],
  "properties": {
    "chats": {
      "type": "array",
      "maxItems": 100,
      "items": { "$ref": "#/$defs/chat" }
    },
    "next_cursor": { "type": ["string", "null"] }
  }
}
```

## 3. `get_chat`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["jid"],
  "properties": {
    "jid": { "type": "string", "minLength": 1 }
  }
}
```

Output `data` is exactly `$defs/chat`. No message rows, no media count and no
derived facts.

## 4. `list_messages`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["chat_jid"],
  "properties": {
    "chat_jid": { "type": "string", "minLength": 1 },
    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 },
    "before": { "type": "string", "minLength": 1 },
    "after": { "type": "string", "minLength": 1 }
  },
  "not": { "required": ["before", "after"] }
}
```

Output `data`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["messages", "next_cursor"],
  "properties": {
    "messages": {
      "type": "array",
      "maxItems": 200,
      "items": { "$ref": "#/$defs/message" }
    },
    "next_cursor": { "type": ["string", "null"] }
  }
}
```

The daemon prefers `after` when both cursors are present. The tool refuses
both instead of hiding that choice.

## 5. `search_messages`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["query"],
  "properties": {
    "query": { "type": "string", "minLength": 1 },
    "chat_jid": { "type": "string", "minLength": 1 },
    "has_media": { "type": "boolean", "default": false },
    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 }
  }
}
```

Output `data`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["messages", "next_cursor"],
  "properties": {
    "messages": {
      "type": "array",
      "maxItems": 100,
      "items": { "$ref": "#/$defs/message" }
    },
    "next_cursor": { "type": "null" }
  }
}
```

Search is local only and has no next page. `next_cursor` is present and null
because that is the daemon response shape.

## 6. `search_contacts`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["query"],
  "properties": {
    "query": { "type": "string" },
    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 }
  }
}
```

Output `data`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["results"],
  "properties": {
    "results": {
      "type": "array",
      "maxItems": 100,
      "items": { "$ref": "#/$defs/contact" }
    }
  }
}
```

An empty query asks for the first bounded page of the local address book. The
result has no total and no server source. `phone` is present only when the
local address holds one.

## 7. `list_calls`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 }
  }
}
```

Output `data`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["calls"],
  "properties": {
    "calls": {
      "type": "array",
      "maxItems": 100,
      "items": { "$ref": "#/$defs/call" }
    }
  }
}
```

This is the daemon's in-memory log since it started, not durable history. A
restart empties it.

## 8. `read_privacy`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {}
}
```

Output `data`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["settings", "read_receipts"],
  "properties": {
    "settings": {
      "type": "array",
      "maxItems": 7,
      "items": { "$ref": "#/$defs/privacy_setting" }
    },
    "read_receipts": { "$ref": "#/$defs/privacy_value" }
  }
}
```

`read_receipts` has only `all`, `none` and `unknown` in practice. The schema
uses the shared value enum so a reported value is never renamed to fit a
narrower guess. Missing categories stay missing. `contact_blacklist` is a
reported value only. There is no privacy write tool here.

## 9. `store_stats`

Input:

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {}
}
```

Output `data`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["database_bytes", "media_bytes", "media_files"],
  "properties": {
    "database_bytes": { "type": "integer", "minimum": 0 },
    "media_bytes": { "type": "integer", "minimum": 0 },
    "media_files": { "type": "integer", "minimum": 0 }
  }
}
```

`database_bytes` is the shared SQLite file for every local account in this
profile. Do not label it as one account's database. The two media fields are
account scoped because every cache key carries its account prefix.

## Error codes

MCP protocol failures before a tool runs use JSON-RPC codes: `-32600` malformed
request, `-32601` unknown method or unknown tool name, `-32602` bad params,
`-32603` internal server fault, `-32800` cancelled. Those are not tool
envelopes.

Startup binding failures are the seven codes from `invalid_account_id` through
`frame_plane_mismatch` in the table below. They do not kill the process. The
server answers `tools/list`, and every `tools/call` returns a failure envelope
with the cached startup code instead of running a tool.

Once a tool is executing, the envelope uses these codes. `message` is a short
human sentence. No code carries a phone number, JID, message body, token or
account path in its message.

| Code | When it is used |
| --- | --- |
| `invalid_account_id` | The startup account id is not a positive integer |
| `account_not_selected` | No startup account flag or environment value |
| `account_unavailable` | The named account is missing, retired, stopping or could not be selected |
| `daemon_not_running` | No daemon is listening on the resolved socket |
| `socket_connect_failed` | The socket exists but connection failed for another I/O reason |
| `protocol_mismatch` | Wire or legacy hello reports an unsupported protocol version |
| `frame_plane_mismatch` | A connection answered with the wrong frame shape for its plane |
| `invalid_request` | A daemon frame was malformed or not valid for the current connection state |
| `invalid_argument` | Tool input failed schema validation or daemon input validation |
| `not_connected` | The account has no live session, or a network read could not reach the server |
| `not_found` | The named chat or message is not in local state |
| `permission_denied` | The daemon refused an otherwise valid request |
| `too_busy` | The daemon is at its client or command limit |
| `timeout` | A hello, ack or unaddressed data frame did not arrive in time |
| `unsupported` | The daemon does not have the requested capability or frame |
| `response_too_large` | The serialized result would exceed 1 MiB |
| `internal_error` | A retryable or otherwise uncategorised daemon failure, or a local serialization fault |
| `read_only_violation` | Reserved for a future write tool reached without the explicit write switch |

Legacy error mapping is fixed: `version_mismatch` maps to `protocol_mismatch`,
`malformed` to `invalid_request` or `invalid_argument` depending on whether the
daemon or the tool made the bad frame, `no_session` to `not_connected`,
`refused` to `permission_denied`, `failed` to `internal_error` with the retry
flag named in the message, and `too_many_clients` to `too_busy`. Wire `ApiError`
codes pass through unchanged when they are in this table. An unknown daemon
code becomes `internal_error`, never a made up synonym.

## Proof expected in the implementation wave

- Fake daemon only. Every JID and phone fixture uses `.invalid`.
- Schema tests pin all nine tool names, input bounds and `additionalProperties`.
- A fake wire daemon covers hello, account selection, every response shape and
  each mapped error.
- A fake legacy daemon covers the three allowed request names, an unaddressed
  data frame before its ack, duplicate frames, and a refused unknown request.
- No app, daemon, CLI or MCP binary launch against a real account. No pairing,
  message, call, reaction, vote, post or setting is touched during validation.
- Repository gates still apply: fmt, clippy with warnings denied, workspace
  nextest, doctests, xtask test and clippy, `git diff --check`, the added-line
  dash audit and the no account leak audit.
