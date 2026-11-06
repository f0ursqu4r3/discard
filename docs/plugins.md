# Plugins

Discard's interface and behavior are Lua plugins. The default interface is a bundled plugin. Agents can write
plugins too, but a plugin an agent writes never runs until you approve the exact code.

## What a plugin is

A folder with `plugin.toml` and `init.lua`. Lua runs in a Luau sandbox: no files, no network, no processes. The
API is generated from the Rust definitions (`crates/discard-lua/src/api_spec.rs`). Agents read it through the
`plugin_api_docs` tool. The same text is the authoring guide in a Builder agent's memory.

Limits for each plugin: one OS thread and one Luau state, a CPU budget per call (about 250 ms), a 32 MB memory
cap, and a bounded event queue. A plugin that breaks a limit is stopped. Trees may have 2000 nodes and 32 levels.

## Where things live

Under `$DISCARD_HOME` (default `~/.discard`):

| Path | Content |
| --- | --- |
| `plugins/` | A git repository. One folder per installed plugin. One commit per approved change. |
| `plugin-drafts/` | Drafts written by agents. A draft never runs. |
| `plugins.db` | Approvals: which exact bytes you approved, signed with HMAC-SHA256. |
| `plugin-key-salt` | Salt for the passphrase key, if you use `DISCARD_PLUGIN_PASSPHRASE`. Not secret. |

## The review flow

1. The Builder agent (`discard --builder`) writes a draft with `plugin_create` and `plugin_edit`.
2. It tries the draft with `plugin_test`. That loads it in a sandbox and renders its slots with sample state.
3. It submits the draft with `plugin_reload`. Discard takes a copy of the bytes and queues a review.
4. A banner and the **Plugins** button show the waiting review. The review screen shows the code diff, the
   capabilities the plugin asks for (and what changed since the last approved version), and a warning if it asks to
   run programs.
5. **Approve and run** installs exactly the reviewed bytes, commits them, records a signed approval, and loads the
   plugin. If the plugin fails to load, Discard undoes the install and keeps the old version.

Agents have no tool that approves, installs, or trusts. Only the buttons in Discard's own window do.

## Rules Discard enforces

- **Exact bytes.** The review holds a copy of the draft. Editing the draft afterward does not change what is
  installed. After the swap, Discard hashes the installed files and undoes the change if they differ.
- **Every edit is reviewed.** A new review is needed for any code change. Capabilities start at zero.
- **Trust future edits** can be switched on only for a plugin that asks for `ui` and nothing else. Code edits to
  such a plugin load without a new review, but a capability change always needs one.
- **Load-time check.** At startup Discard reads each plugin's files into memory once, hashes those bytes, and loads
  those same bytes only if the hash matches a signed approval. A plugin edited on disk, or dropped in without
  review, is disabled and listed as "modified outside review". A forged database row does not verify.
- **Signing key.** The key lives in the macOS Keychain. If you set `DISCARD_PLUGIN_PASSPHRASE`, a key is derived from
  it instead. With no key at all, Discard loads no user plugins and says why.
- **Failures revert.** A plugin that fails three times in a row, or breaks a CPU or memory limit once, goes back to
  its previous approved version. A plugin with no earlier version is removed.
- **Safe mode.** `discard --safe-mode` loads no user plugins.
- **Protected surfaces.** Approval prompts, the review screen, the plugin list, and the plugin error list are drawn
  by Rust. A plugin can change theme colors, but protected surfaces keep a minimum contrast.
- **Permission.** `plugin.write` is off for every agent by default. The bundled Builder agent has it on.
  The plugin tools are on a separate MCP connection with their own token. `plugin_rollback` always asks you.

## Files and memory

`discard.fs` and `discard.memory` work only after the user approves the capability.

```toml
[capabilities.fs]
paths = ["~/notes", "/tmp/scratch"]   # absolute, or start with ~/. No `..`.
write = false                          # read-only unless true

[capabilities.memory]
scopes = ["project", "global"]         # agent, agent-project, project, global
write = false                          # append and edit need true
```

