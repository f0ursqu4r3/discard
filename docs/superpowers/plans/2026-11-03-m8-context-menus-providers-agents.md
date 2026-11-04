# M8: Context Menus, Plugin Menu Items, Providers and Agents Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add context menus on message, code block, and channel row. Let plugins add menu items. Add the Providers and Agents pages to Settings.

**Architecture:** `discard-core` holds the pure logic: new actions, the context menu model, agent edit rules, and the provider page model. `discard-lua` gets one new API function, `discard.menu_item`. `discard-ui` draws the menus and pages and runs targeted actions through `dispatch`. `discard-app` passes provider status and the workspace service to the UI.

**Tech Stack:** Rust 2024, `gpui-kit` 0.9 (`gpui-component` 0.9 `ContextMenuExt`, `PopupMenu`, `PopupMenuItem`), `mlua` (Luau).

**Spec:** `PLAN.md`, section "Menus and Settings", and milestone M8. The Appearance page of M8 shipped in phases 1 to 4 (commits `915f776` to `8d20454`). This plan covers the rest.

## Global Constraints

- `discard-core` has no GPUI dependency. TDD-first: write the failing test before the code.
- One Rust `Action` enum. The menu bar, context menus, keybindings, and the command palette all dispatch it. No surface has its own handler.
- Rust owns all menu structure and every protected item: Safe Mode, Review Plugins, Plugin Errors, Settings, Quit. Plugins cannot remove, reorder, or replace Rust items.
- A plugin with the `ui` capability may add items to the Plugins menu and to a labeled section at the end of context menus. Each item shows the plugin name. No new capability is needed.
- A plugin menu callback receives only the target type and ID (for example `message`, `<id>`). It reads content through APIs it already holds.
- Archive channel uses the existing worktree data-loss check and confirmation. Discard never deletes user data without that confirmation.
- Agents page: saving a policy change shows a before and after diff. A looser policy shows a warning and needs an explicit confirm. If the provider cannot enforce a policy, Settings disables that choice and says why. A plugin or agent can never save a policy change through Settings.
- A missing provider capability is shown in the UI, not hidden. Discard never reads or stores credentials. It shows the exact command to run.
- Build order of pages: General, Plugins, Providers, Agents, Appearance. The last three of that list are this plan, except Appearance (done).
- Commit messages carry no attribution lines.

## Decisions this plan makes (not in the spec)

- Context menu targets in M8: message, code block (`Block::Code`), channel row. Diff block, tool-call block, server icon, and member or agent row stay in M9. The streaming (live) row has no message ID, so it gets no context menu.
- Menu items for a message: Copy Message Text. Code block: Copy Code. Channel row: Copy Channel Name and Archive Channel…. These are the only Rust items in M8.
- `Action` stays payload-free and `Copy`. Copy actions need a target, so they run through `dispatch::run_on(action, Some(target), cx)`. `ArchiveChannel` needs no target: the UI has one channel, so it acts on the current channel. The File menu item "Archive Channel…" turns on and leaves `FUTURE_ITEMS`. New Channel… and Switch Channel… stay "Soon".
- After a successful archive, the main window locks the composer and shows a notice with a **Relaunch** button. The engine holds the archived channel for its whole life, so a new channel needs a new process. On the next start, `ensure_workspace` makes a new channel. Archive is blocked while a turn runs.
- `ensure_workspace` reuses an existing agent with the same provider and `plugin_write` value when it must create a channel. Without this, each archive adds a duplicate agent and drops edits made on the Agents page.
- Plugin menu API: `discard.menu_item(target, label, run)`. Targets: `app` (Plugins menu), `message`, `code_block`, `channel`. At most 8 items per plugin. The label is 1 to 40 characters with no control characters. Items show as `Label (plugin-name)`.
- Agents page edits `model`, `system_prompt`, `policy`, and `plugin_write`. Name and provider are read-only. Creating agents from the UI is in no milestone and is left out. An edit applies at the next launch, because the engine reads the agent once at start. The page says so and offers **Relaunch**.
- Provider status comes from the app: it runs `check_setup` at start and on **Re-check**, on the tokio runtime. The UI never calls a provider.
- Plugin-declared settings fields (`PLAN.md`, "Plugin settings") are in no milestone. This plan leaves them out. They need a decision: M9 or later.

## Review Focus

