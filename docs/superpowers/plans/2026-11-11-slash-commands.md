# Slash Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A user types `/name` in a composer to run a built-in command or a Lua plugin command. The command never reaches the agent.

**Architecture:** `discard-core` owns the command form, the built-in definitions, and the registry merge. `discard-lua` stores a description and an argument hint for each plugin command. `discard-app` exposes two Tauri commands (`list_commands`, `run_command`) and dispatches built-in commands through the existing `Action` path. `Composer.vue` intercepts a command message before the send, shows a command popup, and shows a result panel.

**Tech Stack:** Rust (Tauri 2, mlua), Vue 3 + TypeScript, Vitest through `vp test`, pnpm.

**Spec:** `docs/superpowers/specs/2026-11-11-slash-commands-design.md`. Decision numbers in this plan (`D12a`) refer to it. Read it before each task.

## Global Constraints

- Command form: `^/([a-z][a-z0-9_-]{0,31})(\s|$)`, matched from the first character of the text as typed (D1).
- Plugin command name rule: `^[a-z][a-z0-9_-]{0,31}$` (D17).
- Limits: `description` 120 characters, `args` 40 characters, output 16 KB (16 384 bytes) with the suffix `… (truncated)` (D18, D38).
- Exact copy: `Unknown command: /name`, `/name is not available yet.`, `No turn is running.`, `Stop the turn before archiving the channel`, `/archive does not apply to a thread.`, `A command cannot have attachments.`, `Plugin command`, `No matching command`, `Could not load commands.`, `Dismiss command result`, `Soon`.
- A built-in command with a matching `Action` goes through `actions::run`. No second handler (D12).
- Planned commands (`/model`, `/clear`) have no handler. Each has a `TODO` comment in code that states the missing behavior. The UI label is `Soon` (`AGENTS.md`).
- No change to the transcript schema, the store, or the provider adapters.
- Web tests go in a `__tests__` directory beside the code they test.
- The TypeScript and Rust copies of the command form each have a comment that names the other.
- No Claude attribution in commits. No `Co-Authored-By` trailer.
- Commit on a feature branch, not on `main`.

## Review Focus

1. **`//` escape and the draft.** `App.vue` clears the draft only when `drafts[id].trim() === text`. After `//foo` is sent as `/foo`, the draft must clear. Test in Task 5.
2. **Enter before the registry loads.** The mention popup swallows Enter while it loads. The command popup must not: Enter on `/help` runs the command while the list is empty or loading (D52, D53). Test in Task 6.
3. **Markup in plugin output.** Output such as `<img src=x onerror=alert(1)>` or `**bold**` shows as literal text (D37). Test in Task 5.
4. **Channel switch while a command runs.** One composer instance serves many channels. A result that arrives after `channelId` changed must not show, and must not clear the draft of the new channel. Test in Task 5.
5. **Truncation inside a multi-byte character.** A cut at byte 16 384 that lands inside a UTF-8 character must not panic and must not produce invalid text. Test in Task 1.

## File Structure

| File | Responsibility |
|---|---|
| `crates/discard-core/src/slash.rs` (create) | Command form, built-in definitions, registry merge, help text, output truncation. Pure. |
| `crates/discard-lua/src/runtime.rs` (modify) | `discard.command(name, run, opts?)`, name rule, `CommandInfo`. |
| `crates/discard-lua/src/manager.rs` (modify) | `commands()` list for running plugins. |
| `crates/discard-lua/src/api_spec.rs` (modify) | New signature and doc text. |
| `crates/discard-app/src/desktop/slash.rs` (create) | Registry build from core + plugins, built-in dispatch, plugin dispatch. |
| `crates/discard-app/src/desktop/actions.rs`, `core.rs`, `commands.rs`, `mod.rs`, `dto.rs` (modify) | Channel target for `CancelTurn`, core helpers, Tauri commands, DTO. |
| `web/src/components/slash.ts` (create) | TypeScript copy of the command form, popup query, filter. Pure. |
| `web/src/bridge/api.ts`, `types.ts` (modify) | `getCommands`, `runCommand`, `CommandDto`. |
| `web/src/components/Composer.vue` (modify) | Intercept, result panel, command popup. |
| `web/src/App.vue`, `components/workspace/ThreadPanel.vue` (modify) | Pass `channelId`; apply the `//` escape before the send. |

---

### Task 1: Core command model

**Files:**
- Create: `crates/discard-core/src/slash.rs`
- Modify: `crates/discard-core/src/lib.rs` (add `pub mod slash;`)
- Test: unit tests in `crates/discard-core/src/slash.rs`

**Interfaces:**
- Consumes: `discard_core::action::Action`.
- Produces:

```rust
pub const OUTPUT_LIMIT: usize = 16 * 1024;
pub const DESCRIPTION_LIMIT: usize = 120; // characters
pub const ARGS_LIMIT: usize = 40;         // characters

/// Keep aligned with `parseSlash` in web/src/components/slash.ts.
pub fn valid_name(name: &str) -> bool;
pub fn parse(text: &str) -> Option<(&str, &str)>; // (name, trimmed args)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin { Help, Stop, Settings, Members, Plugins, Archive, Model, Clear }
impl Builtin {
    pub const ALL: [Builtin; 8];                 // in the order of the D12 table
    pub fn name(self) -> &'static str;           // "help", "stop", ...
    pub fn description(self) -> &'static str;
    pub fn action(self) -> Option<Action>;       // None for Help, Model, Clear
    pub fn planned(self) -> bool;                // true for Model, Clear
    pub fn from_name(name: &str) -> Option<Builtin>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCommand { pub plugin: String, pub name: String, pub description: Option<String>, pub args: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry { pub name: String, pub description: String, pub args: Option<String>, pub source: String, pub planned: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadowed { pub plugin: String, pub name: String, pub by: String } // `by` is "builtin" or the winning plugin

/// `plugins` is in load order. Built-ins come first in `ALL` order, then plugin commands in alphabetical order.
pub fn registry(plugins: &[PluginCommand]) -> (Vec<Entry>, Vec<Shadowed>);
pub fn help_text(entries: &[Entry]) -> String;
pub fn truncate_output(text: String) -> String;
```

