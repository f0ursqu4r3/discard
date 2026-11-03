# M7: Actions, Settings, Menu Bar, Settings Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one `Action` layer, a validated `SettingsStore`, a macOS menu bar, and a Settings window with General and Plugins pages. The plugin panel moves into Settings.

**Architecture:** `discard-core` holds the pure logic: `Action`, the menu model, the settings file, and the page list. It has no GPUI. `discard-ui` converts the menu model to GPUI menus and draws a second window. `discard-app` wires the store and relaunches the process for safe mode.

**Tech Stack:** Rust 2024, `gpui-kit` 0.9 (GPUI 0.3.11 `set_menus`, `on_action`, `bind_keys`), `toml_edit` (new direct dependency, already in `Cargo.lock` through `toml`).

**Spec:** `PLAN.md`, section "Menus and Settings", and milestone M7.

## Global Constraints

- `discard-core` has no GPUI dependency. TDD-first: write the failing test before the code.
- Rust owns all menu structure and every protected item: Safe Mode, Review Plugins, Plugin Errors, Settings, Quit. Plugins cannot change them. (Plugin menu items are M8.)
- Menu items with no backend show greyed, tagged "Soon", with a tooltip and no handler. One Rust list holds them.
- Settings: separate OS window, single instance, Cmd+,. Sidebar plus search. Deep links open a page.
- `~/.discard/settings.toml` has `version = 1`. Discard keeps unknown keys on write. An invalid value falls back to its default for that key only, and the window shows the error. A bad file never blocks startup. Changes apply live.
- System keys stay fixed: Cmd+Q, Cmd+, , Cmd+W, Cmd+H, Cmd+M, Cmd+C, Cmd+V, Cmd+X, Cmd+A, Cmd+Z.
- A Rust-rendered banner and badge stay in the main window for a pending review or a plugin problem. The full plugin list, review cards, trust toggle, rollback, and error list move to Settings > Plugins.
- Safe mode relaunches the app with or without `--safe-mode`. Plugins load once at startup.
- Commit messages carry no attribution lines.

## Decisions this plan makes (not in the spec)

- General page fields: `general.default_provider` (applies at next launch) and `general.confirm_while_working` (confirm Quit and Safe Mode relaunch while a turn runs). "Reopen last channel" shows as a "Soon" row.
- Cancel turn gets the default key Cmd+. .
- Settings uses `toml_edit`, so a write keeps comments and formatting.
- Plugin-declared settings fields (`PLAN.md`, Q10) are in no milestone. This plan leaves them out. Recommend M8.

## Review Focus

- `settings.toml` is not valid TOML: the app starts with defaults, the window shows the problem, and the first write saves the bad text to `settings.toml.invalid`.
- A key has the wrong type (`confirm_while_working = "yes"`) or an unknown provider: only that key falls back. Other keys keep their values.
- A write must keep unknown keys, comments, and a future `[plugins.<name>]` table.
- The data directory cannot be written: `apply` returns an error, memory state does not change, nothing panics.
- Settings opens twice, or a banner deep-links while it is open: one window exists, and it switches page.
- Relaunch arguments: `--safe-mode` never appears twice, and `--provider codex` stays.

---

### Task 1: Action layer and menu model

**Files:**
- Create: `crates/discard-core/src/action.rs`
- Modify: `crates/discard-core/src/lib.rs` (add `pub mod action;`)

