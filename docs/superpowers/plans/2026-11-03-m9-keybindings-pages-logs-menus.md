# M9: Keybindings, Memory, Workspaces, Logs, Advanced, and the Remaining Context Menus Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the Settings window (Keybindings, Memory, Workspaces, Logs, Advanced), add the rotating redacted log file, and add context menus on the server icon, the agent row, the diff block, and the tool-call block.

**Architecture:** `discard-core` holds the pure logic: the keymap, the log file writer and log line model, the page models, and the reset rules. `discard-lua` accepts four more `discard.menu_item` targets. `discard-ui` draws the pages and menus. `discard-app` installs the log sink, binds the keys from settings, and passes the memory store to the UI.

**Tech Stack:** Rust 2024, `gpui-kit` 0.9 (`gpui-component` 0.9), `toml_edit`, `tracing`, `tracing-subscriber`, `rusqlite`, `mlua` (Luau).

**Spec:** `PLAN.md`, section "Menus and Settings" (Keybindings, Advanced, and Logs pages; context menu targets) and milestone M9. M8 left plugin-declared settings fields in no milestone. This plan leaves them out too.

## Global Constraints

- `discard-core` has no GPUI dependency. TDD-first: write the failing test before the code.
- One Rust `Action` enum. The menu bar, context menus, keybindings, and the command palette all dispatch it. No surface has its own handler.
- Rust owns all menu structure and every protected item: Safe Mode, Review Plugins, Plugin Errors, Settings, Quit. Plugins cannot remove, reorder, or replace Rust items.
- A plugin with the `ui` capability may add items to the Plugins menu and to a labeled section at the end of context menus. Each item shows the plugin name. A plugin menu callback receives only the target type and ID.
- Keybindings page: it edits the map from key to action. System keys are fixed: Cmd+Q, Cmd+, , Cmd+W, Cmd+H, Cmd+M, Cmd+C, Cmd+V, Cmd+X, Cmd+A, Cmd+Z. A conflict blocks the save and names the other action. Open Settings, Quit, and Cancel turn can be rebound but not unbound. Plugins cannot register keybindings in v1.
- Advanced page: three destructive actions: Reset settings (backs up to `settings.toml.bak`), Rebuild memory index, Clear logs. Each shows a confirmation that lists what it changes. There is no "delete all data" action.
- Logs page: Discard first adds a rotating, redacted log file in `~/.discard/logs/`, next to the stderr output. The viewer is read-only and follows the file live. It filters by level, component (provider, plugin, approval, storage, MCP, workspace), and session, turn, or plugin ID.
- `settings.toml`: `version = 1`. Discard keeps unknown keys when it writes. An invalid value falls back to its default for that key only, and the window shows the error. A bad file never blocks startup.
- Settings validation has failure tests: invalid values, unknown keys, a bad file at startup, a conflicting keybinding.
- Discard never reads or stores credentials. Every log line passes through `redact`.
- Commit messages carry no attribution lines.

## Decisions this plan makes (not in the spec)

- **Rule for each action's key** (`KeyRule` in `keymap.rs`). `Locked`: Close Window, Hide Discard, Minimize. They keep Cmd+W, Cmd+H, and Cmd+M and cannot be edited. `Required`: Quit, Open Settings, Cancel Turn. They always have one chord. The user may replace it, never remove it. `Optional`: every other action that needs no target. The user may replace, remove, or reset it. Actions that need a target (the Copy actions) have no key.
- **A reserved chord belongs to its owner for good.** The reserved chords are the eleven system keys. No action but the owner may take one of them, even after Quit or Open Settings moves to another chord. Cmd+C, Cmd+V, Cmd+X, Cmd+A, and Cmd+Z have no owner: nobody may take them. This reads "fixed" and "can be rebound" in the spec together.
- **Settings format.** A `[keybindings]` table in `settings.toml` maps an action ID to a chord string. An empty string means "unbound" and is valid only for `Optional` actions. An absent key means the default. A hand edit with a bad entry (unknown action, bad chord, wrong rule, reserved chord, or a conflict) drops only that entry and reports one problem. A default chord always wins over a custom chord in a conflict. Among custom chords that conflict, the action that comes first in `Action::all()` wins.
- **Chord text** is GPUI's format, in canonical order: `ctrl-alt-shift-cmd-<key>`, lowercase. A chord needs at least one of `ctrl`, `alt`, or `cmd`. Function keys (`f1` to `f12`) may stand alone. The key is one printable character or a name from `NAMED_KEYS`.
- **Key changes apply at the next launch.** Rebinding at run time would need `clear_key_bindings`, which also clears the input component's bindings. The page says "Applies at next launch" and offers **Relaunch** (`dispatch::relaunch_same_mode`), as the Agents page does.
- **Log rotation:** `discard.log` rotates at 5 MiB (`MAX_LOG_BYTES`) and keeps 4 older files (`discard.log.1` to `discard.log.4`), so the folder stays under 25 MiB. A log file that cannot be opened never blocks startup: stderr logging continues and one warning goes to stderr.
- **Log component** comes from the event `target`, through `component_of`. No existing log call changes. The ID filter matches the `session`, `turn`, and `plugin` fields of an event, as a case-sensitive substring.
- **Memory page is read-only.** It lists the files of each scope, shows a selected file as text, and has a **Reveal** button. Editing and deleting stay with the agents and the user's own editor. Rebuild index is on Advanced.
- **Workspaces page is read-only.** It lists projects, their channels (archived ones tagged), the workspace mode, and each worktree's branch and path. It marks a missing directory. Moving a channel to a worktree and archiving a channel other than the current one stay out of M9.
- **New context menu items.** Diff block: Copy Diff. Tool-call block: Copy Tool Input, Copy Tool Output (greyed when the call has no output yet). Agent row: Copy Agent Name, Agent Settings…. Server icon: Copy Project Name, Copy Project Path, Workspace Settings…. The two "Settings…" items are new actions that open a Settings page.
- **New plugin targets:** `diff_block`, `tool_call`, `agent`, `server`. The callback ID is the message ID plus `:` and the block index for `diff_block` and `tool_call` (as for `code_block`), the agent ID for `agent`, and the project ID for `server`.
- **Order of tasks.** The pages come first, so the two "Settings…" menu items open pages that exist.

