# MCP write plan, approved but blocked on Wave 4

> Phase 0 froze nine reads in `docs/mcp-tools.md`. This file records the write
> tools the user later approved. Nothing here is implemented yet. No write tool
> ships until repo Wave 4 multi-account is done and each tool passes its own
> gates. Fuck electron, fuck webview.

## Approved scope

The first MCP build stays read only and uses the nine frozen tools. After Wave
4, a separate approved wave adds these tools, one at a time:

| Tool | What it does | Daemon surface |
| --- | --- | --- |
| `select_account` | Rebind the MCP process to another existing local account | Control plane `ListAccounts` plus account-scoped reattach |
| `send_text` | Send one text message, with optional reply target | Wire `SendText` |
| `send_reaction` | Add or remove one emoji reaction | Wire `SendReaction` |
| `vote_poll` | Vote on one poll with selected option indices | Wire `VotePoll` |
| `post_status` | Post one text status | Wire `SendStatus` |
| `post_to_channel` | Post one text item to a named channel | Wire `PostToChannel` |
| `start_call` | Start a voice or video call with confirmation | Existing call start path |
| `end_call` | End the active call on the selected account | Existing call end path |
| `set_privacy` | Set one generic privacy category | Legacy `SetPrivacyCategory` |
| `set_read_receipts` | Set DM read receipts to all or none | Legacy `SetReadReceipts` |

`change settings` means `set_privacy` and `set_read_receipts` only. It does not
mean logout, reset, remove, pair, unpair, purge, key reset or media deletion.

There is no `logout` tool. There is no tool that removes an account, resets an
account, starts pairing, clears local data, writes media to disk, edits the
plugin store or changes the daemon process. Those stay human operations in the
GUI or CLI, where the user can see the whole device consequence.

## Account selection

`select_account` is a write tool because it changes the process binding. It is
the only tool that accepts an account id. Every other tool has no account field
and acts on the currently selected account.

Rules:

1. The account must already exist. `select_account` never creates one.
2. The first call returns a preview with the target account id and a
   confirmation token. It does not rebind.
3. The second call with a fresh token rebinds both daemon connections. Both
   hellos must succeed. If either fails, the old binding is restored and no
   tool runs against a half attached connection.
4. A confirmed switch ends any active call on the old account. The preview says
   so before the token is issued. This follows the Wave 4 product decision,
   already recorded in `docs/multi-account-todo.md`.
5. A switch does not log out, reset, remove or reconnect any WhatsApp session.
   It only changes which existing account the MCP process observes.

## Write gates

Every write tool needs all of this before it lands:

- A process switch, `--allow-writes` or `OXIDEZAP_MCP_ALLOW_WRITES=1`. Default
  stays read only. `read_only_violation` is returned when a write tool is
  called without the switch.
- A two call confirmation flow: preview plus token, then execute. Tokens are
  single use, short lived and bound to the tool name, account id and payload
  digest.
- An explicit `access: write` declaration in the tool schema. The daemon's
  exhaustive access classification and wire `read_only` flag stay the backstop.
- A local audit line with request id, tool name, account id and outcome. No
  message body, phone number, JID, emoji text, poll option text, token or
  account path goes into that line.
- Synthetic fixtures only. No app, daemon, CLI or MCP server launch against a
  real account. No message, reaction, vote, post, call or setting touched
  during validation.

`set_privacy` also applies the core rules already in the tree: reject
`Unknown`, reject `contact_blacklist`, reject any pair where
`category.accepts(value)` is false and reject any value where
`value.is_settable()` is false. `set_read_receipts` only sends `all` or `none`.

## Order

Writes land in this order, each as its own gated slice:

1. `select_account`, after Wave 4 exposes a real account switch path.
2. `send_text`, then `send_reaction` and `vote_poll`.
3. `post_status`, then `post_to_channel`.
4. `set_read_receipts`, then `set_privacy`.
5. `start_call` and `end_call` last, because calls need the Wave 4 global call
   coordinator and the active call confirmation rule.

No batching of these slices. A failed gate stops that slice, not the program.

## Error codes to add in the write wave

The read codes in `docs/mcp-tools.md` stay unchanged. The write wave adds:

| Code | When it is used |
| --- | --- |
| `write_disabled` | A write tool was called without the explicit write switch |
| `confirmation_required` | The first write call correctly returned a preview and token |
| `confirmation_invalid` | The token is expired, used, for another tool or for another payload |
| `account_switch_failed` | One of the two account scoped hellos failed after a switch |
| `active_call_blocks_switch` | A confirmed switch would end a call and the preview was not accepted |
| `call_unavailable` | No active call to end, or the call backend refused the start |

`confirmation_required` is a normal preview outcome, not a server fault. A
failed write never returns `ok: true` with a half applied effect.

## Reopening the frozen contracts

`docs/mcp-tools.md` still freezes the original nine reads and their envelopes.
This document freezes the write scope, not the write schemas. Each write slice
adds its own input and output schema here before code lands. A read contract
only changes if a new daemon response shape forces it, and the reason goes in
`docs/our-changes.md`.