Built-in table (name, description, action, planned):

| Name | Description | Action | Planned |
|---|---|---|---|
| `help` | `Show the available commands` | none | no |
| `stop` | `Stop the running turn` | `CancelTurn` | no |
| `settings` | `Open Settings` | `OpenSettings` | no |
| `members` | `Show or hide the member list` | `ToggleMemberList` | no |
| `plugins` | `Review plugins` | `ReviewPlugins` | no |
| `archive` | `Archive this channel` | `ArchiveChannel` | no |
| `model` | `Change the model of the agent` | none | yes |
| `clear` | `Start a new agent session` | none | yes |

The `Model` and `Clear` definitions each get a comment: `// TODO: <missing behavior>` (change the model of the channel's agent; start a new agent session in the channel).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn parses_the_command_form() {
    assert_eq!(parse("/help"), Some(("help", "")));
    assert_eq!(parse("/echo  a b \n c "), Some(("echo", "a b \n c")));
    assert_eq!(parse("/stop\nnow"), Some(("stop", "now")));
}
#[test] fn rejects_text_that_is_not_a_command() {
    for text in ["", "/", "//help", " /help", "/Help", "/usr/bin/env", "/1x", "hello /help", &format!("/{}", "a".repeat(33))] {
        assert_eq!(parse(text), None, "{text}");
    }
    assert!(parse(&format!("/{}", "a".repeat(32))).is_some());
}
#[test] fn valid_name_matches_the_plugin_rule() {
    assert!(valid_name("a") && valid_name("git-log_2"));
    for name in ["", "A", "1a", "a b", "a/b", &"a".repeat(33)] { assert!(!valid_name(name), "{name}"); }
}
#[test] fn builtins_are_in_table_order_and_planned_ones_have_no_action() {
    let names: Vec<_> = Builtin::ALL.iter().map(|b| b.name()).collect();
    assert_eq!(names, ["help", "stop", "settings", "members", "plugins", "archive", "model", "clear"]);
    for b in Builtin::ALL { if b.planned() { assert_eq!(b.action(), None); } }
    assert_eq!(Builtin::Stop.action(), Some(Action::CancelTurn));
    assert_eq!(Builtin::from_name("archive"), Some(Builtin::Archive));
    assert_eq!(Builtin::from_name("nope"), None);
}
#[test] fn registry_merges_with_builtin_and_first_plugin_precedence() {
    let p = |plugin: &str, name: &str| PluginCommand { plugin: plugin.into(), name: name.into(), description: None, args: None };
    let (entries, shadowed) = registry(&[p("b", "zed"), p("a", "help"), p("a", "zed"), p("a", "apple")]);
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["help", "stop", "settings", "members", "plugins", "archive", "model", "clear", "apple", "zed"]);
    assert_eq!(entries.last().unwrap().source, "b");
    assert_eq!(entries.last().unwrap().description, "Plugin command");
    assert_eq!(entries[0].source, "builtin");
    assert!(entries[6].planned && !entries[0].planned);
    assert_eq!(shadowed, [
        Shadowed { plugin: "a".into(), name: "help".into(), by: "builtin".into() },
        Shadowed { plugin: "a".into(), name: "zed".into(), by: "b".into() },
    ]);
}
#[test] fn help_lists_every_entry() {
    let (entries, _) = registry(&[PluginCommand { plugin: "p".into(), name: "echo".into(), description: Some("Repeat".into()), args: Some("<text>".into()) }]);
    let text = help_text(&entries);
    assert!(text.contains("/help — Show the available commands"));
    assert!(text.contains("/model — Change the model of the agent (Soon)"));
    assert!(text.contains("/echo <text> — Repeat (p)"));
    assert_eq!(text.lines().count(), 9);
}
#[test] fn truncates_output_on_a_character_boundary() {
    assert_eq!(truncate_output("short".into()), "short");
    let long = "é".repeat(OUTPUT_LIMIT); // 2 bytes each
    let out = truncate_output(format!("x{long}")); // the cut lands inside an é
    assert!(out.ends_with("… (truncated)"));
    assert!(out.len() <= OUTPUT_LIMIT + "… (truncated)".len());
    assert_eq!(truncate_output("a".repeat(OUTPUT_LIMIT)).len(), OUTPUT_LIMIT);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p discard-core slash`
Expected: compile error, `slash` module not found.

- [ ] **Step 3: Implement `slash.rs` to the Interfaces block**

`parse` is a hand-written scan, not a regex dependency. `help_text` line format: `/name[ args] — description[ (Soon)][ (plugin)]`, one line per entry. `truncate_output` cuts at the last `char` boundary at or below `OUTPUT_LIMIT` (`str::floor_char_boundary` or a manual loop).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p discard-core slash`
Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/discard-core/src/slash.rs crates/discard-core/src/lib.rs
git commit -m "feat: add the slash command model"
```

---

### Task 2: Plugin command metadata

**Files:**
- Modify: `crates/discard-lua/src/runtime.rs` (`LoadedInfo` near line 66, `api.set("command", …)` near line 612, `LoadedInfo` build near line 671)
- Modify: `crates/discard-lua/src/manager.rs` (`run_command` near line 166, add `commands`)
- Modify: `crates/discard-lua/src/service.rs` (`TestReport.commands` stays `Vec<String>`; map names)
- Modify: `crates/discard-lua/src/api_spec.rs:33`
- Modify: `crates/discard-lua/Cargo.toml` if `discard-core` is not yet a dependency (check first; if a dependency cycle results, copy the name rule into `runtime.rs` with a comment that names `discard_core::slash::valid_name`)
- Test: `crates/discard-lua/tests/runtime.rs`, unit tests in `manager.rs`

**Interfaces:**
- Consumes: `discard_core::slash::{valid_name, DESCRIPTION_LIMIT, ARGS_LIMIT}`.
- Produces:

```rust
// runtime.rs
#[derive(Debug, Clone, PartialEq)]
pub struct CommandInfo { pub name: String, pub description: Option<String>, pub args: Option<String> }
pub struct LoadedInfo { /* … */ pub commands: Vec<CommandInfo>, /* … */ } // was Vec<String>; sorted by name

