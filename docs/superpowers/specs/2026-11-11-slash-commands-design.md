# Slash commands

Date: 2026-11-11

## Goal

A user types `/name` in a composer to run a Discard command. Built-in commands and plugin commands use one registry. A command runs in Discard. It never reaches the agent.

## Why

- Plugins can register a command today (`discard.command` in `crates/discard-lua/src/api_spec.rs`), and the plugin manager can run it (`run_command` in `crates/discard-lua/src/manager.rs`). No app code calls it. The API is dead code.
- `PLAN.md` lists slash commands as plugin functionality.
- The Claude provider runs with `--disable-slash-commands`. The `/` prefix has no other owner.

## Owner decisions (2026-11-11)

The owner approved the feature and left all design decisions to the designer. Each decision below is the designer's call.

## Terms

- **Command**: a named operation that runs in Discard from a composer.
- **Command message**: composer text that has the command form (decision 1).
- **Built-in command**: a command that Discard defines.
- **Plugin command**: a command that a Lua plugin registers with `discard.command`.
- **Planned command**: a built-in command with no behavior yet. See `AGENTS.md`, "Unimplemented UI actions".
- **Command popup**: the autocomplete list for commands.
- **Result panel**: the area above the composer input that shows the output or the error of the last command.

## Goals

- Run built-in commands and plugin commands from every composer.
- Show the available commands while the user types.
- Keep command text and command output out of the transcript and out of the agent context.

## Non-goals

- A command that sends a prompt to the agent (prompt templates). A later spec can add it.
- A command palette. `PLAN.md` plans it as a separate surface.
- A context argument for plugin command handlers (channel, project, thread).
- A qualified form such as `/plugin:name` for a shadowed command.
- A UI surface that lists shadowed plugin commands.
- Structured arguments, argument autocomplete, and argument validation by Discard.
- Rich output. The output is plain text.

## Anti-goals

- A typed command that reaches the agent as message text, for example after a typing error in the name.
- A plugin command that replaces or hides a built-in command.
- A planned command that does a no-op or opens an unrelated destination.

## Decisions

### Command form

1. Composer text is a command message when it matches `^/([a-z][a-z0-9_-]{0,31})(\s|$)`. The match is on the text as typed, from the first character. Leading whitespace makes the text a normal message.
2. The command name is capture group 1. The arguments are the remaining text, trimmed. The arguments can have more than one line.
3. Text such as `/usr/bin/env` does not match, because `/` follows the name. It is a normal message.
4. Text that starts with `//` is a normal message. Discard removes the first `/` before the send. This is the escape for a literal message that has the command form.
5. The name match is exact and lowercase. `/Help` does not have the command form, and it is a normal message.

### Registry

6. Rust owns the registry. The webview gets it through a Tauri command and does not keep its own list of built-in commands.
7. A registry entry has: name, description, argument hint (optional), source (`builtin` or the plugin name), and state (`ready` or `planned`). The plugin info struct stores only command names today. It must also store the description and the argument hint.
8. A built-in name wins. A plugin command with a built-in name is not in the registry. Discard writes a warning to its log that names the plugin and the command. The check runs each time the registry is built, so a new built-in name in a later release produces the warning on the next load.
9. Between plugins, the first plugin in load order wins. This is the current behavior of `run_command`. Discard writes a warning to its log for each shadowed command. A shadowed command is not a plugin failure, and it does not show in the UI.
10. Only plugins with the status `Running` supply commands. In safe mode the registry has only built-in commands.
11. The registry changes when plugins load, reload, or stop. The webview reads the registry each time the command popup opens, so it does not need a change event.

### Built-in commands

12. A built-in command that has a matching `Action` dispatches that `Action` through `actions::run`. It adds no second handler. `PLAN.md` requires this.
12a. A command acts on the channel of its composer. The dispatch passes that channel as the action target. Where an `Action` handler ignores the target today (`CancelTurn` uses the active channel), the builder extends that handler to use a channel target. The handler stays the single handler for all surfaces.
12b. In the thread composer the channel is the thread channel. `/stop` there stops the turn of the thread, as the Stop control of that composer does. `/archive` there returns the error `/archive does not apply to a thread.`
12c. In the direct message composer the channel is the direct message channel. The commands follow the same rules as in a channel composer.

| Command | Behavior | State |
|---|---|---|
| `/help` | Shows all registry entries in the result panel: name, argument hint, description, source. | ready |
| `/stop` | `Action::CancelTurn` with the composer's channel as the target. | ready |
| `/settings` | `Action::OpenSettings`. | ready |
| `/members` | `Action::ToggleMemberList`. | ready |
| `/plugins` | `Action::ReviewPlugins`. | ready |
| `/archive` | `Action::ArchiveChannel` with the composer's channel as the target. It starts the existing archive flow. | ready |
| `/model` | Change the model of the channel's agent. | planned |
| `/clear` | Start a new agent session in the channel. | planned |

13. A built-in command ignores arguments that it does not use.
14. A built-in command that the current state blocks returns an error and does nothing. Examples: `/stop` with no running turn returns `No turn is running.`; `/archive` during a turn returns the existing text `Stop the turn before archiving the channel`. A built-in command adds no state check that its `Action` does not have.
14a. `/archive` returns when the existing archive flow starts. The confirm step and the later notices show as they do today. The result panel shows only a refusal that is known at once.
15. A planned command has no handler. Its shared definition has a `TODO` comment that states the missing behavior. Running it returns the error `/name is not available yet.`

### Plugin API