## Review Focus

- A hand-edited `[keybindings]` table with an unknown action, a bad chord, a reserved chord, a conflict, or an empty value on a `Required` action: startup works, only the bad entries are dropped, the defaults hold, and the problems show on the General page.
- Unbinding a `Required` action, editing a `Locked` one, or taking a chord another action holds (or a system key): refused, the file stays as it was, and the message names the other action or the rule.
- The log directory is missing, read-only, or full: Discard starts, logs to stderr, and warns once. A secret never reaches the file. Rotation and Clear logs while the sink writes lose no later line and crash nothing.
- The log viewer meets a last line with no newline yet, a line that is not JSON, a very long line, or a file that shrank (rotation or Clear logs): it shows a partial line only once complete, shows bad lines as raw text, truncates long lines for display, and restarts from the top of the new file.
- Reset settings on a missing file, on a file that does not parse, and when the folder is read-only: the confirm text says what happens, a failure keeps the original file, and an older `settings.toml.bak` is replaced only after the new copy is written.
- Memory and Workspaces pages with an empty store, a missing `memory/` folder, a symlinked scope folder, or a project or worktree folder that no longer exists: no crash, no link followed, and a "missing" tag.

---

### Task 1: Keymap model and `[keybindings]` in settings

**Files:**
- Create: `crates/discard-core/src/keymap.rs`
- Modify: `crates/discard-core/src/lib.rs`, `crates/discard-core/src/action.rs`, `crates/discard-core/src/settings.rs`
- Test: in those files.

**Interfaces:**
- Produces in `action.rs`: `Action::label(self) -> &'static str` (the menu label without "…", unique per action). `default_keymap()` stays.
- Produces in `keymap.rs`:
  - `pub struct Chord(String)` with `Chord::as_str(&self) -> &str`, `PartialEq, Eq, Clone, Debug`.
  - `pub fn parse_chord(text: &str) -> Result<Chord, KeymapError>`. Returns the canonical chord.
  - `pub fn display_chord(chord: &Chord) -> String`. `cmd-shift-p` becomes `Cmd+Shift+P`.
  - `pub enum KeyRule { Locked, Required, Optional }` and `pub fn rule(action: Action) -> Option<KeyRule>`. `None` for actions that need a target.
  - `pub const RESERVED: &[(&str, Option<Action>)]`: the eleven system chords and their owner.
  - `pub struct Keymap` with `Keymap::defaults() -> Keymap`, `chord(&self, Action) -> Option<&Chord>`, `is_custom(&self, Action) -> bool`, `bindings(&self) -> Vec<(Action, Chord)>` (in `Action::all()` order).
  - `pub fn resolve(raw: &[(String, String)]) -> (Keymap, Vec<SettingsIssue>)`. `raw` holds the `[keybindings]` entries. Each issue has the key `keybindings.<id>`.
  - `pub fn check_change(map: &Keymap, action: Action, chord: Option<&str>) -> Result<Option<Chord>, KeymapError>`.
  - `pub enum KeymapError { BadChord(String), NotBindable(Action), Locked(Action), CannotUnbind(Action), Reserved { chord: String, owner: Option<Action> }, InUse { chord: String, action: Action } }` with `thiserror` messages that use `Action::label` and `display_chord`.
- Produces in `settings.rs`: `Settings.keymap: Keymap`. `Change::Keybinding { action: Action, chord: Option<String> }` (`None` unbinds and writes `""`). `Change::ResetKeybinding(Action)` (removes the key). `Keymap` derives `Debug, Clone, PartialEq, Eq, Default` (`Default` is `defaults()`).