// manager.rs
/// Commands of every plugin with the status `Running`, in load order. `(plugin name, command)`.
pub fn commands(&self) -> Vec<(String, CommandInfo)>;
```

Lua: `discard.command(name: string, run: (args: string) -> string?, opts: { description: string?, args: string? }?)`.

- [ ] **Step 1: Write the failing tests** (use the existing load helper of each test file)

```rust
// tests/runtime.rs
#[test] fn command_registration_keeps_description_and_args() {
    let h = load(r#"
        discard.command("echo", function(a) return a end, { description = "Repeat", args = "<text>" })
        discard.command("bare", function() end)
    "#);
    assert_eq!(h.info().commands, [
        CommandInfo { name: "bare".into(), description: None, args: None },
        CommandInfo { name: "echo".into(), description: Some("Repeat".into()), args: Some("<text>".into()) },
    ]);
    assert_eq!(h.run_command("echo", "hi").unwrap().as_deref(), Some("hi"));
}
#[test] fn command_registration_truncates_long_opts() {
    let h = load(&format!(r#"discard.command("x", function() end, {{ description = "{}", args = "{}" }})"#, "d".repeat(200), "a".repeat(80)));
    let c = &h.info().commands[0];
    assert_eq!(c.description.as_ref().unwrap().chars().count(), 120);
    assert_eq!(c.args.as_ref().unwrap().chars().count(), 40);
}
#[test] fn command_registration_refuses_an_invalid_name() {
    for name in ["Bad", "1x", "a b", ""] {
        let error = try_load(&format!(r#"discard.command("{name}", function() end)"#)).unwrap_err().to_string();
        assert!(error.contains("command name"), "{error}");
    }
}

// manager.rs
#[test] fn commands_lists_only_running_plugins_in_load_order() {
    // two plugins: "b" then "a", each with one command; terminate "a".
    // assert_eq!(m.commands().iter().map(|(p, c)| (p.as_str(), c.name.as_str())).collect::<Vec<_>>(), [("b", "…")]);
}
```

Adapt `load`, `try_load`, and `info()` to the helpers that the test files already have. Write the manager test body with the plugin fixture helpers of the existing `manager.rs` tests.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p discard-lua command`
Expected: compile error, `CommandInfo` not found.

- [ ] **Step 3: Implement**

- `api.set("command", …)` takes `(String, Function, Option<Table>)`. Refuse an invalid name with `mlua::Error::runtime("invalid command name \"…\": use a-z, 0-9, _ and -, start with a letter, 32 characters at most")`. Store metadata in a side `Arc<Mutex<Vec<CommandInfo>>>`, as `menu_info` does. A second registration of the same name replaces the first.
- Truncate by characters, not bytes.
- `manager.run_command` compares `c.name == name`. It also requires the status `Running`.
- `api_spec.rs`: signature `(name: string, run: (args: string) -> string?, opts: { description: string?, args: string? }?) -> ()`; doc `Register a slash command. The user runs it with /name in a composer. The name uses a-z, 0-9, _ and -, and starts with a letter. The returned text is shown to the user.`
- Fix every other compile error from the `commands` type change (`service.rs` `TestReport`, existing tests).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p discard-lua`
Expected: all pass, including existing tests.

- [ ] **Step 5: Commit**

```bash
git add crates/discard-lua
git commit -m "feat: store a description and an argument hint for plugin commands"
```

---

### Task 3: App registry and run path

**Files:**
- Create: `crates/discard-app/src/desktop/slash.rs`
- Modify: `crates/discard-app/src/desktop/mod.rs` (`mod slash;`, add both commands to `generate_handler!` near line 247)
- Modify: `crates/discard-app/src/desktop/actions.rs:54-80` (`run`)
- Modify: `crates/discard-app/src/desktop/core.rs` (new helpers beside `plugin_action` near line 983 and `cancel_in` near line 1185)
- Modify: `crates/discard-app/src/desktop/commands.rs`, `dto.rs`
- Test: unit tests in `slash.rs`; extend `crates/discard-app/src/plugin_events.rs` tests only if its helper is the easiest plugin fixture

**Interfaces:**
- Consumes: Task 1 `slash::*`; Task 2 `PluginManager::commands`, `PluginManager::run_command`.
- Produces:

```rust
// dto.rs — serde camelCase, like the other DTOs
pub struct CommandDto { pub name: String, pub description: String, pub args: Option<String>, pub source: String, pub planned: bool }

// core.rs
/// The state of a channel a command can target: the active channel or a loaded thread.
pub struct CommandChannel { pub turn_running: bool, pub archived: bool, pub is_thread: bool }
pub fn command_channel(&self, channel_id: &str) -> Option<CommandChannel>;
pub fn plugin_commands(&self) -> Vec<discard_core::slash::PluginCommand>; // empty when plugins are unavailable
/// Run a plugin command in the memory context (agent, project) of `channel_id`. `None` when no running plugin has the command.
pub fn run_plugin_command(&self, channel_id: &str, name: &str, args: &str) -> Option<Result<Option<String>, String>>;

// slash.rs
/// What a built-in command does. Pure; the Tauri command executes it.
#[derive(Debug, PartialEq)]
pub enum Step { Output(String), Dispatch(Action), Error(String) }
pub fn builtin_step(builtin: Builtin, channel: &CommandChannel, entries: &[Entry]) -> Step;

// commands.rs
#[tauri::command] pub async fn list_commands(core: State<'_, Arc<AppCore>>) -> Result<Vec<CommandDto>, String>;
#[tauri::command] pub async fn run_command(app: tauri::AppHandle, core: State<'_, Arc<AppCore>>, actions: State<'_, Arc<DesktopActions>>, channel_id: String, name: String, args: String) -> Result<Option<String>, String>;
```

`builtin_step` rules:

| Builtin | Condition | Step |
|---|---|---|
| any planned | — | `Error("/name is not available yet.")` |
| `Help` | — | `Output(help_text(entries))` |
| `Stop` | `!turn_running` | `Error("No turn is running.")` |
| `Stop` | else | `Dispatch(CancelTurn)` |
| `Archive` | `is_thread` | `Error("/archive does not apply to a thread.")` |
| `Archive` | `turn_running` | `Error("Stop the turn before archiving the channel")` |
| `Archive` | else | `Dispatch(ArchiveChannel)` |
| `Settings`, `Members`, `Plugins` | — | `Dispatch(action)` |

`run_command` order (D23–D34):

1. `!slash::valid_name(&name)` → `Err(format!("Unknown command: /{name}"))`.
2. `core.command_channel(&channel_id)` is `None` → `Err("The channel is not open.")`. `archived` → `Err("The channel is archived.")`. (Designer's calls; the composer is disabled in both states.)
3. Build the registry: `slash::registry(&core.plugin_commands())`. Log one warning per `Shadowed` with the plugin, the command, and the winner (D8, D9). Use the log macro that `core.rs` already uses.
4. `Builtin::from_name` → `builtin_step`. `Dispatch(action)` calls `actions::run(&app, core.inner(), &actions, action, Some(ContextTarget::Channel { id: channel_id }))` and returns `Ok(None)`. `Output(text)` returns `Ok(Some(truncate_output(text)))`. `Error(text)` returns `Err(text)`.
5. Else `core.run_plugin_command` inside `tauri::async_runtime::spawn_blocking` (D31). `Some(Ok(text))` → `Ok(text.map(truncate_output))`. `Some(Err(e))` → find the owning plugin from the registry entry `source`, call `core.plugin_failure(plugin, &e)`, return `Err(truncate_output(e))`. `None` → `Err(format!("Unknown command: /{name}"))`.

`actions::run` change (D12a, D12b): the guard on line 55 returns `None` for a channel target that is not the active channel. Keep the guard for every action except `CancelTurn`. `Plan::CancelTurn` becomes `core.cancel_in(channel id of the target, or None)`. `cancel_in` already accepts the active channel or a loaded thread. All other callers pass no channel target for `CancelTurn`, so their behavior does not change.

`run_plugin_command` follows `plugin_action` (line 983): take agent and project from the state of `channel_id` (active channel or parked thread) instead of `state.active`, wrap the call in `PluginMemory::for_context`, then `self.clear_slots()`.

`list_commands` returns the registry entries as `CommandDto`. It does not log shadow warnings.

- [ ] **Step 1: Write the failing tests** (in `slash.rs`)

```rust
fn ch(turn_running: bool, is_thread: bool) -> CommandChannel { CommandChannel { turn_running, archived: false, is_thread } }
fn entries() -> Vec<Entry> { discard_core::slash::registry(&[]).0 }

#[test] fn planned_commands_return_the_not_available_error() {
    assert_eq!(builtin_step(Builtin::Model, &ch(false, false), &entries()), Step::Error("/model is not available yet.".into()));
    assert_eq!(builtin_step(Builtin::Clear, &ch(true, true), &entries()), Step::Error("/clear is not available yet.".into()));
}
#[test] fn stop_needs_a_running_turn() {
    assert_eq!(builtin_step(Builtin::Stop, &ch(false, false), &entries()), Step::Error("No turn is running.".into()));
    assert_eq!(builtin_step(Builtin::Stop, &ch(true, true), &entries()), Step::Dispatch(Action::CancelTurn));
}
#[test] fn archive_is_refused_in_a_thread_and_during_a_turn() {
    assert_eq!(builtin_step(Builtin::Archive, &ch(false, true), &entries()), Step::Error("/archive does not apply to a thread.".into()));
    assert_eq!(builtin_step(Builtin::Archive, &ch(true, false), &entries()), Step::Error("Stop the turn before archiving the channel".into()));
    assert_eq!(builtin_step(Builtin::Archive, &ch(false, false), &entries()), Step::Dispatch(Action::ArchiveChannel));
}
#[test] fn help_outputs_the_registry_and_the_rest_dispatch() {
    assert!(matches!(builtin_step(Builtin::Help, &ch(false, false), &entries()), Step::Output(text) if text.contains("/stop")));
    assert_eq!(builtin_step(Builtin::Settings, &ch(true, false), &entries()), Step::Dispatch(Action::OpenSettings));
    assert_eq!(builtin_step(Builtin::Members, &ch(false, true), &entries()), Step::Dispatch(Action::ToggleMemberList));
    assert_eq!(builtin_step(Builtin::Plugins, &ch(false, false), &entries()), Step::Dispatch(Action::ReviewPlugins));
}
```

Add one `core.rs` or `plugin_events.rs` test with the existing plugin fixture: a plugin registers `echo`; `core.run_plugin_command(active_id, "echo", "hi")` returns `Some(Ok(Some(..)))`; `core.run_plugin_command(active_id, "nope", "")` returns `None`; `core.plugin_commands()` has one entry with the plugin name.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p discard-app slash`
Expected: compile error, `builtin_step` not found.