**Interfaces:**
- Produces:
  - `enum Action { OpenSettings, Quit, CancelTurn, ReviewPlugins, ShowPluginErrors, ToggleSafeMode }` (`Debug, Clone, Copy, PartialEq, Eq, Hash`) with `fn id(self) -> &'static str`, `fn from_id(&str) -> Option<Action>`, `fn all() -> &'static [Action]`.
  - `struct FutureItem { label: &'static str, tooltip: &'static str }` and `const FUTURE_ITEMS: &[FutureItem]` with: "New Channel…", "Archive Channel…", "Switch Channel…", "Discard Documentation".
  - `enum StandardItem { Undo, Redo, Cut, Copy, Paste, SelectAll }`.
  - `enum MenuEntry { Action { label: String, action: Action, checked: bool, enabled: bool }, Standard(StandardItem), Future(&'static FutureItem), Separator, Services }`.
  - `struct MenuSpec { title: &'static str, entries: Vec<MenuEntry> }`.
  - `struct MenuContext { safe_mode: bool, turn_running: bool, pending_reviews: usize }` (`Clone, PartialEq, Eq, Default`).
  - `fn menu_bar(ctx: &MenuContext) -> Vec<MenuSpec>`: titles in order `Discard, File, Edit, View, Plugins, Window, Help`.
  - `fn default_keymap() -> &'static [(Action, &'static str)]`: OpenSettings `cmd-,`, Quit `cmd-q`, CancelTurn `cmd-.`.
  - `fn needs_confirm(action: Action, turn_running: bool, confirm_while_working: bool) -> bool`.

Menu content: Discard = Settings…, Services, Quit. File = New Channel… (future), Archive Channel… (future), Cancel Turn. Edit = the six standard items. View = Switch Channel… (future). Plugins = Review Plugins…, Plugin Errors…, Safe Mode. Window = empty (the OS fills it). Help = Discard Documentation (future).

- [ ] **Step 1: Write failing tests** in `action.rs` (`#[cfg(test)]`):
  - `titles_are_in_order`
  - `every_action_appears_in_the_menu_bar` (each `Action::all()` item occurs in at least one `MenuEntry::Action`)
  - `ids_round_trip` and `from_id("nope") == None`
  - `cancel_turn_enabled_only_while_a_turn_runs`
  - `safe_mode_entry_is_checked_when_safe_mode_is_on`
  - `review_label_counts_pending`: `"Review Plugins…"` at 0, `"Review Plugins (2)…"` at 2
  - `future_items_are_in_the_menu_as_future_entries` (all four `FUTURE_ITEMS` appear as `MenuEntry::Future`)
  - `keymap_has_no_duplicate_keys`
  - `confirm_only_for_quit_and_safe_mode_while_working`: true for `Quit` and `ToggleSafeMode` only when `turn_running && confirm_while_working`; false for every other case and action
- [ ] **Step 2: Run** `cargo test -p discard-core action::` — expect FAIL (module missing).
- [ ] **Step 3: Implement** the interfaces above in `action.rs`. `menu_bar` is a pure function.
- [ ] **Step 4: Run** `cargo test -p discard-core action::` — expect PASS. Run `cargo clippy -p discard-core --all-targets -- -D warnings` — expect clean.
- [ ] **Step 5: Commit** `feat: add the Action layer and menu model`

---

### Task 2: SettingsStore

**Files:**
- Create: `crates/discard-core/src/settings.rs`
- Modify: `crates/discard-core/src/lib.rs` (add `pub mod settings;`), `crates/discard-core/Cargo.toml` (add `toml_edit` with the version already in `Cargo.lock`), root `Cargo.toml` (`[workspace.dependencies]`)

**Interfaces:**
- Consumes: `crate::bootstrap::PROVIDERS`.
- Produces:
  - `struct Settings { general: GeneralSettings }`, `struct GeneralSettings { default_provider: String, confirm_while_working: bool }`. Defaults: `"claude"`, `true`.
  - `struct SettingsIssue { key: String, message: String }` (`key` is `"general.default_provider"`, or `"file"` for a parse failure, or `"version"`).
  - `enum Change { DefaultProvider(String), ConfirmWhileWorking(bool) }`.
  - `enum SettingsError { Invalid(String), Io(String) }` (`thiserror`).
  - `struct SettingsStore` with:
    - `fn open(path: &Path) -> SettingsStore` (never fails)
    - `fn settings(&self) -> &Settings`
    - `fn issues(&self) -> &[SettingsIssue]`
    - `fn apply(&mut self, change: Change) -> Result<(), SettingsError>`
    - `fn reload_if_changed(&mut self) -> bool`
  - File layout: `version = 1`, then `[general]` with the two keys.