- **Files.** `read`, `write`, `list`, `exists`, `mkdir`, `remove`. Discard resolves each path and every link in it,
  and refuses any path that ends outside the listed folders. A read or write holds at most 1 MB of UTF-8 text. A
  write replaces the file in one step. `remove` deletes files only.
- **Memory.** `list`, `read`, `search`, `append`, `edit`. A call for a scope the manifest does not name is refused
  before it reaches memory. `edit` needs the opaque version returned by `read`. Agent and project scopes use the
  callback's actual conversation context. Project scopes fail in a direct conversation.
- Plugin `append` creates or updates a versioned memory item. The original path stays usable through `list`, `read`,
  `search`, `append`, and `edit`. `core/` paths create instruction items that enter every applicable response prompt.
  Other paths create fact items. Settings and applicable conversation memory views can inspect, edit, move, forget,
  and undo these items with the same history and version checks as other memory.
- Plugin writes use the approved plugin capability. They are explicit writes, not automatic assistant saves. They
  have no inferred source conversation. Each item stores its original scope and path in its metadata; it has no
  second raw file. The normal item UUID path remains valid in its current scope.
- After a move, the old scope/path returns an error. `list` and `search` in the new scope return the item UUID path.
  The plugin needs a grant for that scope to use it. Forget removes the item from reads and retrieval. Append cannot
  recreate a moved or forgotten item through its old path. Edits made with an old version fail, including after Undo
  restores the same text. Read the current item again before retrying.
- Legacy raw files remain readable. New plugin writes use versioned items. This new application does not migrate
  legacy raw files; plugin mutation of an existing legacy raw path returns an error.
- A failed call raises a Lua error. Use `pcall` to handle it.
- Each write, remove, append, and edit is logged with the plugin name and path.

## Events

`discard.on(name, handler)` receives these events. Delivery is in order, on a separate thread, and the app never
waits for a plugin. A slow plugin loses its oldest queued events.

| Event | Payload |
| --- | --- |
| `message_sent` | `channel_id`, `text` |
| `tool_call` | `call_id`, `tool` |
| `tool_result` | `call_id`, `is_error` |
| `approval_requested` | `approval_id`, `call_id`, `tool` |
| `agent_status` | `status`: `idle`, `working`, `awaiting_approval`, or `offline` |
| `turn_finished` | `outcome`: `completed`, `failed`, or `cancelled` |

Payloads hold names, ids, and flags. They never hold tool input, tool output, or agent text, because those can hold
secrets. The three tool events are sent only if the provider shows tool calls.

## Limits you should know

- `net`, `exec`, and `approvals_decide` can be declared and are shown in review, but the runtime gives plugins no
  functions for them yet.
- File scopes are checked when each call runs, not held open. A folder swapped for a link between the check and the
  use is a race that Discard does not close. The same-user agent threat in `PLAN.md` applies.
- `plugin_test` runs a draft with no files and no memory. A call to `discard.fs` or `discard.memory` fails there, so
  a draft must not call them at load time.
- Plugins see the memory of this app's agent and project only.
- Events are observe-only. A handler can read an event, but a `{ block = true }` result changes nothing yet. Blocking a
  message or a tool call is not built.
- The Keychain may ask for your permission the first time, and again after a rebuild of an unsigned binary.
- Linux and Windows secret stores are not wired. Use `DISCARD_PLUGIN_PASSPHRASE` there.
- The review screen was built and logic-tested but not checked visually in this milestone.

## Menu items

A plugin with the `ui` capability can call `discard.menu_item(target, label, run)`. Targets are `app`, `message`,
`code_block`, `channel`, `diff_block`, `tool_call`, `agent`, and `server`. `app` adds an item to the Plugins menu. Other items appear at the end of the matching
context menu. The label shows as `Label (plugin-name)`. A plugin can register at most eight items. Each label must
contain 1 to 40 characters and no control characters. The callback receives only `{ kind, id }`. A code block ID is
the message ID, a colon, and the block index. An app item receives `kind = "app"` and an empty ID.

For `diff_block` and `tool_call`, the callback ID is the message ID, a colon, and the block index. For `agent` and `server`, it is the agent ID or project ID.