- Archive while a turn runs: blocked with a notice. The channel, worktree, and menu state do not change.
- Archive of a worktree with uncommitted files: the confirm notice lists the items. Cancel keeps the channel and directory. Confirm removes the directory and keeps the branch.
- Sending a message after archive: the composer ignores it and no worktree is created again.
- A plugin menu callback that errors, runs over its CPU budget, or whose plugin is terminated: the user sees a notice, other plugins and Rust items still work, and a terminated plugin's items leave every menu. A label with a newline or over 40 characters fails at registration.
- Loosening a policy (or turning on `plugin.write`) on the Agents page without the explicit confirm does not save. A policy the provider cannot enforce cannot be picked or saved. Tightening a policy does not change the running session until relaunch, and the page says so.

---

### Task 1: Targeted actions, context menu model, plugin item model

**Files:**
- Modify: `crates/discard-core/src/action.rs`
- Create: `crates/discard-core/src/context_menu.rs`
- Modify: `crates/discard-core/src/lib.rs` (add `pub mod context_menu;`)

**Interfaces:**
- Produces in `action.rs`:
  - New `Action` variants: `ArchiveChannel` (`"archive_channel"`), `CopyMessageText` (`"copy_message_text"`), `CopyCode` (`"copy_code"`), `CopyChannelName` (`"copy_channel_name"`). Add them to `id`, `all`.
  - `fn Action::needs_target(self) -> bool`: true for the three `Copy*` actions only.
  - `enum MenuTarget { App, Message, CodeBlock, Channel }` (`Debug, Clone, Copy, PartialEq, Eq`) with `fn id(self) -> &'static str` returning `"app"`, `"message"`, `"code_block"`, `"channel"`, and `fn parse(&str) -> Option<MenuTarget>`.
  - `struct PluginMenuItem { plugin: String, index: usize, target: MenuTarget, label: String }` (`Debug, Clone, PartialEq, Eq`).
  - `MenuEntry::Plugin { plugin: String, index: usize, label: String }`. `label` is final: `"{label} ({plugin})"`.
  - `MenuContext` gains `archived: bool` and `plugin_items: Vec<PluginMenuItem>`.
  - `FUTURE_ITEMS` loses "Archive Channel…" (length 3).
- Produces in `context_menu.rs`:
  - `enum ContextTarget { Message { id: String, has_text: bool }, CodeBlock { message_id: String, block: usize }, Channel { id: String } }` (`Debug, Clone, PartialEq, Eq`) with `fn kind(&self) -> MenuTarget` and `fn target_id(&self) -> String` (`CodeBlock` gives `"{message_id}:{block}"`).
  - `fn context_menu(target: &ContextTarget, ctx: &MenuContext) -> Vec<MenuEntry>`.
  - `fn message_text(blocks: &[Block]) -> String`: text of `Markdown`, `Code`, and `Diff` blocks, joined by a blank line. `ToolCall` and `Approval` blocks add nothing.
  - `fn resolve_copy(messages: &[Message], channel_name: &str, target: &ContextTarget) -> Option<String>`: the text a Copy action puts on the clipboard. `None` when the message or block is missing, or when the block is not `Block::Code`.

Menu content. File menu: "Archive Channel…" is an `Action` entry, enabled when `!ctx.turn_running && !ctx.archived`. Plugins menu: the four Rust entries stay as they are, then (only when `target == App` items exist) a `Separator` and one `MenuEntry::Plugin` per `App` item, in the order given. `context_menu`: Message gives "Copy Message Text" (enabled when `has_text`); CodeBlock gives "Copy Code"; Channel gives "Copy Channel Name", `Separator`, "Archive Channel…" (same enabled rule). Then, when plugin items for `target.kind()` exist, a `Separator` and their `MenuEntry::Plugin` entries.

- [ ] **Step 1: Write failing tests** in `action.rs`:
  - `archive_channel_is_enabled_only_when_idle_and_not_archived`: three `MenuContext` values (default, `turn_running`, `archived`). Only the default is enabled.
  - `future_items_are_new_channel_switch_channel_and_documentation`: `FUTURE_ITEMS.len() == 3` and none is "Archive Channel…". Update `future_items_are_in_the_menu_as_future_entries` to match.
  - `targeted_actions_are_the_copy_actions_and_stay_out_of_the_menu_bar`: `needs_target` is true for exactly the three `Copy*` actions. Change `every_action_appears_in_the_menu_bar` to skip them.
  - `plugin_items_follow_the_rust_items_in_the_plugins_menu`: with two `App` items and one `Message` item in the context, the Plugins menu ends with `Separator` then two `Plugin` entries. The first four entries are unchanged. The `Message` item does not appear.
  - `plugin_entry_label_carries_the_plugin_name`: label is `"Pin (notes)"` for label `Pin`, plugin `notes`.
  - `menu_target_ids_round_trip`: `parse(id())` gives the same target for all four. `parse("diff")` is `None`.
  - `ids_round_trip` keeps passing with the four new actions.