- [ ] **Step 1: Write failing tests** in `settings.rs`, each with a `tempfile::tempdir()`:
  - `missing_file_gives_defaults_and_creates_nothing`
  - `valid_file_is_read`
  - `wrong_type_falls_back_for_that_key_only`: `confirm_while_working = "yes"` and `default_provider = "codex"` → provider `"codex"`, confirm `true`, one issue with key `general.confirm_while_working`
  - `unknown_provider_falls_back_with_issue`: `"gemini"` → `"claude"`
  - `unparseable_file_gives_defaults_and_a_file_issue`
  - `apply_writes_the_value_and_keeps_unknown_keys_and_comments`: file holds `# note`, `[plugins.demo]`, `extra = 1`; all survive
  - `apply_on_a_bad_file_saves_it_to_settings_toml_invalid_first`
  - `apply_rejects_an_unknown_provider_and_writes_nothing`
  - `apply_fails_when_the_directory_cannot_be_created`: path is `<file>/settings.toml`, where `<file>` is a regular file. State is unchanged. No panic.
  - `missing_version_is_written_as_1`; `version = 2` gives a `version` issue, still reads values, and keeps `2` on write
  - `reload_ignores_our_own_write` (returns `false`), `reload_picks_up_a_hand_edit_and_clears_the_issue` (returns `true`). Compare file bytes, not mtime.
- [ ] **Step 2: Run** `cargo test -p discard-core settings::` — expect FAIL.
- [ ] **Step 3: Implement** the interfaces. Keep the raw `toml_edit::DocumentMut` in the store and read typed values from it key by key. Write by temp file plus rename. Remember the last bytes read or written for `reload_if_changed`.
- [ ] **Step 4: Run** `cargo test -p discard-core settings::` — expect PASS. Run clippy as in Task 1.
- [ ] **Step 5: Commit** `feat: add SettingsStore with validated, comment-preserving settings.toml`

---

### Task 3: Settings pages and search

**Files:**
- Create: `crates/discard-core/src/settings_page.rs`
- Modify: `crates/discard-core/src/lib.rs` (add `pub mod settings_page;`)

**Interfaces:**
- Produces:
  - `enum SettingsPage { General, Appearance, Agents, Providers, Plugins, Memory, Workspaces, Keybindings, Logs, Advanced }` (`Debug, Clone, Copy, PartialEq, Eq`) with `fn all() -> &'static [SettingsPage]`, `fn title(self) -> &'static str`, `fn is_available(self) -> bool` (true for `General` and `Plugins` only).
  - `fn search(query: &str) -> Vec<SettingsPage>`: case-insensitive substring match over page titles and a static list of field labels. Empty query returns all pages. Results keep `all()` order.

- [ ] **Step 1: Write failing tests:**
  - `all_has_ten_pages_and_two_are_available`
  - `empty_query_returns_every_page`
  - `provider_matches_general_and_providers`: `search("provider") == [General, Providers]`
  - `rollback_matches_plugins_only`
  - `search_is_case_insensitive`
  - `no_match_returns_nothing`: `search("zzz")`
- [ ] **Step 2: Run** `cargo test -p discard-core settings_page::` — expect FAIL.
- [ ] **Step 3: Implement.** Field labels to index: General: "Default provider", "Confirm while working", "Reopen last channel". Plugins: "Safe mode", "Review", "Roll back", "Trust future edits", "Plugin problems".
- [ ] **Step 4: Run** the tests — expect PASS.
- [ ] **Step 5: Commit** `feat: add the settings page list and search`

---