16. `discard.command` gets an optional third argument: `discard.command(name, run, opts?)`. `opts` is a table with the optional string fields `description` and `args` (the argument hint). A call with two arguments stays valid.
17. Discard refuses a name that does not match `^[a-z][a-z0-9_-]{0,31}$`. The call raises a Lua error, so the plugin fails to load with a clear message.
18. `description` has a limit of 120 characters and `args` a limit of 40 characters. Discard truncates longer values.
19. A plugin command with no description shows the text `Plugin command` in the command popup and in `/help`.
20. The handler signature does not change: `run(args: string) -> string?`. The existing call budget and deadline apply. The call holds the plugin manager lock, as other plugin calls do. The deadline limits the wait.
20a. A plugin command runs in the memory context (agent and project) of the composer's channel, as `core.plugin_action` does for the active channel.
21. `api_spec.rs` and the generated plugin API documentation show the new signature.

### Run

22. On send, the composer tests the text for the command form before all other send rules. A command message never calls `send_message`, and it does not fire the `message_sent` plugin event.
23. A new Tauri command runs the command. Its input is the channel id, the name, and the arguments. Its result is the optional output text, or an error.
24. Rust validates the name again and looks it up in the registry. The webview is not trusted to send only known names.
25. An unknown name returns the error `Unknown command: /name`. The draft stays in the composer, so the user can correct it.
26. A command can run while a turn runs. The composer's rule that blocks a send during a turn does not apply to a command message. `/stop` depends on this.
27. The composer is disabled when its channel is archived or when its `busy` prop is set. No command runs then. This state is separate from the state in decision 32.
28. A command message with attachments does not run. The result panel shows `A command cannot have attachments.` The draft and the attachments stay.
29. A pending reply does not change a command and is not consumed by it. The reply chip stays.
30. In the thread composer, a command ignores the "Also post to" box and does not clear it.
31. A plugin command must not block the UI thread or the Tauri main thread while it runs.
32. The composer does not accept a second command while one runs. The send control shows the existing busy state.
33. On success the draft clears. On an error the draft stays.
34. A plugin command error goes through the existing plugin failure path (`core.plugin_failure`), and the result panel also shows it. The failure path can replace an earlier plugin notice, as it does for other plugin calls. The error text shows as plain text with the limit of decision 38.

### Result panel

35. The result panel is above the composer input, in the same stack as the reply chip and the attachment row.
36. It shows the last command line (`/name args`, one line, truncated) and the output or the error below it.
37. The output is plain text with preserved line breaks. Discard does not render it as Markdown or HTML.
38. Discard truncates output above 16 KB and appends `… (truncated)`.
39. A success with no output shows no result panel. An earlier result panel closes.
40. The result panel has `role="status"` for output and `role="alert"` for an error. An error uses the destructive color token.
41. The result panel has a dismiss button with the accessible label `Dismiss command result`. It also closes when the user sends a message or runs another command.
42. Each composer keeps its own result. The result is not saved. It does not survive a channel switch or a restart.
43. The result panel has a maximum height and scrolls inside it.

### Command popup

44. The command popup opens when the whole draft matches `^/[a-z0-9_-]*$`. It closes when the draft no longer matches, for example after the first space.
45. It uses the same listbox pattern as the mention popup in `Composer.vue`: the same position, the same visual style, `role="listbox"`, and `aria-activedescendant` on the input. The two popups cannot be open at the same time, because the mention popup needs `@`.
46. It lists registry entries whose name starts with the typed text. Built-in commands come first in the order of the table in decision 12. Plugin commands follow in alphabetical order.
47. Each row shows `/name`, the argument hint, and the description. A plugin command row also shows the plugin name.
48. A planned command row is disabled, cannot be selected, and shows the label `Soon`.
49. Up and Down move the selection and skip disabled rows. Escape closes the popup and keeps the draft.
50. Tab completes the selected row: the draft becomes `/name ` with the caret at the end.
51. Enter runs the selected command when the draft is equal to `/name` of the selected row. Otherwise Enter completes the row, as Tab does.
52. When no entry matches, the popup shows one disabled row: `No matching command`. Enter then sends the draft through the normal run path, which returns the unknown command error.
53. When the registry request fails, the popup shows one disabled row: `Could not load commands.` A typed command still runs on Enter.
54. The command popup is available in the channel composer, the thread composer, and the direct message composer.

## Constraints

- Follow `AGENTS.md`: planned commands stay visible, disabled, and labelled `Soon`; `TODO` stays in code.
- Web tests go in a `__tests__` directory beside the code they test.
- The parse rule lives in one pure TypeScript module, like `mentions.ts`. Rust has the same rule for its own validation. A comment in each names the other.
- No change to the transcript schema, the store, or the provider adapters.

## Tests

- Parse: the command form, `//` escape, path-like text, uppercase, leading whitespace, multi-line arguments.
- Registry: built-in wins over plugin; first plugin wins; safe mode; shadowed commands produce log warnings.
- Target: `/stop` and `/archive` act on the composer's channel when it is not the active channel; `/archive` in a thread composer.
- Plugin API: two-argument and three-argument registration; refused names; truncated `opts` values.
- Run: unknown name; planned command; blocked built-in; plugin error; output truncation; a command message never calls `send_message`.
- Composer: popup open and close rules; keyboard rules; disabled planned rows; run during a turn; attachments refusal; draft kept on error and cleared on success; result panel dismiss.

## Left to the builder

- The names of the Tauri commands, the DTOs, and the new modules.
- The place of the built-in command definitions (`discard-core` is the expected home, beside `Action`).
- Whether the composer or `App.vue` calls the bridge.
- How to keep the plugin call off the UI thread.
- The exact layout and maximum height of the result panel.