- [ ] **Step 2: Write failing tests** in `context_menu.rs`:
  - `message_menu_has_copy_and_it_is_disabled_without_text` (`has_text: false` gives `enabled: false`).
  - `code_block_menu_is_copy_code_only`.
  - `channel_menu_lists_copy_name_then_archive`, and archive is disabled while `turn_running`.
  - `plugin_items_for_other_targets_are_not_listed`: a `Channel` item does not show in the `Message` menu.
  - `plugin_items_come_last_after_a_separator_in_every_menu`.
  - `message_text_joins_text_blocks_and_skips_tool_calls_and_approvals`.
  - `resolve_copy_finds_message_code_and_channel_name`, and returns `None` for an unknown message ID, a block index out of range, a `Block::Diff`, and a `Block::Markdown` block index.
  - `code_block_target_id_joins_message_and_index`.
- [ ] **Step 3: Run** `cargo test -p discard-core action:: context_menu::` — expect FAIL (compile errors).
- [ ] **Step 4: Implement** the interfaces above. `menu_bar` and `context_menu` stay pure. Update the doc comment on `FUTURE_ITEMS` users in `docs/settings.md` later (Task 8).
- [ ] **Step 5: Run** `cargo test -p discard-core` and `cargo clippy -p discard-core --all-targets -- -D warnings` — expect PASS and clean.
- [ ] **Step 6: Commit** `feat: add the context menu model, copy and archive actions, and plugin menu items`

---

### Task 2: Agent queries and agent reuse

**Files:**
- Modify: `crates/discard-core/src/store.rs`
- Modify: `crates/discard-core/src/bootstrap.rs`

**Interfaces:**
- Produces in `Store`:
  - `fn list_agents(&self) -> Result<Vec<(AgentId, AgentConfig)>>`: insert order.
  - `fn find_agent(&self, provider: &str, plugin_write: bool) -> Result<Option<(AgentId, AgentConfig)>>`: the first match in insert order.
  - `fn update_agent(&self, id: &AgentId, cfg: &AgentConfig) -> Result<()>`: `StoreError::NotFound` when no row matches.
- `ensure_workspace`: when no existing channel matches, call `find_agent(provider, role == Builder)` and reuse its ID and config. Create an agent only when `find_agent` gives `None`.

- [ ] **Step 1: Write failing tests** in `store.rs`: `list_agents_returns_agents_in_insert_order`, `find_agent_matches_provider_and_plugin_write`, `update_agent_replaces_the_config`, `update_agent_unknown_id_is_not_found`.
- [ ] **Step 2: Write failing tests** in `bootstrap.rs`: `after_archiving_the_only_channel_a_new_run_makes_a_new_channel_and_reuses_the_agent` (call `ensure_workspace`, `archive_channel`, call again: new channel ID, same agent ID, `list_agents().len() == 1`), `reused_agent_keeps_its_edits` (change the model with `update_agent` before the second call; the second `Workspace.agent.model` shows the edit).
- [ ] **Step 3: Run** `cargo test -p discard-core store:: bootstrap::` — expect FAIL.
- [ ] **Step 4: Implement** the three store methods (`ORDER BY rowid` on `agents`) and the reuse in `ensure_workspace`.
- [ ] **Step 5: Run** `cargo test -p discard-core` and clippy — expect PASS and clean.
- [ ] **Step 6: Commit** `feat: reuse an existing agent when a channel is created again`

---

### Task 3: `discard.menu_item` in the Lua runtime

**Files:**
- Modify: `crates/discard-lua/src/runtime.rs`
- Modify: `crates/discard-lua/src/manager.rs`
- Modify: `crates/discard-lua/src/api_spec.rs`
- Test: `crates/discard-lua/tests/runtime.rs`, `crates/discard-lua/tests/capabilities.rs`

**Interfaces:**
- Produces:
  - Lua: `discard.menu_item(target: string, label: string, run: (target: { kind: string, id: string }) -> ()) -> ()`. Needs `ui` (use `need(ui, "ui", "discard.menu_item")` like `slot`). Errors: unknown target (`unknown menu target \`x\``), bad label (`menu label must be 1 to 40 characters with no control characters`), over 8 items (`too many menu items (max 8)`). Items keep registration order. A repeated call adds another item.
  - `struct MenuItemInfo { pub target: String, pub label: String }` (`Debug, Clone, PartialEq`) and `LoadedInfo.menu_items: Vec<MenuItemInfo>`.
  - `PluginHandle::run_menu_item(&self, index: usize, kind: &str, id: &str) -> Result<(), PluginError>`: runs under the CPU budget. The callback gets a table with exactly the keys `kind` and `id`.
  - `PluginManager::menu_items(&self) -> Vec<(String, usize, MenuItemInfo)>`: `(plugin, index, info)` for plugins with `Status::Running`, in load order.
  - `PluginManager::run_menu_item(&mut self, plugin: &str, index: usize, kind: &str, id: &str) -> Result<(), PluginError>`: mirror `run_action`, including failure counting.
  - `api_spec::FUNCTIONS` gets a `menu_item` entry with `capability: Some("ui")`.