- [ ] **Step 3: Implement to the Interfaces block and the rules above**

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p discard-app && cargo clippy -p discard-app -p discard-core -p discard-lua --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/discard-app
git commit -m "feat: add the list_commands and run_command bridge commands"
```

---

### Task 4: Web command form and bridge

**Files:**
- Create: `web/src/components/slash.ts`
- Modify: `web/src/bridge/api.ts`, `web/src/bridge/types.ts`
- Test: `web/src/components/__tests__/slash.test.ts`

**Interfaces:**
- Consumes: Tauri commands `list_commands`, `run_command` (Task 3).
- Produces:

```ts
// types.ts
export interface CommandDto { name: string; description: string; args: string | null; source: string; planned: boolean }

// api.ts
export const getCommands = () => invoke<CommandDto[]>('list_commands')
export const runCommand = (channelId: string, name: string, args: string) =>
  invoke<string | null>('run_command', { channelId, name, args })

// slash.ts — keep aligned with discard_core::slash::parse.
export interface SlashCommand { name: string; args: string }
export function parseSlash(text: string): SlashCommand | null
/** `//x` becomes `/x`. Other text is unchanged. */
export function unescapeSlash(text: string): string
/** The typed name part when the whole draft is `/` plus name characters; else null. */
export function slashQuery(text: string): string | null
/** Entries whose name starts with `query`, in registry order. */
export function filterCommands(entries: CommandDto[], query: string): CommandDto[]
/** `/name args` on one line for the result panel. */
export function commandLine(command: SlashCommand): string
```

- [ ] **Step 1: Write the failing tests**

```ts
import { describe, expect, it } from 'vitest'
import { commandLine, filterCommands, parseSlash, slashQuery, unescapeSlash } from '../slash'