- [ ] **Step 1: Write failing tests** in `keymap.rs`:
  - `parse_chord_puts_modifiers_in_canonical_order_and_lowercases`: `"Cmd-Shift-P"` gives `"shift-cmd-p"`; `"cmd-alt-ctrl-k"` gives `"ctrl-alt-cmd-k"`.
  - `parse_chord_accepts_a_comma_and_a_dash_as_the_key`: `"cmd-,"` and `"cmd--"` parse.
  - `parse_chord_rejects`: `""`, `"cmd-"`, `"p"`, `"shift-p"`, `"cmd-pp"`, `"cmd-cmd-p"`, `"hyper-p"`, `"cmd-f13"`. `"f5"` alone parses.
  - `display_chord_joins_capitalized_parts_with_plus`.
  - `every_action_has_a_rule_and_the_copy_actions_have_none`.
  - `defaults_match_default_keymap_and_reserved_owners`: each reserved chord with an owner is that owner's default chord.
  - `resolve_keeps_a_valid_override_and_marks_it_custom`.
  - `resolve_drops_each_bad_entry_with_one_issue`: unknown action id, bad chord, `locked` action, a reserved chord for another action, an empty value on a `Required` action. Each issue key is `keybindings.<id>`. The defaults hold.
  - `resolve_lets_a_default_chord_beat_a_custom_one`: `zoom = "cmd-."` (Cancel Turn's default) is dropped.
  - `resolve_lets_the_first_action_win_between_two_custom_chords`.
  - `check_change_refuses_each_rule`: Locked, CannotUnbind, NotBindable, Reserved with owner and without, InUse naming the other action. The message for InUse is `Cmd+K is already used by Zoom` style.
  - `check_change_allows_moving_quit_but_cmd_q_stays_reserved`: Quit to `cmd-shift-q` is `Ok`. Then `resolve` with `zoom = "cmd-q"` drops it.
  - `an_optional_action_can_be_unbound_and_a_required_one_can_be_replaced`.
- [ ] **Step 2: Write failing tests** in `settings.rs`: `keybinding_change_writes_the_table_and_keeps_comments`; `unbind_writes_an_empty_string`; `reset_keybinding_removes_the_key_and_keeps_the_table_otherwise`; `refused_keybinding_change_leaves_the_file_as_it_was` (compare bytes); `bad_keybinding_entries_become_issues_and_defaults_hold` (a `[keybindings]` that is not a table is one issue, `keybindings`); `a_hand_edit_to_a_keybinding_is_read_on_reload`. In `action.rs`: `labels_are_unique_and_not_empty`.
- [ ] **Step 3: Run** `cargo test -p discard-core keymap:: settings:: action::` and expect FAIL (compile errors).
- [ ] **Step 4: Implement** the interfaces above. `evaluate` in `settings.rs` collects the `[keybindings]` string entries and calls `keymap::resolve`. `apply` calls `check_change` first for the two new variants. Update `docs/settings.md` later (Task 10). Add `pub mod keymap;` to `lib.rs`.
- [ ] **Step 5: Run** `cargo test -p discard-core` and `cargo clippy -p discard-core --all-targets -- -D warnings`. Expect PASS and clean.
- [ ] **Step 6: Commit** `feat: add the keymap model and keybindings in settings`

---

### Task 2: Keybindings page and startup binding

**Files:**
- Create: `crates/discard-ui/src/settings/keybindings.rs`
- Modify: `crates/discard-core/src/settings_page.rs`, `crates/discard-ui/src/settings/mod.rs`, `crates/discard-ui/src/menus.rs`, `crates/discard-ui/src/lib.rs`, `crates/discard-core/src/keymap.rs`
- Test: `settings_page.rs`, `keymap.rs`, `settings/mod.rs`, `menus.rs`.

**Interfaces:**
- Consumes: Task 1.
- Produces in `keymap.rs`: `pub struct KeyRow { pub action: Action, pub rule: KeyRule, pub chord: Option<Chord>, pub default: Option<Chord>, pub custom: bool }` and `pub fn rows(map: &Keymap) -> Vec<KeyRow>`: one row per action with a rule, in `Action::all()` order.
- Produces in `settings_page.rs`: `LABEL_KEYBINDINGS_RECORD = "Record"`, `LABEL_KEYBINDINGS_RESET = "Reset"`, `LABEL_KEYBINDINGS_UNBIND = "Unbind"`. `Keybindings` is available. Its search labels are those three plus `"Shortcut"` and `"Key"`.
- Produces in `menus.rs`: `bind_keys(cx, map: &Keymap)` binds `map.bindings()` instead of `default_keymap()`.

- [ ] **Step 1: Write failing tests** in `keymap.rs`: `rows_skip_copy_actions_and_flag_custom_rows`; `rows_show_a_removed_optional_chord_as_none`. In `settings_page.rs`: `keybinding_labels_are_searchable`; update `all_has_ten_pages_…` to six available. In `settings/mod.rs`: update `search_keeps_unavailable_pages_not_clickable` to use `"workspaces"` (still unavailable).
- [ ] **Step 2: Run** `cargo test -p discard-core keymap:: settings_page::` and `cargo test -p discard-ui settings::`. Expect FAIL.
- [ ] **Step 3: Implement** `rows`, the labels, and the page flag. Implement the page: one row per `KeyRow` with the action label, the chord (`display_chord`, or "Not set"), a "Locked" tag for `Locked` rows, **Record**, **Reset** (only when `custom`), and **Unbind** (only for `Optional` rows with a chord). **Record** puts the row in capture mode. The next key press (read with the key-down handler of a focusable element; build the text with `Keystroke::unparse`) goes to `parse_chord`, then to `SettingsStore::apply(Change::Keybinding { … })`. Escape cancels capture. A `KeymapError` shows inline on the row until the next change. After a successful change, show "Applies at next launch" and a **Relaunch** button that calls `dispatch::relaunch_same_mode`. In `menus::bind_keys`, take the keymap from `settings.settings().keymap` as it is at startup.
- [ ] **Step 4: Run** `cargo build --workspace`, `cargo test --workspace`, and clippy. Expect PASS and clean.
- [ ] **Step 5: Verify** with the `run` skill. Open Settings > Keybindings. Record `cmd-shift-z` for Zoom. Expect "Applies at next launch". Click **Relaunch**. Expect Window > Zoom to show the key. Record `cmd-q` for Zoom. Expect the inline error that names Quit. Try **Unbind** on Quit. Expect no button. Edit `settings.toml` by hand to `quit = ""`. Expect a problem on the General page and Cmd+Q still quitting.
- [ ] **Step 6: Commit** `feat: add the Keybindings settings page`

---

### Task 3: Rotating redacted log file

**Files:**
- Create: `crates/discard-core/src/log_file.rs`
- Modify: `crates/discard-core/src/logging.rs`, `crates/discard-core/src/lib.rs`, `crates/discard-app/src/main.rs`
- Test: `log_file.rs`, `logging.rs`.

**Interfaces:**
- Produces in `log_file.rs`:
  - `pub const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;` `pub const KEEP_ROTATED: usize = 4;` `pub const ACTIVE_NAME: &str = "discard.log";`
  - `pub struct RotatingFile` with `RotatingFile::open(dir: &Path, max_bytes: u64, keep: usize) -> io::Result<RotatingFile>` and `impl Write`. It appends to `discard.log` (created with the directory if needed). Before a write that would pass `max_bytes`, it renames `discard.log.N` to `.N+1` (dropping the oldest), then `discard.log` to `discard.log.1`, then starts a new file. A write never splits across files.
  - `pub fn log_files(dir: &Path) -> Vec<PathBuf>`: the active file and the rotated ones that exist, oldest first.
  - `pub fn clear(dir: &Path) -> io::Result<usize>`: deletes rotated files and truncates the active file to zero length with `set_len(0)`. Returns the number of files it changed. A missing folder returns `Ok(0)`.
  - `pub struct SharedFile(Arc<Mutex<RotatingFile>>)` with `Clone`, `impl Write`, and `impl<'a> MakeWriter<'a>`.
- Produces in `logging.rs`: `pub fn init(log_dir: Option<&Path>)`. The subscriber writes JSON lines, redacted, to stderr and, when the file opens, to the file too. An open failure prints one line to stderr (`discard: log file disabled: <reason>`) and goes on with stderr only. The old `init()` is replaced.

- [ ] **Step 1: Write failing tests** in `log_file.rs` (use `tempfile`): `appends_lines_to_the_active_file`; `rotates_before_a_write_that_would_pass_the_limit_and_never_splits_a_line` (limit 100 bytes, write 3 lines of 40 bytes); `keeps_only_the_newest_rotated_files` (keep 2, write enough for 5 files, check that `.3` does not exist and `.1` holds the newest of the old lines); `log_files_lists_oldest_first`; `clear_truncates_the_active_file_and_deletes_the_rotated_ones_and_later_writes_land_at_the_start` (write, clear, write again, read: only the second write); `clear_on_a_missing_folder_is_zero`; `open_creates_the_directory`; `open_in_a_path_under_a_file_is_an_error`; `shared_file_clones_write_to_one_file_in_order` (two clones, two threads).
- [ ] **Step 2: Write failing tests** in `logging.rs`: `the_file_gets_the_redacted_line_and_never_the_secret` (build the subscriber with `RedactingMakeWriter(SharedFile)`, log `api_key=sk-live-secret-value-12345`, read the file, assert `[REDACTED]` and no secret); `a_log_dir_that_cannot_open_gives_none_and_does_not_panic` (extract `fn open_sink(dir: &Path) -> Result<SharedFile, String>` and test the error).
- [ ] **Step 3: Run** `cargo test -p discard-core log_file:: logging::` and expect FAIL.
- [ ] **Step 4: Implement.** A `Tee<A, B>` `MakeWriter` writes each event to stderr and to the file; an error from the file never fails the stderr write. In `main.rs` call `logging::init(Some(&data_dir.join("logs")))`.
- [ ] **Step 5: Run** `cargo test --workspace` and clippy. Expect PASS and clean.
- [ ] **Step 6: Verify** with the `run` skill. Start the app. Expect `~/.discard/logs/discard.log` (or `$DISCARD_HOME/logs`) with JSON lines. Run `chmod 500` on the `logs` folder, start again. Expect one `log file disabled` line on stderr and a working app.
- [ ] **Step 7: Commit** `feat: write a rotating redacted log file`

---

### Task 4: Log line model and the Logs page

**Files:**
- Create: `crates/discard-core/src/log_view.rs`, `crates/discard-ui/src/settings/logs.rs`
- Modify: `crates/discard-core/src/lib.rs`, `crates/discard-core/src/settings_page.rs`, `crates/discard-ui/src/settings/mod.rs`
- Test: `log_view.rs`, `settings_page.rs`, `settings/mod.rs`.

**Interfaces:**
- Consumes: Task 3 (`log_files`, `ACTIVE_NAME`).
- Produces in `log_view.rs`:
  - `pub enum Level { Trace, Debug, Info, Warn, Error }` (ordered, `Level::parse(&str) -> Option<Level>` on `"INFO"` and so on). `pub enum Component { Provider, Plugin, Approval, Storage, Mcp, Workspace, Other }` with `label()`.
  - `pub fn component_of(target: &str) -> Component`: `discard_providers*` is Provider; `discard_lua*`, `discard_app::plugin*` is Plugin; `discard_mcp*` is Mcp; `discard_core::approval*` is Approval; `discard_core::store*` is Storage; `discard_core::workspace*` and `discard_core::worktree*` is Workspace; the rest is Other.
  - `pub struct LogLine { pub time: String, pub level: Level, pub component: Component, pub message: String, pub session: Option<String>, pub turn: Option<String>, pub plugin: Option<String>, pub raw: String }` and `pub fn parse_line(raw: &str) -> LogLine`. It never fails. A line that is not a JSON object becomes `level: Info`, `component: Other`, `message: raw`.
  - `pub struct LogFilter { pub min_level: Level, pub component: Option<Component>, pub id: String }` with `Default` (`Info`, `None`, empty) and `matches(&self, &LogLine) -> bool`.
  - `pub const MAX_LINES: usize = 2000;` and `pub const MAX_DISPLAY_CHARS: usize = 400;` and `pub fn shorten(text: &str) -> String` (cuts at `MAX_DISPLAY_CHARS` characters and adds `…`; never cuts inside a character).
  - `pub struct LogTail` with `LogTail::open(dir: &Path) -> LogTail`, `poll(&mut self) -> Vec<String>`: the complete new lines since the last poll. It keeps a partial last line until its newline arrives. When the active file is shorter than the saved offset, or its first line differs from the saved one after a rotation, it restarts from the top of the active file. On the first poll it returns the last `MAX_LINES` lines of the newest files.
- Produces in `settings_page.rs`: `LABEL_LOG_LEVEL = "Level"`, `LABEL_LOG_COMPONENT = "Component"`, `LABEL_LOG_ID = "Session, turn, or plugin ID"`, `LABEL_LOG_FOLLOW = "Follow"`. `Logs` is available.

- [ ] **Step 1: Write failing tests** in `log_view.rs`: `parse_line_reads_a_tracing_json_event` (time, level, message, session, turn, plugin, target); `parse_line_keeps_a_line_that_is_not_json_as_raw_text`; `parse_line_survives_a_missing_fields_object`; `component_of_maps_each_target_prefix`; `filter_applies_level_component_and_id_together`; `the_id_filter_matches_session_turn_or_plugin_as_a_substring_and_is_case_sensitive`; `shorten_cuts_long_text_on_a_character_boundary` (use `é` and an emoji); `tail_returns_only_complete_lines_and_the_rest_on_a_later_poll`; `tail_restarts_after_the_file_shrinks` (truncate with `set_len(0)`); `tail_restarts_after_a_rotation` (rename active to `.1`, write a new active file); `tail_first_poll_returns_at_most_max_lines_of_the_newest_lines`; `tail_with_no_folder_returns_nothing`.
- [ ] **Step 2: Write failing tests** in `settings_page.rs`: `log_labels_are_searchable`; update the available count to seven. In `settings/mod.rs`: switch the unavailable-page test to `"memory"`.
- [ ] **Step 3: Run** `cargo test -p discard-core log_view:: settings_page::` and expect FAIL.
- [ ] **Step 4: Implement** the model. Implement the page: a row of level buttons, a row of component buttons (with "All"), a text input for the ID, a **Follow** switch, and a read-only list of the matching lines, newest at the bottom, each as `time level component message` with `shorten`. The page keeps at most `MAX_LINES` lines in memory. The existing 700 ms poll in `SettingsView::new` calls `LogTail::poll` only while the Logs page shows, and redraws when it returns lines. With **Follow** on, the list scrolls to the bottom on new lines. The window gets `discard_home` from `Shared`. No line is ever editable.
- [ ] **Step 5: Run** `cargo build --workspace`, `cargo test --workspace`, and clippy. Expect PASS and clean.
- [ ] **Step 6: Verify** with the `run` skill. Open Settings > Logs. Expect lines. Send a message in the main window. Expect new lines to appear within one second. Pick "Warn". Expect fewer lines. Type a session ID. Expect only that session's lines. Turn **Follow** off. Expect no scroll on new lines.
- [ ] **Step 7: Commit** `feat: add the Logs settings page`

---

### Task 5: Memory page

**Files:**
- Create: `crates/discard-core/src/memory_page.rs`, `crates/discard-ui/src/settings/memory.rs`
- Modify: `crates/discard-core/src/memory.rs` (make `scopes_on_disk` `pub`), `crates/discard-core/src/store.rs`, `crates/discard-core/src/lib.rs`, `crates/discard-core/src/settings_page.rs`, `crates/discard-ui/src/shared.rs`, `crates/discard-ui/src/lib.rs`, `crates/discard-ui/src/settings/mod.rs`, `crates/discard-app/src/main.rs`
- Test: `memory_page.rs`, `store.rs`, `settings_page.rs`, `settings/mod.rs`.

**Interfaces:**
- Produces in `store.rs`: `Store::list_projects(&self) -> Result<Vec<Project>>` (insert order, `ORDER BY rowid`).
- Produces in `memory_page.rs`: `pub struct FileRow { pub path: String, pub bytes: usize, pub core: bool }`; `pub struct ScopeSummary { pub scope: Scope, pub title: String, pub files: Vec<FileRow> }`; `pub fn summaries(memory: &MemoryStore, agents: &[(AgentId, AgentConfig)], projects: &[Project]) -> Vec<ScopeSummary>`. Order: Global, then projects, then agents, each in store order, then scopes that exist only on disk (title: the scope label). A scope with a listing error gets no files. A file is `core` when its path starts with `core/`.
- Produces in `shared.rs`: `Shared.memory: Arc<discard_core::memory::MemoryStore>`. `discard_ui::run` takes it as a new argument after `store`.
- Produces in `settings_page.rs`: `LABEL_MEMORY_SCOPES = "Memory scopes"`, `LABEL_MEMORY_FILES = "Memory files"`, `LABEL_MEMORY_REVEAL = "Reveal"`. `Memory` is available.

- [ ] **Step 1: Write failing tests** in `store.rs`: `list_projects_returns_projects_in_insert_order`. In `memory_page.rs` (use `MemoryStore::open_in_memory_index` on a temp dir): `summaries_list_global_project_and_agent_scopes_with_names_as_titles`; `a_file_under_core_is_marked_core`; `a_scope_on_disk_with_no_known_owner_is_listed_by_its_label`; `an_empty_store_gives_a_global_scope_with_no_files`; `a_symlinked_scope_folder_gives_no_files_and_is_not_followed` (unix only: `#[cfg(unix)]`).
- [ ] **Step 2: Write failing tests** in `settings_page.rs`: `memory_labels_are_searchable`; update the available count to eight. In `settings/mod.rs`: switch the unavailable-page test to `"workspaces"`.
- [ ] **Step 3: Run** `cargo test -p discard-core store:: memory_page:: settings_page::` and expect FAIL.
- [ ] **Step 4: Implement.** In `main.rs`, pass `Arc::clone(&memory_store)` to `discard_ui::run`. The page lists each scope with its file count, a button per file, the selected file as read-only text (read through `MemoryStore::read`; show the error text on failure; cut the shown text at 20,000 characters and say so), and a **Reveal** button for the scope folder (`cx.reveal_path`). The page reads the store when it opens and on each file click, not on every frame.
- [ ] **Step 5: Run** `cargo build --workspace`, `cargo test --workspace`, and clippy. Expect PASS and clean.
- [ ] **Step 6: Verify** with the `run` skill. Create a note through a session or by writing `memory/global/notes.md`. Open Settings > Memory. Expect the scope and file. Click the file. Expect its text. Delete the `memory` folder while the page is open and click the file. Expect an error line and no crash.
- [ ] **Step 7: Commit** `feat: add the Memory settings page`

---

### Task 6: Workspaces page

**Files:**
- Create: `crates/discard-core/src/workspace_page.rs`, `crates/discard-ui/src/settings/workspaces.rs`
- Modify: `crates/discard-core/src/store.rs`, `crates/discard-core/src/lib.rs`, `crates/discard-core/src/settings_page.rs`, `crates/discard-ui/src/settings/mod.rs`
- Test: `store.rs`, `workspace_page.rs`, `settings_page.rs`, `settings/mod.rs`.

**Interfaces:**
- Consumes: `Store::list_projects` (Task 5).
- Produces in `store.rs`: `Store::all_channels_for_project(&self, p: &ProjectId) -> Result<Vec<(Channel, bool)>>`: every channel in order, with its `archived` flag.
- Produces in `workspace_page.rs`: `pub struct WorktreeInfo { pub path: String, pub branch: String, pub exists: bool }`; `pub struct ChannelRow { pub name: String, pub mode: WorkspaceMode, pub archived: bool, pub current: bool, pub worktree: Option<WorktreeInfo> }`; `pub struct ProjectRow { pub name: String, pub path: String, pub exists: bool, pub channels: Vec<ChannelRow> }`; `pub fn rows(store: &Store, current: &ChannelId) -> Result<Vec<ProjectRow>, StoreError>`. `exists` is `Path::exists` (a symlink to nothing counts as missing).
- Produces in `settings_page.rs`: `LABEL_WORKSPACE_PROJECTS = "Projects"`, `LABEL_WORKSPACE_WORKTREE = "Worktree"`, `LABEL_WORKSPACE_REVEAL = "Reveal"`. `Workspaces` is available.

- [ ] **Step 1: Write failing tests** in `store.rs`: `all_channels_include_archived_ones_with_the_flag`. In `workspace_page.rs` (`Store::open_in_memory`, temp dirs): `rows_list_each_project_with_its_channels_and_mark_the_current_one`; `an_archived_channel_is_tagged_and_keeps_no_worktree` (after `WorkspaceService`-style archive, `worktree_for` is `None`); `a_worktree_record_shows_branch_and_path`; `a_missing_project_or_worktree_folder_is_marked_not_existing`; `an_empty_store_gives_no_rows`.
- [ ] **Step 2: Write failing tests** in `settings_page.rs`: `workspace_labels_are_searchable`; update the available count to nine. In `settings/mod.rs`: the unavailable-page test now has no page left. Replace it with a test of a new pure helper `fn entry_for(page: SettingsPage, available: bool) -> SidebarEntry` that `sidebar_entries` uses, and assert that `available: false` gives `clickable: false`.
- [ ] **Step 3: Run** `cargo test -p discard-core store:: workspace_page:: settings_page::` and `cargo test -p discard-ui settings::`. Expect FAIL.
- [ ] **Step 4: Implement.** The page shows one card per project (name, path, a "missing" tag, **Reveal**), then its channels: name, mode (`Worktree` or `Main checkout`), an "Archived" tag, a "Current" tag, and the worktree branch and path with **Reveal**. It reads the store when it opens and every 700 ms while it shows. The page has no write action.
- [ ] **Step 5: Run** `cargo build --workspace`, `cargo test --workspace`, and clippy. Expect PASS and clean.
- [ ] **Step 6: Verify** with the `run` skill, in a git repo with `--worktree`. Open Settings > Workspaces. Expect the project, the channel, and the branch. Archive the channel from the File menu. Expect the "Archived" tag within a second. Remove the project folder's worktree by hand. Expect "missing".
- [ ] **Step 7: Commit** `feat: add the Workspaces settings page`

---

### Task 7: Advanced page

**Files:**
- Create: `crates/discard-core/src/advanced.rs`, `crates/discard-ui/src/settings/advanced.rs`
- Modify: `crates/discard-core/src/settings.rs`, `crates/discard-core/src/memory.rs`, `crates/discard-core/src/lib.rs`, `crates/discard-core/src/settings_page.rs`, `crates/discard-ui/src/settings/mod.rs`
- Test: `settings.rs`, `memory.rs`, `advanced.rs`, `settings_page.rs`, `settings/mod.rs`.

**Interfaces:**
- Consumes: `log_file::clear` (Task 3), `Shared.memory` (Task 5).
- Produces in `settings.rs`: `SettingsStore::reset(&mut self) -> Result<Option<PathBuf>, SettingsError>`. It copies the file to `settings.toml.bak` (a temp file, then a rename, so an older `.bak` stays until the new copy is whole), then writes `version = 1\n` with the same atomic write as `apply`. It returns the backup path, or `None` when there was no file. On any error, the file and the in-memory state stay as they were. After success the state is the defaults and `issues()` is empty. A file that did not parse is backed up like any other.
- Produces in `memory.rs`: `MemoryStore::rebuild_index(&self) -> Result<ReconcileStats>`: deletes every row of `mem_files` and `mem_fts` under the write lock, then runs `reconcile`. `indexed` counts all files.
- Produces in `advanced.rs`: `pub enum AdvancedAction { ResetSettings, RebuildMemoryIndex, ClearLogs }` with `title()` and `button_label()`; `pub fn confirm_lines(action: AdvancedAction, home: &Path) -> Vec<String>`. `ResetSettings`: "Copies settings.toml to settings.toml.bak. Replaces an older settings.toml.bak.", "Sets general, appearance, and keybindings back to the defaults.", "Keeps your memory files, themes, plugins, and history.", "Key changes apply at next launch." `RebuildMemoryIndex`: "Rebuilds memory-index.db from the memory files.", "Does not change any memory file." `ClearLogs`: "Deletes the older log files in `<home>/logs`.", "Empties the current log file.", "Does not change settings or history." Plus `pub fn clear_logs(home: &Path) -> Result<usize, String>` (calls `log_file::clear` on `home/logs`).
- Produces in `settings_page.rs`: `LABEL_RESET_SETTINGS = "Reset settings"`, `LABEL_REBUILD_MEMORY_INDEX = "Rebuild memory index"`, `LABEL_CLEAR_LOGS = "Clear logs"`. `Advanced` is available.

- [ ] **Step 1: Write failing tests** in `settings.rs`: `reset_backs_up_then_writes_a_minimal_file`; `reset_replaces_an_older_backup`; `reset_with_no_file_returns_none_and_writes_the_minimal_file`; `reset_of_an_unparseable_file_backs_it_up_and_clears_the_issue`; `reset_in_a_read_only_folder_fails_and_keeps_the_file_and_the_state` (`#[cfg(unix)]`, `0o500` on the folder); `reset_drops_keybindings_and_unknown_keys_from_the_live_file_but_keeps_them_in_the_backup`. In `memory.rs`: `rebuild_index_reindexes_every_file_and_search_still_finds_them`; `rebuild_index_after_the_index_was_emptied_by_hand_restores_search`. In `advanced.rs`: `confirm_lines_name_the_files_each_action_changes_and_none_say_delete_all`; `clear_logs_counts_changed_files_and_a_missing_folder_is_zero`.
- [ ] **Step 2: Write failing tests** in `settings_page.rs`: `advanced_labels_are_searchable`; the available count is now ten (rename the test to `all_ten_pages_are_available`).
- [ ] **Step 3: Run** `cargo test -p discard-core settings:: memory:: advanced:: settings_page::` and expect FAIL.
- [ ] **Step 4: Implement.** The page has one card per action: a title, the `confirm_lines` text, and a button. The first click turns the button into **Confirm** with a **Cancel**; the confirm text is visible from the start. The work runs on `cx.background_spawn` for rebuild and clear. A result line shows under the card: the backup path, the counts, or the error text. Reset also takes effect in the open window (the poll redraws). There is no other action on the page.
- [ ] **Step 5: Run** `cargo build --workspace`, `cargo test --workspace`, and clippy. Expect PASS and clean.
- [ ] **Step 6: Verify** with the `run` skill. Change the theme source on Appearance. Open Advanced, click Reset settings, click Cancel. Expect no change. Click Reset settings, then Confirm. Expect `settings.toml.bak` with the old text, a minimal `settings.toml`, and the default theme. Click Rebuild memory index. Expect a count. Click Clear logs. Expect the Logs page to show no old lines.
- [ ] **Step 7: Commit** `feat: add the Advanced settings page`

---

### Task 8: Context menu model for the new targets, and plugin targets

**Files:**
- Modify: `crates/discard-core/src/action.rs`, `crates/discard-core/src/context_menu.rs`, `crates/discard-lua/src/runtime.rs`, `crates/discard-lua/src/api_spec.rs`, `crates/discard-lua/tests/runtime.rs`
- Test: those files.

**Interfaces:**
- Produces in `action.rs`: new actions `CopyDiff`, `CopyToolInput`, `CopyToolOutput`, `CopyAgentName`, `CopyProjectName`, `CopyProjectPath` (all `needs_target`), and `OpenAgentSettings`, `OpenWorkspaceSettings` (no target). IDs are the snake case names. `MenuTarget` gains `Diff`, `ToolCall`, `Agent`, `Server` with IDs `diff_block`, `tool_call`, `agent`, `server`. The test `every_action_appears_in_the_menu_bar` becomes `every_action_appears_in_a_menu`: it accepts a menu bar entry or any context menu.
- Produces in `context_menu.rs`: `ContextTarget` gains `DiffBlock { message_id: String, block: usize }`, `ToolCall { message_id: String, block: usize, has_output: bool }`, `Agent { id: String, name: String }`, `Server { id: String, name: String, path: String }`. `kind()` and `target_id()` cover them (`target_id` is `message:block` for blocks, the ID for `Agent` and `Server`). `context_menu` returns the items listed in "Decisions". `resolve_copy` handles all copy actions: add a parameter, so the signature becomes `resolve_copy(messages: &[Message], channel_name: &str, target: &ContextTarget, action: Action) -> Option<String>`. `CopyDiff` gives the text of a `Block::Diff` (and `None` for any other block). `CopyToolInput` gives `serde_json::to_string_pretty` of the input. `CopyToolOutput` gives the output, `None` when it is absent. The three `Copy` actions that exist already keep their results.
- Produces in `discard-lua`: `discard.menu_item` accepts `diff_block`, `tool_call`, `agent`, `server` as well. The `api_spec` doc string lists all eight targets.

- [ ] **Step 1: Write failing tests** in `context_menu.rs`: `diff_block_menu_is_copy_diff_only`; `tool_call_menu_greys_copy_output_until_there_is_output`; `agent_menu_lists_copy_name_then_agent_settings`; `server_menu_lists_copy_name_copy_path_then_workspace_settings`; `plugin_items_follow_each_new_menu_after_a_separator`; `resolve_copy_finds_diff_tool_input_and_tool_output_and_none_for_the_wrong_block_type`; `target_ids_for_the_new_targets`. In `action.rs`: update `ids_round_trip` coverage and `menu_target_ids_round_trip` to the eight targets.
- [ ] **Step 2: Write failing tests** in `tests/runtime.rs`: `menu_item_accepts_the_four_new_targets` and keep `unknown_target_fails`. The `api_spec` drift tests fail until the doc string changes.
- [ ] **Step 3: Run** `cargo test -p discard-core action:: context_menu::` and `cargo test -p discard-lua`. Expect FAIL.
- [ ] **Step 4: Implement** the interfaces. In `discard-ui`, extend the exhaustive `match` in the `action_types!` list so the crate compiles (the handlers arrive in Task 9). `dispatch::plan` maps `OpenAgentSettings` to `Plan::OpenSettings(SettingsPage::Agents)` and `OpenWorkspaceSettings` to `Plan::OpenSettings(SettingsPage::Workspaces)`; the six copy actions go with the other targeted ones.
- [ ] **Step 5: Run** `cargo build --workspace`, `cargo test --workspace`, and clippy. Expect PASS and clean.
- [ ] **Step 6: Commit** `feat: add context menu items and plugin targets for diffs, tool calls, agents, and projects`

---

### Task 9: Context menus in the main window

**Files:**
- Modify: `crates/discard-ui/src/view.rs`, `crates/discard-ui/src/dispatch.rs`, `crates/discard-ui/src/lib.rs`, `crates/discard-app/src/main.rs`, `crates/discard-ui/src/menus.rs`
- Test: `dispatch.rs`, `menus.rs`.

**Interfaces:**
- Consumes: Task 8.
- Produces: `ViewData` gains `agent_id: String`, `project_id: String`, `project_path: String`. `main.rs` fills them from `workspace`.

- [ ] **Step 1: Write failing tests** in `dispatch.rs`: `the_new_copy_actions_need_a_target` (extend `copy_actions_need_a_target`); `open_agent_and_workspace_settings_open_their_pages` (`plan(..)` returns `Plan::OpenSettings` for each page); `plan_targeted_copies_for_every_copy_action`. In `menus.rs`: `plugin_items_for_the_new_targets_are_kept_out_of_the_app_table` (the Plugins menu lists only `app` items).
- [ ] **Step 2: Run** `cargo test -p discard-ui dispatch:: menus::` and expect FAIL.
- [ ] **Step 3: Implement.** In `view.rs`, give the server avatar (`server_rail`), the agent row (`member_list`), the `Block::Diff` element, and the `Block::ToolCall` element an element ID and `.context_menu(...)` built from `context_menu(&target, &self.menu_ctx)`. `render_block` builds the `DiffBlock` and `ToolCall` targets from the message ID and block index, as for code blocks. The live row passes no target and adds no menu. A right click on a diff or tool-call block shows only that block's menu: stop propagation as the code block handler does. In `dispatch.rs`, pass the `action` to `resolve_copy`.
- [ ] **Step 4: Run** `cargo build --workspace`, `cargo test --workspace`, and clippy. Expect PASS and clean.
- [ ] **Step 5: Verify** with the `run` skill. Right-click a diff block: Copy Diff, then paste in the composer. Right-click a tool-call block: Copy Tool Input shows JSON. While a call still runs, Copy Tool Output is greyed. Right-click the agent row: Copy Agent Name, and Agent Settings… opens Settings on Agents. Right-click the server icon: Copy Project Name, Copy Project Path, and Workspace Settings… opens Workspaces. Install a plugin (`--builder`) that calls `discard.menu_item` for each of the four new targets. Expect its item last in each menu, labeled `Label (name)`. Click it. Expect its callback to run with `{ kind, id }`.
- [ ] **Step 6: Commit** `feat: add context menus on the server icon, agent row, diff block, and tool-call block`

---

### Task 10: Docs and final check

**Files:**
- Modify: `docs/settings.md`, `docs/plugins.md`, `docs/memory.md`, `PLAN.md`, `README.md` (only if it lists milestones or pages)

- [ ] **Step 1: Update `docs/settings.md`.** Add `[keybindings]` to the file example and the key table (format, empty value, the rules). Add the new problems to "A bad file never stops startup". Replace "You cannot change any key yet" and the "Keybindings page is not built" sentence. Add "Default keys" rules: Locked, Required, Optional, reserved chords, and "Applies at next launch". Update "Pages that work now" to all ten and delete the "Soon" sidebar text. Add sections for the Keybindings, Memory, Workspaces, Logs, and Advanced pages with the rules from this plan. Update "Context menus": add the four new targets and their items. Remove the limits "A log file is planned for M9", "You cannot change any key yet", and "The Keybindings, Memory, Workspaces, Logs, and Advanced pages arrive in M9". Add the new limits: key changes need a relaunch, the Workspaces and Memory pages are read-only, and plugin-declared settings are not built.
- [ ] **Step 2: Update `docs/plugins.md`** with the four new `discard.menu_item` targets and their callback IDs. Update `docs/memory.md` with a pointer to the Memory page and to Rebuild memory index.
- [ ] **Step 3: Update `PLAN.md`.** Under M9, no change to the list. Add one line under "Settings window" that plugin-declared settings are still unbuilt and are in no milestone.
- [ ] **Step 4: Run** `cargo build --workspace`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`. Expect PASS and clean.
- [ ] **Step 5: Verify** with the `run` skill, end to end: open each of the ten Settings pages from the sidebar. Expect no "Soon" rows. Search "shortcut". Expect Keybindings. Search "log". Expect Logs. Quit with Cmd+Q. Start again. Expect the log file to hold both runs.
- [ ] **Step 6: Commit** `docs: describe the M9 pages, keybindings, logs, and context menus`