- [ ] **Step 1: Write failing tests** in `tests/runtime.rs`: `menu_item_callback_receives_only_kind_and_id` (callback returns the sorted keys of its argument through a `discard.command`; expect `"id,kind"`), `menu_items_keep_registration_order_and_info`, `a_ninth_menu_item_fails_the_load` (assert `PluginError::Load` and the message), `label_with_newline_or_over_40_characters_fails`, `unknown_target_fails`, `a_callback_that_errors_counts_as_a_failure` (use the manager; `failure_count` goes to 1).
- [ ] **Step 2: Write failing tests** in `tests/capabilities.rs`: `menu_item_needs_the_ui_capability`.
- [ ] **Step 3: Write failing tests** in `manager.rs`: `menu_items_leave_the_list_when_the_plugin_is_terminated`, `menu_items_are_listed_in_load_order`.
- [ ] **Step 4: Run** `cargo test -p discard-lua` — expect FAIL. The existing `api_spec` drift tests also fail until the spec lists `menu_item`.
- [ ] **Step 5: Implement.** Keep the items in a Rust `Arc<Mutex<Vec<(MenuItemInfo, RegistryKey)>>>` or a Lua table indexed by position, with the target and label in `LoadedInfo`. Add a `Request::MenuItem { index, kind, id, reply }` variant. Write the `api_spec` entry with a doc string that names the four targets, the limits, and that the callback sees only `{ kind, id }`.
- [ ] **Step 6: Run** `cargo test -p discard-lua` and clippy — expect PASS and clean (the drift tests included).
- [ ] **Step 7: Commit** `feat: let plugins add menu items with discard.menu_item`

---

### Task 4: Targeted dispatch, copy, and archive

**Files:**
- Modify: `crates/discard-ui/src/dispatch.rs`
- Modify: `crates/discard-ui/src/shared.rs` (add `workspace: Arc<GitWorkspace>`, `channel_id: ChannelId`)
- Modify: `crates/discard-ui/src/lib.rs` (`run` takes the workspace and the channel ID; `Shared` gets both)
- Modify: `crates/discard-ui/src/view.rs` (archive state, composer lock, notice)
- Modify: `crates/discard-ui/src/menus.rs` (new `Action` variants in `action_types!`)
- Modify: `crates/discard-app/src/main.rs` (pass `GitWorkspace` and the channel ID to `discard_ui::run`)

**Interfaces:**
- Consumes: `ContextTarget`, `resolve_copy`, `Action::needs_target` (Task 1). `GitWorkspace::archive(&ChannelId, force: bool) -> Result<ArchiveOutcome, String>` (existing, blocks on git).
- Produces in `dispatch.rs`:
  - `pub fn run_on(action: Action, target: Option<ContextTarget>, cx: &mut App)`. `run(action, cx)` calls `run_on(action, None, cx)`.
  - `pub(crate) enum TargetedPlan { Copy, Archive, ArchiveBlocked, Nothing }` and `pub(crate) fn plan_targeted(action: Action, has_target: bool, turn_running: bool, archived: bool) -> TargetedPlan`. A `Copy*` action without a target is `Nothing`. `ArchiveChannel` is `ArchiveBlocked` while a turn runs and `Nothing` once archived.
  - `pub(crate) enum ArchiveStep { Done(String), NeedsConfirm(Vec<String>), Failed(String) }` and `pub(crate) fn archive_step(result: Result<ArchiveOutcome, String>) -> ArchiveStep`. `Done` carries the user message: `"Channel archived. The branch \`{b}\` is kept. Relaunch to start a new channel."` with a branch, `"Channel archived. Relaunch to start a new channel."` without.
  - `pub(crate) fn confirm_archive(cx)` and `pub(crate) fn cancel_archive(cx)`: the confirm notice's buttons. `confirm_archive` calls `archive(.., true)`.
  - `pub(crate) fn relaunch_same_mode(cx)`: `perform(Confirm::Relaunch { safe_mode: <current> }, cx)`.