describe('parseSlash', () => {
  it('reads the name and the trimmed arguments', () => {
    expect(parseSlash('/help')).toEqual({ name: 'help', args: '' })
    expect(parseSlash('/echo  a b \n c ')).toEqual({ name: 'echo', args: 'a b \n c' })
    expect(parseSlash('/stop\nnow')).toEqual({ name: 'stop', args: 'now' })
  })
  it.each(['', '/', '//help', ' /help', '/Help', '/usr/bin/env', '/1x', 'hello /help', '/' + 'a'.repeat(33)])(
    'does not treat %j as a command',
    (text) => expect(parseSlash(text)).toBeNull(),
  )
})
describe('unescapeSlash', () => {
  it('removes one slash from a double slash', () => {
    expect(unescapeSlash('//help me')).toBe('/help me')
    expect(unescapeSlash('///x')).toBe('//x')
    expect(unescapeSlash('/usr/bin')).toBe('/usr/bin')
    expect(unescapeSlash('a // b')).toBe('a // b')
  })
})
describe('slashQuery', () => {
  it('matches only a draft that is a partial command name', () => {
    expect(slashQuery('/')).toBe('')
    expect(slashQuery('/he')).toBe('he')
    expect(slashQuery('/he ')).toBeNull()
    expect(slashQuery('//')).toBeNull()
    expect(slashQuery('/He')).toBeNull()
    expect(slashQuery('x /he')).toBeNull()
  })
})
describe('filterCommands', () => {
  const entry = (name: string) => ({ name, description: '', args: null, source: 'builtin', planned: false })
  it('keeps registry order and filters by prefix', () => {
    const all = [entry('help'), entry('stop'), entry('settings')]
    expect(filterCommands(all, 's').map((e) => e.name)).toEqual(['stop', 'settings'])
    expect(filterCommands(all, '')).toHaveLength(3)
    expect(filterCommands(all, 'x')).toEqual([])
  })
})
describe('commandLine', () => {
  it('joins the name and the arguments on one line', () => {
    expect(commandLine({ name: 'echo', args: 'a\nb' })).toBe('/echo a b')
    expect(commandLine({ name: 'help', args: '' })).toBe('/help')
  })
})
```

- [ ] **Step 2: Run to verify failure**

Run: `cd web && pnpm test src/components/__tests__/slash.test.ts`
Expected: FAIL, cannot resolve `../slash`.

- [ ] **Step 3: Implement `slash.ts` and the two bridge functions**

- [ ] **Step 4: Run to verify pass**

Run: `cd web && pnpm test src/components/__tests__/slash.test.ts && pnpm typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/components/slash.ts web/src/components/__tests__/slash.test.ts web/src/bridge
git commit -m "feat: add the web command form and bridge calls"
```

---

### Task 5: Composer run path and result panel

**Files:**
- Modify: `web/src/components/Composer.vue` (`send()` near line 307; template stack above the input)
- Modify: `web/src/App.vue` (`<Composer>` near line 1368; `send` near line 682; `sendThread` near line 773)
- Modify: `web/src/components/workspace/ThreadPanel.vue:267`
- Test: `web/src/components/__tests__/Composer.commands.test.ts`; update mounts in `Composer.threads.test.ts`

**Interfaces:**
- Consumes: `parseSlash`, `unescapeSlash`, `commandLine`, `runCommand` (Task 4).
- Produces: new required prop on `Composer.vue`: `channelId: string` (the id of the channel or thread that the composer serves). Root element of the result panel: `data-ui-id="command-result"`.

Behavior of `send()` (D22–D33), in this order:

1. If `props.archived || props.busy || importing` → return (existing rule).
2. `const command = parseSlash(text.value)`. No trim before the parse.
3. If `command`:
   - A command already runs → return (D32).
   - Attachments present → show the error `A command cannot have attachments.` with the command line; keep the draft and the attachments; return (D28).
   - Do not apply the `props.running && !props.sendWhileRunning` rule (D26).
   - Await `runCommand(props.channelId, command.name, command.args)`.
   - If `props.channelId` changed during the wait → drop the result and do not touch the draft.
   - Success: clear the draft (emit `update:draft` as the existing code does). Output `null` or empty → close the result panel (D39). Else show the output.
   - Failure: keep the draft; show `String(error)` as an error.
   - Never emit `send`. Never change `alsoPost` or the reply (D29, D30).
4. Else the existing send path. It also closes the result panel (D41).

The send button is enabled for a command message during a turn. While a command runs, the send control shows the existing busy state (the `LoaderCircle` pattern that `importing` uses).

`//` escape (D4, Review Focus 1): `Composer.vue` emits the text as typed. `App.vue` applies `unescapeSlash` to the body it passes to `sendMessage` in `send` and in `sendThread`, before `joinReply`. The comparison `drafts[id]?.trim() === text` keeps the typed text, so the draft clears.