### Task 4: App wiring and relaunch

**Files:**
- Create: `crates/discard-app/src/relaunch.rs`
- Modify: `crates/discard-app/src/main.rs`, `crates/discard-app/src/args.rs`, `crates/discard-ui/src/lib.rs` (signature only in this task)

**Interfaces:**
- Consumes: `SettingsStore::open`, `Settings`.
- Produces:
  - `relaunch::args(original: &[String], safe_mode: bool) -> Vec<String>`: drops every `--safe-mode`, then appends one when `safe_mode` is true.
  - `args::default_provider(env: Option<&str>, settings: &str) -> String`: a non-empty env value wins, else the settings value.
  - `discard_ui::run(..., settings: Arc<Mutex<SettingsStore>>) -> RunOutcome` with `enum RunOutcome { Quit, Relaunch { safe_mode: bool } }`. In this task `run` always returns `Quit`.

- [ ] **Step 1: Write failing tests:** `relaunch_adds_safe_mode_once`, `relaunch_removes_safe_mode`, `relaunch_keeps_provider_flag_and_order`, `relaunch_input_with_two_safe_mode_flags_yields_one`, `env_provider_beats_settings`, `empty_env_falls_to_settings`.
- [ ] **Step 2: Run** `cargo test -p discard-app` — expect FAIL.
- [ ] **Step 3: Implement.** In `main`: open `SettingsStore` at `data_dir.join("settings.toml")` before `args::parse`. Pass `Some(&default_provider(...))` as the env argument. A flag still wins inside `parse`. Log each settings issue with `warn!`. After `discard_ui::run` returns `Relaunch { safe_mode }`, spawn `std::env::current_exe()` with `relaunch::args(&argv, safe_mode)`, then return.
- [ ] **Step 4: Run** `cargo test -p discard-app` and `cargo build -p discard-app` — expect PASS.
- [ ] **Step 5: Commit** `feat: load settings at startup and support relaunch`

---

### Task 5: Settings window shell and General page

**Files:**
- Create: `crates/discard-ui/src/shared.rs`, `crates/discard-ui/src/settings/mod.rs`, `crates/discard-ui/src/settings/general.rs`
- Modify: `crates/discard-ui/src/lib.rs`, `crates/discard-ui/src/view.rs` (constructor takes `Shared`)

**Interfaces:**
- Produces:
  - `struct Shared { plugins: Arc<PluginService>, load_report: Rc<LoadReport>, settings: Arc<Mutex<SettingsStore>>, plugin_epoch: Arc<AtomicU64>, plugin_notice: Arc<Mutex<Option<String>>> }` (`Clone`).
  - `pub fn open_settings(page: SettingsPage, cx: &mut App)`: opens the window, or activates the existing one and switches its page. A `Global` holds the window handle and the `SettingsView` entity. If the handle's `update` fails, the window is closed, so open a new one.
  - `SettingsView`: sidebar from `SettingsPage::all()` (unavailable pages greyed, tagged "Soon", not clickable), a search input using `settings_page::search`, a content pane. Every 700 ms it calls `reload_if_changed` and redraws on `true`.
  - General page: issue list at the top; default provider choice from `bootstrap::PROVIDERS` with the note "Applies at next launch"; a switch for `confirm_while_working`; a greyed "Reopen last channel" row tagged "Soon". Changes go through `SettingsStore::apply`. An `Err` shows inline.

- [ ] **Step 1: Implement** the shell with a stub Plugins page that says "Moves here in Task 6". Settings opens from the channel-list button for now.
- [ ] **Step 2: Verify** with the `run` skill: Cmd+, is not wired yet, so click the button. Expect one Settings window. Click it again. Expect no second window. Change the provider and the switch. Expect `~/.discard/settings.toml` to show the new values. Edit the file by hand. Expect the page to update within 1 s. Write `confirm_while_working = "yes"`. Expect an inline issue and the app still running.
- [ ] **Step 3: Run** `cargo build -p discard-ui` and clippy — expect clean.
- [ ] **Step 4: Commit** `feat: add the Settings window with search and the General page`