- Produces on `DiscardView`: `archived: bool`, `pending_archive: Option<Vec<String>>`, with `set_archived`, `set_pending_archive`, and an early return in `send` when `archived`. The menu context sets `archived`.

- [ ] **Step 1: Write failing tests** in `dispatch.rs`: `copy_actions_need_a_target`, `archive_is_blocked_while_a_turn_runs`, `archive_does_nothing_once_archived`, `archive_runs_when_idle`, `archive_step_maps_archived_with_and_without_a_branch`, `archive_step_maps_needs_confirmation_to_the_item_list`, `archive_step_maps_an_error_to_failed`.
- [ ] **Step 2: Write failing test** in `view.rs` (pure helper): `send_is_ignored_after_archive` — extract the guard as `fn may_send(archived: bool, text: &str) -> bool` and test it.
- [ ] **Step 3: Run** `cargo test -p discard-ui dispatch:: view::` — expect FAIL.
- [ ] **Step 4: Implement.** Copy: `cx.write_to_clipboard(ClipboardItem::new_string(text))` with the text from `resolve_copy(view.history(), &channel_name, &target)`. Archive: run `workspace.archive` on `cx.background_spawn`, then update the view with `archive_step`. The notice with the item list shows **Cancel** and **Archive and delete**. After `Done`, call `refresh_menus` so File > Archive Channel… greys. Add `Archive Channel…` handling to `action_types!` (the `match` is exhaustive, so the four new actions must be listed).
- [ ] **Step 5: Run** `cargo build --workspace`, `cargo test --workspace`, and clippy — expect PASS and clean.
- [ ] **Step 6: Verify** with the `run` skill in a git repo with `--worktree`: File > Archive Channel… on a clean channel. Expect the notice and the locked composer. Click **Relaunch**. Expect a new `general` channel. Repeat with an untracked file in the worktree. Expect the item list, then Cancel keeps everything.
- [ ] **Step 7: Commit** `feat: add targeted actions, copy, and channel archive with confirmation`

---

### Task 5: Context menus and plugin items in the windows

**Files:**
- Modify: `crates/discard-ui/src/view.rs` (message row, code block, channel row)
- Modify: `crates/discard-ui/src/menus.rs` (`MenuEntry::Plugin`, `RunPluginItem`, popup builder)
- Modify: `crates/discard-ui/src/dispatch.rs` (`run_plugin_item`)

**Interfaces:**
- Consumes: `context_menu`, `ContextTarget`, `PluginMenuItem`, `MenuEntry::Plugin` (Task 1); `PluginManager::menu_items`, `run_menu_item` (Task 3); `run_on` (Task 4).
- Produces:
  - `menus::to_popup(entries: Vec<MenuEntry>, target: ContextTarget) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static`. A Rust action item calls `dispatch::run_on(action, Some(target.clone()), cx)` from its `on_click`. A disabled item is disabled. A plugin item calls `dispatch::run_plugin_item`.
  - `menus::to_gpui` handles `MenuEntry::Plugin` with a GPUI data action `RunPluginItem { slot: usize }`. `slot` indexes a `PluginMenuTable(Vec<(String, usize)>)` global, rebuilt in `menus::install`. Declare it with `#[derive(Clone, PartialEq, gpui_kit::Action)]` and `#[action(namespace = discard, no_json)]`. Check the derive in the GPUI source first. If it needs serde, add the derives. The native Plugins menu shows at most 32 plugin items.
  - `dispatch::run_plugin_item(plugin: &str, index: usize, target: Option<&ContextTarget>, cx: &mut App)`: kind and ID come from the target (`"app"` and `""` for the menu bar). On `Err(e)`, set `plugin_notice` to `"{plugin}: {e}"` and bump `plugin_epoch`, so the main window shows the problem line.
  - `MenuContext.plugin_items` is filled from `PluginManager::menu_items()` in `refresh_menus`, and again when `plugin_epoch` changes.