Result panel (D35–D43): in the stack above the input, beside the reply chip and the attachment row. Line 1: `commandLine(command)`, one line, CSS truncation. Line 2: output in an element with `white-space: pre-wrap` and text interpolation (`{{ }}`), never `v-html`, `SafeMarkdown`, or `SafeHtml`. `role="status"` for output, `role="alert"` plus the destructive color token for an error. Maximum height `max-h-48` with `overflow-y-auto`. A dismiss button (`X` icon, `aria-label="Dismiss command result"`). The result is local component state. It clears when `props.channelId` changes (D42).

- [ ] **Step 1: Write the failing tests**

Mock `../../bridge/api` as `Composer.threads.test.ts` does, with `runCommand: vi.fn()`. Mount with `channelId: 'c1'`, `archived: false`, `busy: false`, `running: false`, `channel: 'general'`. Helper `type(wrapper, text)` sets the textarea value and triggers `input`; helper `enter(wrapper)` triggers `keydown` Enter.

```ts
it('runs a command and does not send it', async () => {
  runCommand.mockResolvedValue('pong')
  await type(w, '/ping a b'); await enter(w); await flushPromises()
  expect(runCommand).toHaveBeenCalledWith('c1', 'ping', 'a b')
  expect(w.emitted('send')).toBeUndefined()
  expect(w.find('[data-ui-id="command-result"]').text()).toContain('/ping a b')
  expect(w.find('[data-ui-id="command-result"] [role="status"]').text()).toBe('pong')
  expect(w.find('textarea').element.value).toBe('')
})
it('keeps the draft and shows an alert on an error', async () => {
  runCommand.mockRejectedValue('Unknown command: /nope')
  await type(w, '/nope'); await enter(w); await flushPromises()
  expect(w.find('[data-ui-id="command-result"] [role="alert"]').text()).toBe('Unknown command: /nope')
  expect(w.find('textarea').element.value).toBe('/nope')
})
it('shows no panel for a success with no output and closes an earlier one', async () => {
  runCommand.mockResolvedValueOnce('first').mockResolvedValueOnce(null)
  await type(w, '/a'); await enter(w); await flushPromises()
  await type(w, '/b'); await enter(w); await flushPromises()
  expect(w.find('[data-ui-id="command-result"]').exists()).toBe(false)
})
it('shows markup in the output as literal text', async () => {
  runCommand.mockResolvedValue('<img src=x onerror=alert(1)> **bold**')
  await type(w, '/x'); await enter(w); await flushPromises()
  const panel = w.find('[data-ui-id="command-result"]')
  expect(panel.find('img').exists()).toBe(false)
  expect(panel.find('strong').exists()).toBe(false)
  expect(panel.text()).toContain('<img src=x onerror=alert(1)> **bold**')
})
it('runs a command while a turn runs', async () => {
  await w.setProps({ running: true }); runCommand.mockResolvedValue(null)
  await type(w, '/stop'); await enter(w); await flushPromises()
  expect(runCommand).toHaveBeenCalledWith('c1', 'stop', '')
})
it('does not run a command in an archived or busy composer', async () => {
  await w.setProps({ busy: true }); await type(w, '/help'); await enter(w)
  expect(runCommand).not.toHaveBeenCalled()
})
it('refuses a command with attachments and keeps both', async () => {
  await w.setProps({ attachments: [attachment] }); await type(w, '/help'); await enter(w)
  expect(runCommand).not.toHaveBeenCalled()
  expect(w.find('[role="alert"]').text()).toBe('A command cannot have attachments.')
  expect(w.find('textarea').element.value).toBe('/help')
})
it('does not accept a second command while one runs', async () => {
  let finish!: (v: null) => void
  runCommand.mockReturnValue(new Promise((r) => (finish = r)))
  await type(w, '/slow'); await enter(w); await enter(w)
  expect(runCommand).toHaveBeenCalledTimes(1); finish(null)
})
it('drops a result that arrives after the channel changed', async () => {
  let finish!: (v: string) => void
  runCommand.mockReturnValue(new Promise((r) => (finish = r)))
  await w.setProps({ draft: '/slow' }); await enter(w)
  await w.setProps({ channelId: 'c2', draft: 'other' }); finish('late'); await flushPromises()
  expect(w.find('[data-ui-id="command-result"]').exists()).toBe(false)
  expect(w.emitted('update:draft')?.at(-1)).not.toEqual([''])
})
it('sends text with the command form of an escaped slash as typed', async () => {
  await type(w, '//help'); await enter(w)
  expect(runCommand).not.toHaveBeenCalled()
  expect(w.emitted('send')?.[0]?.[0]).toBe('//help')
})
it('sends path-like and uppercase text as a message', async () => {
  await type(w, '/usr/bin/env'); await enter(w)
  expect(w.emitted('send')?.[0]?.[0]).toBe('/usr/bin/env')
})
it('closes the result on dismiss and on the next message', async () => {
  runCommand.mockResolvedValue('out')
  await type(w, '/x'); await enter(w); await flushPromises()
  await w.find('[aria-label="Dismiss command result"]').trigger('click')
  expect(w.find('[data-ui-id="command-result"]').exists()).toBe(false)
  await type(w, '/x'); await enter(w); await flushPromises()
  await type(w, 'hello'); await enter(w)
  expect(w.find('[data-ui-id="command-result"]').exists()).toBe(false)
})
it('keeps the reply and the also-post box for a command', async () => {
  await w.setProps({ reply, alsoPostLabel: 'general' }); runCommand.mockResolvedValue(null)
  await w.find('[role="checkbox"]').trigger('click')
  await type(w, '/help'); await enter(w); await flushPromises()
  expect(w.emitted('cancelReply')).toBeUndefined()
  expect(w.find('[role="checkbox"]').attributes('aria-checked')).toBe('true')
})
```