---

### Task 6: Move the plugin panel into Settings

**Files:**
- Create: `crates/discard-ui/src/settings/plugins.rs`, `crates/discard-ui/src/problems.rs`
- Modify: `crates/discard-ui/src/view.rs` (remove `plugin_panel`, `review_card`, `approve_review`, `reject_review`, `rollback_plugin`, `toggle_trust`, `show_plugins`, `trusted`, `panel_error`; shrink `plugin_errors`), `crates/discard-ui/src/settings/mod.rs`

**Interfaces:**
- Produces: `problems::problem_lines(errors: &<type returned by PluginManager::errors()>, statuses: &[PluginStatus], load_report: &LoadReport) -> Vec<String>`. It is the current `plugin_errors` line logic, moved.
- The Plugins page holds the moved code: reviews, review cards, installed list, rollback, trust toggle, the problem list, and `plugin_notice`. After approve, reject, or rollback it bumps `plugin_epoch`. When `load_report.safe_mode` is true it shows "Safe mode: user plugins disabled" and a "Relaunch normally" button (wired in Task 7).
- The main window keeps `review_banner` (its button calls `open_settings(Plugins)`) and shows one line "N plugin problems" with a Details button that calls `open_settings(Plugins)`. Its 700 ms poller also compares `plugin_epoch` and runs `after_plugin_change` when it moves.

- [ ] **Step 1: Write failing tests** in `problems.rs`: `no_problems_gives_no_lines`, `safe_mode_adds_a_line`, `key_problem_adds_a_line`, `modified_outside_review_adds_a_line`, `failed_status_is_not_listed_twice_when_it_is_also_in_errors`.
- [ ] **Step 2: Run** `cargo test -p discard-ui problems::` — expect FAIL.
- [ ] **Step 3: Implement** `problems.rs`, then move the panel code. Delete the main-window panel branch in `render`.
- [ ] **Step 4: Run** `cargo test -p discard-ui` — expect PASS. Run `cargo test -p discard-lua` — expect PASS (unchanged).
- [ ] **Step 5: Verify** with the `run` skill and `--builder`: ask the agent to write a plugin. Expect the banner in the main window. Click Review. Expect Settings on Plugins, with the review card. Approve it. Expect the main window to redraw its slots within 1 s. Roll it back. Expect the same.
- [ ] **Step 6: Commit** `feat: move the plugin panel into Settings`

---

### Task 7: Menu bar, key bindings, dispatch, confirmation

**Files:**
- Create: `crates/discard-ui/src/menus.rs`, `crates/discard-ui/src/dispatch.rs`
- Modify: `crates/discard-ui/src/lib.rs`, `crates/discard-ui/src/view.rs`, `crates/discard-ui/src/settings/plugins.rs`, `crates/discard-ui/src/settings/mod.rs`