- [ ] **Step 1: Write failing tests** in `menus.rs`: `plugin_entry_becomes_a_run_plugin_item_action_with_its_slot`, `plugin_table_lists_each_plugin_item_once_in_menu_order`, `rust_items_are_never_replaced_by_plugin_items` (feed a plugin item labeled "Quit Discard"; the real Quit entry is still first and the plugin entry is a separate `Plugin` item).
- [ ] **Step 2: Write failing tests** in `dispatch.rs`: `a_failing_plugin_item_sets_the_plugin_notice` (extract the pure `fn plugin_item_notice(plugin: &str, error: &str) -> String` and test it).
- [ ] **Step 3: Run** `cargo test -p discard-ui menus:: dispatch::` — expect FAIL.
- [ ] **Step 4: Implement.** In `view.rs`, give the message row, the `Block::Code` element, and the channel row an element ID and `.context_menu(...)` from `ContextMenuExt`. Build each menu from `context_menu(&target, &self.menu_ctx)`. `render_block` takes an `Option<ContextTarget>` for the code block, built by `message_row` from `message.id` and the block index. The live row passes `None` and adds no menu. If a right click on a code block also opens the message menu, stop propagation in the code block handler.
- [ ] **Step 5: Run** `cargo test --workspace` and clippy — expect PASS and clean.
- [ ] **Step 6: Verify** with the `run` skill. Right-click a message: Copy Message Text, then paste in the composer. Right-click a code block: only Copy Code shows. Right-click the channel row: Copy Channel Name, Archive Channel…. Install a plugin (`--builder`) that calls `discard.menu_item` for each target. Expect its item last in each menu and in Plugins, labeled `Label (name)`. Click it. Expect its callback to run. Make the callback error. Expect the problem line in the main window.
- [ ] **Step 7: Commit** `feat: add context menus and plugin menu items`

---

### Task 6: Providers page

**Files:**
- Create: `crates/discard-core/src/provider_page.rs`
- Modify: `crates/discard-core/src/lib.rs`, `crates/discard-core/src/settings_page.rs`
- Create: `crates/discard-ui/src/settings/providers.rs`
- Modify: `crates/discard-ui/src/settings/mod.rs`, `crates/discard-ui/src/shared.rs`, `crates/discard-ui/src/lib.rs`
- Modify: `crates/discard-app/src/main.rs`

**Interfaces:**
- Produces in `provider_page.rs`:
  - `enum SetupLine { Checking, Ready, NotInstalled { install_hint: String }, NotLoggedIn { login_command: String } }`.
  - `struct CapabilityRow { label: &'static str, supported: bool }`.
  - `struct HookRow { label: &'static str, mode: HookMode }`.
  - `struct ProviderCard { id: String, setup: SetupLine, capabilities: Vec<CapabilityRow>, hooks: Vec<HookRow>, ask_policy_blocked: Option<String> }`.
  - `fn provider_card(id: &str, matrix: &CapabilityMatrix, setup: Option<&SetupStatus>) -> ProviderCard`. Capability rows, in order: "Interactive approvals", "Tool call visibility", "Per-session MCP", "Session resume", "Cancellation", "Block tool calls before they run". Hook rows: "Block a tool call" (`HookKind::BlockToolCall`), "Observe events" (`HookKind::ObserveEvents`). `ask_policy_blocked` is the reason from `enforcement_for(ApprovalPolicy::Ask)` when it fails.
  - `settings_page`: labels `LABEL_SETUP_STATUS` = "Setup status", `LABEL_CAPABILITIES` = "Capabilities", `LABEL_HOOK_ENFORCEMENT` = "Hook enforcement", `LABEL_RECHECK` = "Re-check". `SettingsPage::Providers.is_available()` is true and its `field_labels` lists the four.
- Produces in `discard-ui`: `pub struct ProviderInfo { pub id: String, pub matrix: CapabilityMatrix, pub setup: Arc<Mutex<Option<SetupStatus>>> }`, `pub type Recheck = Arc<dyn Fn(&str) + Send + Sync>`, `Shared.providers: Vec<ProviderInfo>`, `Shared.recheck: Recheck`. `discard_ui::run` takes both.
- App: for each name in `PROVIDERS`, build the provider with `provider_for`, clone its matrix, set `setup` to `None`, then `runtime.spawn` a task that awaits `check_setup()` and stores the result. `recheck(id)` sets `setup` to `None` and spawns the task again.