For the escape in `App.vue`, add a test beside the existing `App.vue` send tests in `web/src/__tests__` if a send test exists there: sending `//help` calls `sendMessage` with `/help` and clears the draft. If no `App.vue` send test exists, extract the body mapping into an exported pure function in `web/src/components/slash.ts` only if needed; `unescapeSlash` is already covered in Task 4, so verify the draft clear in Task 7 by hand.

- [ ] **Step 2: Run to verify failure**

Run: `cd web && pnpm test src/components/__tests__/Composer.commands.test.ts`
Expected: FAIL (`runCommand` not called; `send` emitted).

- [ ] **Step 3: Implement the run path, the result panel, the `channelId` prop, and the `App.vue` escape**

Pass `channelId` from `App.vue` (`store.snapshot.channelId`) and from `ThreadPanel.vue` (the open thread's channel id that `sendThread` uses).

- [ ] **Step 4: Run to verify pass**

Run: `cd web && pnpm test && pnpm check`
Expected: all pass, including `Composer.threads.test.ts` with the new prop.

- [ ] **Step 5: Commit**

```bash
git add web/src
git commit -m "feat: run slash commands from the composer"
```

---

### Task 6: Command popup

**Files:**
- Modify: `web/src/components/Composer.vue` (`keydown` near line 324; mention listbox near line 385)
- Test: `web/src/components/__tests__/Composer.commands.test.ts` (new `describe('command popup')`)

**Interfaces:**
- Consumes: `slashQuery`, `filterCommands`, `getCommands` (Task 4); the run path of Task 5.
- Produces: listbox with `aria-label="Commands"`; option rows with `role="option"`.

Rules (D44–D54):

- Open when `slashQuery(text)` is not null, the composer is not archived or busy, and the user has not dismissed the popup. Escape sets the dismissed state; the next input clears it, as for mentions.
- Each time the popup opens, call `getCommands()` (D11). Use a request token as `loadAgents` does.
- Rows: `filterCommands(entries, query)`. Each row: `/name`, the argument hint in muted text, the description, and the `source` when it is not `builtin`.
- Planned row: `aria-disabled="true"`, not selectable, label `Soon`.
- No match: one disabled row `No matching command`. Load failure: one disabled row `Could not load commands.`
- Keys while open: Up and Down move among selectable rows. Tab completes the selected row to `/name ` with the caret at the end. Enter completes when the draft differs from `/name` of the selected row; Enter runs the command (the Task 5 path) when the draft equals it. With no selectable row (no match, loading, or failure), Enter goes to the Task 5 path; it is never swallowed. Escape closes the popup and keeps the draft.
- Reuse the markup, the classes, and the position of the mention listbox. Share one `aria-activedescendant` binding: the active id comes from the open popup.
- A click on a selectable row completes it, as Tab does.

- [ ] **Step 1: Write the failing tests**

`getCommands` mock default: `[help, stop, model(planned), echo(source 'tools', args '<text>', description 'Repeat')]`.

```ts
it('opens for a partial name and lists matches in registry order', async () => {
  await type(w, '/'); await flushPromises()
  const rows = w.findAll('[aria-label="Commands"] [role="option"]')
  expect(rows.map((r) => r.text())).toEqual([
    expect.stringContaining('/help'), expect.stringContaining('/stop'),
    expect.stringContaining('/model'), expect.stringContaining('/echo'),
  ])
  expect(rows[3]!.text()).toContain('<text>'); expect(rows[3]!.text()).toContain('Repeat'); expect(rows[3]!.text()).toContain('tools')
})
it('closes after the first space', async () => {
  await type(w, '/echo '); await flushPromises()
  expect(w.find('[aria-label="Commands"]').exists()).toBe(false)
})
it('marks a planned command as Soon and skips it', async () => {
  await type(w, '/'); await flushPromises()
  const model = w.findAll('[role="option"]')[2]!
  expect(model.attributes('aria-disabled')).toBe('true'); expect(model.text()).toContain('Soon')
  await key(w, 'ArrowDown'); await key(w, 'ArrowDown') // help -> stop -> echo
  expect(w.find('[role="option"][aria-selected="true"]').text()).toContain('/echo')
})
it('completes with Tab', async () => {
  await type(w, '/st'); await flushPromises(); await key(w, 'Tab')
  expect(w.find('textarea').element.value).toBe('/stop ')
  expect(runCommand).not.toHaveBeenCalled()
})
it('completes with Enter on a partial name and runs on an exact name', async () => {
  runCommand.mockResolvedValue(null)
  await type(w, '/he'); await flushPromises(); await enter(w)
  expect(w.find('textarea').element.value).toBe('/help '); expect(runCommand).not.toHaveBeenCalled()
  await type(w, '/help'); await flushPromises(); await enter(w); await flushPromises()
  expect(runCommand).toHaveBeenCalledWith('c1', 'help', '')
})
it('runs on Enter before the registry loads', async () => {
  getCommands.mockReturnValue(new Promise(() => {})); runCommand.mockResolvedValue(null)
  await type(w, '/help'); await enter(w); await flushPromises()
  expect(runCommand).toHaveBeenCalledWith('c1', 'help', '')
})
it('shows one disabled row for no match and still runs on Enter', async () => {
  runCommand.mockRejectedValue('Unknown command: /zzz')
  await type(w, '/zzz'); await flushPromises()
  expect(w.find('[aria-label="Commands"]').text()).toBe('No matching command')
  await enter(w); await flushPromises()
  expect(w.find('[role="alert"]').text()).toBe('Unknown command: /zzz')
})
it('shows a load failure row', async () => {
  getCommands.mockRejectedValue('x'); await type(w, '/'); await flushPromises()
  expect(w.find('[aria-label="Commands"]').text()).toBe('Could not load commands.')
})
it('closes on Escape and keeps the draft', async () => {
  await type(w, '/he'); await flushPromises(); await key(w, 'Escape')
  expect(w.find('[aria-label="Commands"]').exists()).toBe(false)
  expect(w.find('textarea').element.value).toBe('/he')
})
it('reads the registry each time it opens', async () => {
  await type(w, '/'); await flushPromises(); await type(w, 'x'); await type(w, '/'); await flushPromises()
  expect(getCommands).toHaveBeenCalledTimes(2)
})
it('opens in a direct message composer', async () => {
  await w.setProps({ directMessage: true }); await type(w, '/'); await flushPromises()
  expect(w.find('[aria-label="Commands"]').exists()).toBe(true)
})
```

- [ ] **Step 2: Run to verify failure**

Run: `cd web && pnpm test src/components/__tests__/Composer.commands.test.ts`
Expected: the new `describe` fails (no `[aria-label="Commands"]`).

- [ ] **Step 3: Implement the popup to the rules above**

- [ ] **Step 4: Run to verify pass**

Run: `cd web && pnpm test && pnpm check`
Expected: all pass. The mention popup tests still pass.

- [ ] **Step 5: Commit**

```bash
git add web/src/components
git commit -m "feat: add the command popup to the composer"
```

---

### Task 7: Full verification

**Files:** none, unless a check fails.

- [ ] **Step 1: Run every check**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && (cd web && pnpm test && pnpm check)`
Expected: all pass.

- [ ] **Step 2: Check in the app**

Run: `cargo tauri dev` (Vue changes hot reload). Confirm each line:

- `/` opens the popup. `/model` and `/clear` are disabled with `Soon`.
- `/help` lists eight built-in commands in the result panel.
- `/stop` during a turn stops it. `/stop` with no turn shows `No turn is running.`
- `/settings`, `/members`, `/plugins` do the same as their menu items.
- `/archive` in a text channel starts the existing confirm flow. In a thread composer it shows `/archive does not apply to a thread.`
- `/stop` in a thread composer stops only the turn of the thread.
- `/nope` shows `Unknown command: /nope` and keeps the draft. The agent gets no message.
- `//help` sends the message `/help` to the agent, and the draft clears.
- A plugin with `discard.command("echo", function(a) return a end, { description = "Repeat", args = "<text>" })` shows in the popup with its plugin name, and `/echo hi` shows `hi`.
- In safe mode the popup has only built-in commands.

- [ ] **Step 3: Commit any fix**

```bash
git add crates web/src
git commit -m "fix: correct slash command behavior found in verification"
```