**Interfaces:**
- Consumes: `action::{Action, MenuContext, menu_bar, default_keymap, needs_confirm}`, `open_settings`.
- Produces:
  - `menus::install(ctx: &MenuContext, cx: &mut App)`: builds `gpui::Menu`s from `menu_bar(ctx)` and calls `cx.set_menus`. Uses `MenuItem::os_action` with gpui-component's `Undo, Redo, Cut, Copy, Paste, SelectAll` for `Standard` entries, `MenuItem::os_submenu(..., SystemMenuType::Services)` for `Services`, and a disabled `MenuItem::Action` (a `Soon` action with no handler) for `Future`.
  - `menus::to_gpui(entry: &MenuEntry) -> MenuItem`.
  - `dispatch::run(action: Action, cx: &mut App)`:
    - `OpenSettings` → `open_settings(General)`
    - `ReviewPlugins`, `ShowPluginErrors` → `open_settings(Plugins)`
    - `CancelTurn` → main view `cancel`
    - `Quit`, `ToggleSafeMode` → if `needs_confirm` is true, set `pending_confirm` on the main view and activate its window; otherwise act. Quit calls `cx.quit()`. Safe mode stores the `RunOutcome::Relaunch` in an `Arc<Mutex<Option<bool>>>` that `run` returns after `application().run` ends, then calls `cx.quit()`.
  - `DiscardView.pending_confirm: Option<Confirm>` (`Quit` or `Relaunch { safe_mode: bool }`), drawn by Rust as a notice with Confirm and Cancel buttons.
  - One gpui action type per core `Action` (via `actions!(discard, [...])`), each handler calls `dispatch::run`. `cx.bind_keys` uses `default_keymap()`.
  - Menus refresh: `DiscardView` keeps the last `MenuContext`. When `turn_running` or the pending-review count changes (in the update pump and the 700 ms poller), call `menus::install` again.

- [ ] **Step 1: Write failing tests** in `menus.rs`: `future_entry_becomes_a_disabled_item`, `action_entry_keeps_label_checked_and_disabled_flags`, `separator_and_services_convert`.
- [ ] **Step 2: Run** `cargo test -p discard-ui menus::` — expect FAIL.
- [ ] **Step 3: Implement** `menus.rs`, `dispatch.rs`, the action types, the confirm notice, and the refresh. Wire the Plugins page "Relaunch normally" button to `dispatch::run(ToggleSafeMode)`. Wire the channel-list "Plugins" button to `dispatch::run(ReviewPlugins)`.
- [ ] **Step 4: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` — expect PASS and clean.
- [ ] **Step 5: Verify** with the `run` skill:
  1. Menu bar shows Discard, File, Edit, View, Plugins, Window, Help. Future items are grey with "Soon".
  2. Cmd+, opens Settings. A second Cmd+, adds no window.
  3. Cancel Turn is grey while idle, and live during a turn.
  4. Plugins > Safe Mode relaunches with `--safe-mode` (check with `ps`). The Plugins page shows the safe-mode line. "Relaunch normally" restarts without the flag.
  5. During a turn, Cmd+Q shows the confirm notice. Cancel keeps the app open. With `confirm_while_working = false`, Cmd+Q quits at once.
  6. Cmd+C and Cmd+V work in the composer.
- [ ] **Step 6: Commit** `feat: add the menu bar, key bindings, and action dispatch`

---

### Task 8: Docs

**Files:**
- Create: `docs/settings.md`
- Modify: `README.md` (add `docs/settings.md` to the Docs list)

- [ ] **Step 1: Write `docs/settings.md`**: the file path and keys, the precedence (flag, then `DISCARD_PROVIDER`, then settings), the fallback and issue rules, the comment-preserving write and the `.invalid` backup, the menu list with "Soon" items, the default keys, and the limits (default provider applies at next launch; relaunch spawns a new process after the old window closes; plugin settings and plugin menu items arrive later).
- [ ] **Step 2: Run** `cargo test --workspace` — expect PASS.
- [ ] **Step 3: Commit** `docs: describe settings, menus, and key bindings`

---

## Self-review notes

- **Spec coverage:** `Action` layer (T1), `SettingsStore` (T2), pages, search, deep links (T3, T5), startup and relaunch (T4), Settings window, single instance, General (T5), Plugins move plus banner and badge plus safe-mode page text (T6), menu bar, keys, Soon items, confirm, relaunch menu (T7). Not in M7 and not planned: plugin menu items and context menus (M8), Keybindings page (M9), plugin-declared settings (unassigned).
- **Types:** `Shared`, `SettingsPage`, `MenuContext`, `RunOutcome`, and `Change` use the same names in every task.
- **Proportion:** the plan carries signatures, test names, and exact values. It carries no function bodies.