- [ ] **Step 1: Write failing tests** in `provider_page.rs`: `card_lists_the_six_capabilities_in_order_with_their_values`, `block_hook_is_active_observe_only_or_unsupported_by_the_matrix` (use matrices from `Capabilities` flags), `unknown_setup_is_checking`, `not_installed_and_not_logged_in_carry_the_exact_text`, `ask_policy_blocked_gives_the_reason_when_interactive_approvals_is_off`, `ask_policy_blocked_is_none_when_supported`.
- [ ] **Step 2: Write failing tests** in `settings_page.rs`: update `all_has_ten_pages_and_three_are_available` to four, add `provider_labels_are_searchable`, and fix `provider_matches_general_and_providers` if it still holds. In `settings/mod.rs`, change `search_keeps_unavailable_pages_not_clickable` so Providers is clickable and use `"memory"` (still unavailable) for the unavailable-page case.
- [ ] **Step 3: Run** `cargo test -p discard-core provider_page:: settings_page:: ` and `cargo test -p discard-ui settings::` — expect FAIL.
- [ ] **Step 4: Implement** the core model, then the page: one card per provider with its setup line (the exact command, in a monospace text), a **Re-check** button, the capability table (a missing capability is shown as "No", never hidden), the hook rows (`ObserveOnly` shows "Observe only"), and the "cannot enforce Ask" line when set. Add the providers' setup values to the Settings window poll so the page redraws when a check ends.
- [ ] **Step 5: Run** `cargo test --workspace` and clippy — expect PASS and clean.
- [ ] **Step 6: Verify** with the `run` skill. Open Settings > Providers. Expect both cards and "Checking" turning into a result. Click **Re-check**. Expect "Checking" again, then a result. Rename the `codex` binary on `PATH` for a moment. Expect the install hint.
- [ ] **Step 7: Commit** `feat: add the Providers settings page`

---

### Task 7: Agents page

**Files:**
- Create: `crates/discard-core/src/agent_edit.rs`
- Modify: `crates/discard-core/src/lib.rs`, `crates/discard-core/src/settings_page.rs`
- Create: `crates/discard-ui/src/settings/agents.rs`
- Modify: `crates/discard-ui/src/settings/mod.rs`, `crates/discard-ui/src/shared.rs`, `crates/discard-ui/src/lib.rs`
- Modify: `crates/discard-app/src/main.rs` (pass `store: Arc<Mutex<Store>>`)

**Interfaces:**
- Consumes: `Store::list_agents`, `update_agent` (Task 2); `ProviderInfo.matrix` (Task 6); `AgentChange::needs_user_approval` and `Initiator::User` (existing, `agent.rs`); `CapabilityMatrix::enforcement_for` (existing).
- Produces in `agent_edit.rs`:
  - `struct AgentDraft { model: Option<String>, system_prompt: String, policy: ApprovalPolicy, plugin_write: bool }` (`Clone, PartialEq, Eq`) and `fn AgentDraft::from(&AgentConfig) -> AgentDraft`.
  - `struct FieldChange { field: &'static str, before: String, after: String }`. Fields, in order: "Model" (`(default)` for none), "System prompt", "Approval policy" (`Ask before tool calls` or `Deny all`), "plugin.write" (`off` or `on`).
  - `struct PolicyChoice { policy: ApprovalPolicy, enabled: bool, reason: Option<String> }` and `fn policy_choices(matrix: &CapabilityMatrix) -> Vec<PolicyChoice>`: `Ask` first, then `DenyAll`. A disabled choice carries the `enforcement_for` reason.
  - `struct AgentReview { changes: Vec<FieldChange>, needs_confirm: bool, blocked: Option<String> }` and `fn review(current: &AgentConfig, draft: &AgentDraft, matrix: &CapabilityMatrix) -> AgentReview`. `needs_confirm` is true when `AgentChange::SetPolicy` or `AgentChange::SetPluginWrite`, built from the changes and judged with `Initiator::User`, needs approval. `blocked` is set when the draft policy differs from the current one and the provider cannot enforce it.
  - `enum SaveError { Unchanged, Blocked(String), NeedsConfirm, Store(StoreError) }` and `fn save(store: &Store, id: &AgentId, draft: &AgentDraft, matrix: &CapabilityMatrix, confirmed: bool) -> Result<AgentConfig, SaveError>`. It loads the agent, calls `review`, and writes through `Store::update_agent` only when allowed. It takes no `Initiator`: Settings always acts as the user, and no plugin or agent API calls it.
  - `settings_page`: labels `LABEL_AGENT` = "Agent", `LABEL_MODEL` = "Model", `LABEL_SYSTEM_PROMPT` = "System prompt", `LABEL_APPROVAL_POLICY` = "Approval policy", `LABEL_PLUGIN_WRITE` = "plugin.write". `SettingsPage::Agents.is_available()` is true.
- Produces in `discard-ui`: `Shared.store: Arc<Mutex<Store>>`.

- [ ] **Step 1: Write failing tests** in `agent_edit.rs`, each with `Store::open_in_memory()`:
  - `unchanged_draft_has_no_changes_and_save_says_unchanged`.
  - `review_lists_changed_fields_in_order_with_before_and_after`.
  - `loosening_policy_needs_confirm_and_tightening_does_not`.
  - `turning_plugin_write_on_needs_confirm_and_off_does_not`.
  - `save_without_confirm_for_a_loosening_change_returns_needs_confirm_and_writes_nothing` (reload the agent and compare).
  - `save_with_confirm_writes_the_loosening_change`.
  - `save_of_a_tightening_change_needs_no_confirm`.
  - `a_policy_the_provider_cannot_enforce_is_blocked_even_when_confirmed` (matrix with `interactive_approvals: false`, draft `Ask`).
  - `policy_choices_disable_ask_with_the_reason_when_the_provider_cannot_pause`.
  - `save_for_an_unknown_agent_is_a_store_error`.
  - `model_and_prompt_edits_save_without_confirm`.
- [ ] **Step 2: Write failing tests** in `settings_page.rs`: `agent_labels_are_searchable`, and update the available-page test to five.
- [ ] **Step 3: Run** `cargo test -p discard-core agent_edit:: settings_page::` — expect FAIL.
- [ ] **Step 4: Implement** `agent_edit.rs`. `review` builds one `AgentChange` per policy or `plugin_write` change and asks `needs_user_approval(&Initiator::User)`, so the rule has one source.
- [ ] **Step 5: Implement** the page. A button per agent (name and provider). For the selected agent: read-only name and provider; a model text input; a system-prompt text area (the same type the composer uses, `TextareaState` in `view.rs`); two policy buttons built from `policy_choices` (a disabled button shows its reason); a `plugin.write` switch; a review box with each `FieldChange` as `before → after`; a **Save** button. When `needs_confirm`, the first click shows a warning ("This lets the agent do more without asking.") and a **Confirm and save** button. After a successful save, show "Applies at next launch" and a **Relaunch** button that calls `dispatch::relaunch_same_mode`. A `SaveError` shows inline until the next change.
- [ ] **Step 6: Run** `cargo test --workspace` and clippy — expect PASS and clean.
- [ ] **Step 7: Verify** with the `run` skill. Open Settings > Agents. Change the model and save. Expect "Applies at next launch" and the new model after Relaunch. Switch the policy to Deny all and save. Expect no warning. Switch back to Ask. Expect the warning and a second button. Click **Cancel** by picking another agent. Expect no write.
- [ ] **Step 8: Commit** `feat: add the Agents settings page`

---

### Task 8: Docs and final check

**Files:**
- Modify: `docs/settings.md`, `docs/plugins.md`, `docs/provider-claude.md` and `docs/provider-codex.md` (only where they name the Providers page)
- Modify: `crates/discard-lua/src/api_spec.rs` (the authoring guide, if it lists API functions by name)

- [ ] **Step 1: Update `docs/settings.md`.** Menu table: Archive Channel… is live and acts on the current channel; "Four items" becomes three. Add a "Context menus" section (targets, items, the live row has none). Add "Providers page" and "Agents page" sections, with the rules from Global Constraints and the "Applies at next launch" limit. Add "Archive Channel" to the safe-mode-style flow notes: blocked while a turn runs, the confirm list, the lock, and Relaunch. Remove the sentence that says context menus and these pages arrive in M8. Update the sidebar line: "Pages that work now" gets Providers and Agents.
- [ ] **Step 2: Update `docs/plugins.md`.** Document `discard.menu_item`: targets, limits, the `{ kind, id }` argument, the label format, and that items need `ui`.
- [ ] **Step 3: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` — expect PASS and clean.
- [ ] **Step 4: Commit** `docs: describe context menus, plugin menu items, and the Providers and Agents pages`

---

## Self-review notes

- **Spec coverage.** Context menus on message, code block, channel row: Tasks 1, 4, 5. Plugin menu items in the Plugins menu and context menus: Tasks 1, 3, 5. Providers page: Task 6. Agents page with diff, loosening confirm, provider enforcement, and no plugin path: Task 7. Archive with the data-loss check: Task 4. Appearance: already shipped. Left out and named above: plugin-declared settings fields, agent creation from the UI.
- **Type consistency.** `ContextTarget`, `MenuTarget`, `PluginMenuItem`, and `MenuEntry::Plugin` come from Task 1 and are used unchanged in Tasks 4 and 5. `ProviderInfo.matrix` (Task 6) feeds `policy_choices` and `review` (Task 7). `Store::update_agent` (Task 2) is the only agent write in Task 7.
- **Known unverified items.** The GPUI data-action derive (Task 5), the `PopupMenuItem::on_click` handler signature, and whether `ContextMenu` stops propagation between nested menus (Task 5) are not checked yet. Each has a step that checks it first. `GitWorkspace` must be `Send + Sync` for `background_spawn` (Task 4). If it is not, wrap the call in `Arc<Mutex<_>>`.
