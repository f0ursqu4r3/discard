# Tauri + Vue + shadcn-vue Conversion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. The user chose OpenAI Codex as the builder: run each Phase with `/codex-build`, then review the diff before any commit.

**Goal:** Replace the GPUI front end (`discard-ui`, `gpui-kit`) with a Tauri 2 shell and a Vue 3 + shadcn-vue front end, with no loss of behavior.

**Architecture:** `discard-core`, `discard-providers`, `discard-lua`, and `discard-mcp` stay unchanged. `discard-app` becomes the Tauri app. A new `desktop` module owns the `UiState`, the engine channels, and the plugin service. It pushes small JSON snapshots to the webview and takes commands from it. The webview holds no business logic: it renders snapshots, and it renders plugin element trees through fixed components. Lua never runs in the webview.

**Tech Stack:** Rust 2024, Tauri 2, Vue 3.5 + TypeScript, Vite, Tailwind CSS 4, shadcn-vue (reka-ui), vue-tsc, Vitest + @vue/test-utils, markdown-it + DOMPurify, shiki, pnpm.

**Spec:** `PLAN.md` (product design, unchanged except the GPUI lines), `docs/ui-juice-spec.md` (the visual behavior to keep), and the decision in the conversation of 2026-11-04: "switch to Tauri + Vue + shadcn-vue".

## Global Constraints

- `discard-core` keeps no GUI dependency and no Tauri dependency. New Rust code for the bridge lives in `crates/discard-app/src/desktop/`.
- Tauri `2.x`; Vue `^3.5`; Tailwind `^4`; package manager `pnpm`; the front end lives in `web/`.
- macOS first. Do not add code that breaks Linux or Windows builds.
- The webview loads no remote asset, font, script, or style. CSP: `default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src ipc: http://ipc.localhost`.
- Tauri capabilities: only `core:default` plus the events and window calls the app uses. No `fs`, `shell`, `http`, or `opener` plugin.
- `v-html` is allowed in exactly one file: `web/src/components/SafeHtml.vue`, which sanitizes with DOMPurify.
- These surfaces are drawn by fixed Vue components that no plugin element can cover, hide, or restyle: approval card, review banner, plugin problem line, confirm notice, archive notice, safe-mode screen, plugin error list.
- Plugin element props not in the whitelist of `web/src/plugin/props.ts` are ignored. Numeric props are clamped. Colors come only from the 14 theme token names.
- Every color, radius, and spacing value in Vue comes from a CSS variable set from theme tokens. No hard-coded colors outside `web/src/styles/tokens.css` defaults.
- Animations last 200 ms or less and stop under `prefers-reduced-motion`.
- Behavior stays as in the GPUI app. Where this plan says "parity", read the named GPUI file and port every behavior and every pure-function test.
- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm --dir web lint`, `pnpm --dir web typecheck`, `pnpm --dir web test`, and `pnpm --dir web build` all pass at the end of every task.
- Commit messages use Conventional Commits. Do not add `Co-Authored-By` trailers or any Claude attribution line (user rule in `~/.claude/CLAUDE.md`). Commit only after the user approves the diff.

## Review Focus

Failure modes the spec implies and no feature task would otherwise test, most likely first:

1. **Streaming flood.** A turn can emit thousands of deltas per second. The webview must stay responsive, and the last snapshot must equal the true final state. Owned by Task 3.
2. **Hostile plugin output.** A plugin returns `<img src=x onerror=...>` as text, a 2000-node tree, or unknown props such as `style` and `class`. All render inert. Owned by Task 8.
3. **Late or reloaded webview.** The webview mounts after events fired (or reloads in dev). It must recover the full state with one command. Owned by Task 3.
4. **Quit and close while a turn runs.** Cmd+Q, the red close button, and Relaunch ask for confirmation. A failed relaunch never quits. Owned by Task 9.
5. **Unreadable theme.** A missing Ghostty theme, a light background, or a plugin theme with low contrast must not make protected surfaces unreadable. Owned by Task 4.

---

## File Structure

```
Cargo.toml                          workspace: keep members, drop discard-ui in Task 13
crates/discard-app/
  Cargo.toml                        add tauri, tauri-build, serde, notify
  build.rs                          tauri_build::build()
  tauri.conf.json                   window, CSP, devUrl, frontendDist ../../web/dist
  capabilities/default.json         minimal permissions
  src/main.rs                       startup stays; calls desktop::run
  src/desktop/mod.rs                run(), Tauri builder, state registration
  src/desktop/dto.rs                serde DTOs and Snapshot
  src/desktop/core.rs               AppCore: owns UiState, channels, plugin service. No Tauri types.
  src/desktop/commands.rs           #[tauri::command] wrappers over AppCore
  src/desktop/pump.rs               update loop and snapshot throttling
  src/desktop/theme.rs              appearance logic ported from discard-ui/src/appearance.rs
  src/desktop/problems.rs           moved from discard-ui/src/problems.rs
  src/desktop/plan.rs               pure action planning moved from discard-ui/src/dispatch.rs
  src/desktop/menu.rs               MenuSpec -> Tauri menu, accelerators
  src/desktop/windows.rs            main and settings windows, bounds persistence
  src/desktop/settings/*.rs         one file per Settings page
web/
  package.json, vite.config.ts, tsconfig*.json, components.json, eslint config
  src/main.ts, App.vue
  src/bridge/{types.ts,api.ts,store.ts}      TS mirror of dto.rs, invoke/listen wrappers, reactive store
  src/bridge/fixtures/snapshot.json          golden file written by a Rust test
  src/styles/{tokens.css,app.css}
  src/components/ui/                         shadcn-vue generated components
  src/components/{ServerRail,ChannelList,ChannelHeader,MemberList,MessageList,MessageRow,
    BlockView,CodeBlock,DiffBlock,ToolCallBlock,ApprovalCard,Composer,Notices,SafeHtml,
    SafeMarkdown,ContextMenuHost}.vue
  src/plugin/{PluginElement.vue,props.ts}
  src/settings/{SettingsApp.vue,pages/*.vue}
```

---

## Phase 1: Shell and contract

### Task 1: Scaffold the Tauri shell and the web project

**Files:**
- Create: `web/` (Vite Vue TS project), `web/components.json`, `crates/discard-app/{build.rs,tauri.conf.json,capabilities/default.json}`
- Modify: `crates/discard-app/Cargo.toml`, `.gitignore` (`web/node_modules`, `web/dist`)
- Test: `web/src/App.test.ts`

**Interfaces:**
- Produces: a window that loads `web/dist`, titled "Discard", 1200x800. Scripts in `web/package.json`: `dev`, `build`, `lint`, `typecheck`, `test`. `tauri.conf.json` `build.beforeDevCommand` is `pnpm --dir ../../web dev`, `devUrl` is `http://localhost:5173`, `beforeBuildCommand` is `pnpm --dir ../../web build`.

- [ ] **Step 1: Write the failing test** in `web/src/App.test.ts`: mounting `App` renders an element with `data-testid="app-root"`.
- [ ] **Step 2: Run** `pnpm --dir web test`. Expected: FAIL (no project yet).
- [ ] **Step 3: Create the project**: `pnpm create vite web --template vue-ts`; add Tailwind 4 (`@tailwindcss/vite`), init shadcn-vue (`pnpm dlx shadcn-vue@latest init`, neutral base, CSS variables on), add `@tauri-apps/api`, `vitest`, `@vue/test-utils`, `jsdom`, `eslint`, `vue-tsc`. Add the Tauri config, `build.rs`, and capabilities per Global Constraints. Add `tauri = { version = "2", features = [] }` and `tauri-build = "2"` to `discard-app`. Keep `discard_ui::run` as the active entry for now: the Tauri builder is not called yet.
- [ ] **Step 4: Run** `pnpm --dir web test && pnpm --dir web build && cargo build -p discard-app`. Expected: all pass.
- [ ] **Step 5: Commit** `feat: scaffold the Tauri shell and the web project`.

### Task 2: Snapshot DTOs and the golden fixture

**Files:**
- Create: `crates/discard-app/src/desktop/{mod.rs,dto.rs}`, `web/src/bridge/types.ts`, `web/src/bridge/fixtures/snapshot.json`
- Test: `crates/discard-app/src/desktop/dto.rs` (unit), `web/src/bridge/types.test.ts`

**Interfaces:**
- Produces (Rust, all `#[derive(Serialize, Clone, PartialEq, Debug)]`, `camelCase` field names):
  - `BlockDto` (tagged `type`, same shape as `discard_core::model::Block`), `MessageDto { id, role, blocks: Vec<BlockDto>, continuesGroup: bool }`
  - `PromptDto { approvalId, tool, summary, view: "pending"|"approved"|"denied"|"stale" }`
  - `BranchRowDto { key, label, kind: "main"|"worktree"|"branch", depth: u8, current: bool, collapsed: bool }`
  - `ThemeDto { mode: "light"|"dark", tokens: BTreeMap<String,String>, radius: Option<f32>, fontScale: f32, spacingScale: f32, protectedBg, protectedFg, protectedBorder }`
  - `SlotDto { plugin: String, element: Element }` (`Element` from `discard_lua::element`, already serializable)
  - `SnapshotDto { rev: u64, historyRev: u64, project, channel, agent, provider, agentStatus, turnRunning, notice: Option<String>, liveBlocks, prompts, pendingApprovals: usize, unread: bool, archived: bool, archiveBusy: bool, memberListVisible: bool, panels: {channelList: f32, memberList: f32}, branches: Vec<BranchRowDto>, slots: BTreeMap<String, SlotDto>, banners: BannersDto, theme: ThemeDto }`
  - `BannersDto { reviewCount: usize, problemLines: Vec<String>, confirm: Option<String>, archive: Option<ArchiveNoticeDto>, safeMode: bool }`
  - `pub fn history_dto(messages: &[Message]) -> Vec<MessageDto>` (sets `continuesGroup` with `discard_core::ui_state::continues_group`).
- Produces (TS): `web/src/bridge/types.ts` with matching interfaces.

- [ ] **Step 1: Write the failing tests.** Rust: `history_dto_marks_continued_groups` (user, agent, agent gives `[false,false,true]`); `block_dto_serializes_with_type_tag`; `sample_snapshot_matches_golden_file` builds a fixed `SnapshotDto` and compares `serde_json::to_string_pretty` to `web/src/bridge/fixtures/snapshot.json` (set `UPDATE_GOLDEN=1` to rewrite). TS: `types.test.ts` imports the fixture and assigns it to `SnapshotDto` (`satisfies` check via `vue-tsc`) and asserts `snapshot.liveBlocks[0].type`.
- [ ] **Step 2: Run** `cargo test -p discard-app desktop::dto` and `pnpm --dir web test`. Expected: FAIL.
- [ ] **Step 3: Implement** the DTOs and `history_dto` in `dto.rs`, write the TS mirror, generate the fixture with `UPDATE_GOLDEN=1`.
- [ ] **Step 4: Run** both test commands. Expected: PASS.
- [ ] **Step 5: Commit** `feat: add snapshot DTOs and the golden fixture`.

### Task 3: AppCore, commands, and the snapshot pump

**Files:**
- Create: `crates/discard-app/src/desktop/{core.rs,commands.rs,pump.rs}`, `web/src/bridge/{api.ts,store.ts}`
- Modify: `crates/discard-app/src/main.rs` (call `desktop::run` instead of `discard_ui::run`), `desktop/mod.rs`
- Test: `crates/discard-app/tests/core_flow.rs`, `web/src/bridge/store.test.ts`

**Interfaces:**
- Consumes: `UiState`, `Command`, `Update` from `discard_core::ui_state`; the fake provider in `discard-providers`.
- Produces (Rust):
  - `pub struct AppCore` with `pub fn new(init: CoreInit) -> Self`, `pub fn snapshot(&self) -> SnapshotDto`, `pub fn history(&self) -> Vec<MessageDto>`, `pub fn send(&self, text: String)`, `pub fn decide(&self, approval_id: String, decision: String)`, `pub fn cancel(&self)`, `pub fn apply(&self, update: Update)`. It holds `Mutex<UiState>` and the command sender. It has no Tauri types.
  - `pub trait Emit: Send + Sync { fn snapshot(&self, s: &SnapshotDto); fn history(&self, h: &[MessageDto]); }`
  - `pub async fn pump(core: Arc<AppCore>, updates: UnboundedReceiver<Update>, emit: Arc<dyn Emit>, min_interval: Duration)`: applies each update at once, emits at most one snapshot per `min_interval` (default 16 ms) and always emits a final one when the channel is idle. Emits `history` only when `historyRev` changed.
  - Tauri commands: `get_snapshot() -> SnapshotDto`, `get_history() -> Vec<MessageDto>`, `send_message(text: String)`, `decide_approval(approval_id: String, decision: String)`, `cancel_turn()`. Events: `discard://snapshot`, `discard://history`.
- Produces (TS): `api.ts` exports `getSnapshot()`, `getHistory()`, `sendMessage(text)`, `decideApproval(id, decision)`, `cancelTurn()`; `store.ts` exports `useStore()` returning reactive `{ snapshot, history }`, which loads both on mount and then follows events.

- [ ] **Step 1: Write the failing tests.** Rust (`core_flow.rs`, headless, scripted fake provider, `RecordingEmit`): `send_streams_then_finishes` (history ends with the user message and the agent reply; last snapshot has `turnRunning == false`); `ten_thousand_deltas_emit_few_snapshots` (apply 10 000 text deltas through `pump` with 16 ms interval: emitted snapshot count < 500, and the last snapshot's live text equals the concatenation); `get_snapshot_after_the_fact_returns_full_state` (a fresh `snapshot()` call after completion equals the last emitted one). TS: `store.test.ts` with a mocked `@tauri-apps/api`: the store calls `get_snapshot` once on mount, then updates on a `discard://snapshot` event.
- [ ] **Step 2: Run** `cargo test -p discard-app --test core_flow` and `pnpm --dir web test`. Expected: FAIL.
- [ ] **Step 3: Implement** the types above. In `main.rs`, replace the `discard_ui::run(...)` call with `desktop::run(DesktopInit { ... })`, which builds `AppCore`, registers it with `tauri::Builder::manage`, spawns `pump` on the existing tokio runtime with a Tauri-backed `Emit`, and registers the commands. `App.vue` shows the project and channel names and the live text as plain text to prove the loop.
- [ ] **Step 4: Run** both test commands, then `cargo run -p discard-app` and send one message. Expected: tests PASS, and the plain view streams the reply.
- [ ] **Step 5: Commit** `feat: bridge the engine to the webview with snapshots`.

### Task 4: Theme tokens as CSS variables

**Files:**
- Create: `crates/discard-app/src/desktop/theme.rs`, `web/src/styles/tokens.css`, `web/src/bridge/theme.ts`
- Modify: `desktop/core.rs` (fill `ThemeDto`), `web/src/main.ts`
- Test: `theme.rs` (unit, ported), `web/src/bridge/theme.test.ts`

**Interfaces:**
- Consumes: `discard_core::ghostty::live::LiveTheme`, `discard_lua::theme::{ThemeOverrides, ProtectedPalette}`.
- Produces: port `AppearanceState`, `layer`, `as_overrides`, `scales` and the pure tests from `crates/discard-ui/src/appearance.rs` into `theme.rs`, dropping every GPUI type. Add `pub fn theme_dto(state: &AppearanceState, mode: Appearance) -> ThemeDto` and `pub fn contrast_ok(fg: Rgb, bg: Rgb) -> bool` (WCAG ratio >= 4.5). TS: `applyTheme(theme: ThemeDto, root: HTMLElement)` sets `--background`, `--foreground`, `--sidebar`, `--sidebar-foreground`, `--sidebar-accent`, `--primary`, `--primary-foreground`, `--muted`, `--muted-foreground`, `--border`, `--accent`, `--success`, `--warning`, `--danger` (the 14 tokens), plus `--radius`, `--font-scale`, `--space-scale`, `--protected-bg|fg|border`, and toggles the `dark` class. `tokens.css` maps these to the shadcn-vue variable names.

- [ ] **Step 1: Write the failing tests.** Rust: the ported tests, plus `theme_dto_carries_all_14_tokens`, `a_light_background_gives_light_mode`, `protected_palette_falls_back_when_contrast_is_low` (a plugin sets protected fg equal to bg, so the DTO uses `PROTECTED_DEFAULT` colors from `discard-ui/src/shared.rs`). TS: `applyTheme_sets_every_token_and_toggles_dark`.
- [ ] **Step 2: Run** `cargo test -p discard-app desktop::theme` and `pnpm --dir web test`. Expected: FAIL.
- [ ] **Step 3: Implement.** Re-read the theme on the same triggers as the GPUI window: settings change, plugin theme change, Ghostty file change (reuse the 700 ms poll as a tokio interval), system appearance change (Tauri `WindowEvent::ThemeChanged`). Emit through the snapshot, not a new event.
- [ ] **Step 4: Run** both test commands. Expected: PASS.
- [ ] **Step 5: Commit** `feat: drive the web theme from Ghostty and plugin tokens`.

---

## Phase 2: Main window

### Task 5: Layout shell, panels, window state

**Files:**
- Create: `web/src/components/{ServerRail,ChannelList,ChannelHeader,MemberList}.vue`, `desktop/windows.rs`
- Modify: `web/src/App.vue`, `web/src/bridge/api.ts`
- Test: `web/src/components/layout.test.ts`, `windows.rs` (unit)

**Interfaces:**
- Consumes: `SnapshotDto.branches|panels|memberListVisible|archived`, `shadcn-vue` `resizable`, `tooltip`, `badge`, `avatar`.
- Produces: commands `set_panels(channel_list: f32, member_list: f32)`, `set_member_list_visible(visible: bool)`, `toggle_branch(key: String)`. Rust persists through `StateStore` (`set_panels`, `set_member_list_visible`) and saves window bounds with `is_restorable` on resize and move (debounced 300 ms). Components: three-panel layout with the same min and max widths as `MIN_PANEL_WIDTH`/`MAX_PANEL_WIDTH` in `view.rs`; server rail with squircle project icon (`server_initials`, pill heights from `server_pill_height`, count badge for pending approvals, settings cog with review badge); channel list showing the branch tree with collapse; header with member-list toggle.

- [ ] **Step 1: Write the failing tests.** TS: `server_pill_height` port covers selected/hover/unread (same table as the Rust tests in `view.rs:1247`); `serverInitials` port (tests at `view.rs:1270`); `ChannelList` renders rows with indent `depth` and emits `toggle_branch`; `ServerRail` shows the badge only when `pendingApprovals > 0`. Rust: `bounds_saved_only_when_restorable`.
- [ ] **Step 2: Run** `pnpm --dir web test` and `cargo test -p discard-app desktop::windows`. Expected: FAIL.
- [ ] **Step 3: Implement** parity with `view.rs` `server_rail`, `channel_list`, `member_list`, `header`, and `branch_tree.rs` consumers. The plugin slots arrive in Task 8: leave `<slot>` placeholders now.
- [ ] **Step 4: Run** both test commands. Expected: PASS. Then `cargo run -p discard-app` and check the panels resize and persist across a restart.
- [ ] **Step 5: Commit** `feat: add the three-panel layout and window state`.

### Task 6: Message stream and blocks

**Files:**
- Create: `web/src/components/{MessageList,MessageRow,BlockView,CodeBlock,DiffBlock,ToolCallBlock,SafeHtml,SafeMarkdown}.vue`, `web/src/lib/markdown.ts`
- Test: `web/src/components/messages.test.ts`, `web/src/lib/markdown.test.ts`

**Interfaces:**
- Consumes: `MessageDto`, `BlockDto`, `SnapshotDto.liveBlocks|turnRunning|unread`.
- Produces: `renderMarkdown(src: string): string` (markdown-it with `html: false`, then DOMPurify); `SafeHtml` prop `html: string`; `MessageList` props `{ messages: MessageDto[]; live: BlockDto[]; running: boolean; newAnchorId: string | null }` with a "Jump to latest" button when not at the bottom; command `mark_read(message_id: String)` called when the pane is at the bottom and the window is focused. `thinkingLine(running: boolean, agents: string[]): string | null` and `hasLiveRow(running, liveBlocks)` ported from `view.rs:1205-1222`.

- [ ] **Step 1: Write the failing tests.** `renderMarkdown('<img src=x onerror=alert(1)>')` returns no `onerror` and no `<img`; fenced code produces a `CodeBlock`-compatible token; `MessageRow` hides avatar and name when `continuesGroup`; `DiffBlock` classes `+`, `-`, and `@@` lines; `ToolCallBlock` shows name and collapses output over 20 lines; `MessageList` shows the "New" divider above `newAnchorId`; `thinkingLine` and `hasLiveRow` match the Rust test table (`view.rs:1308-1330`); streaming cursor shows only while `running`.
- [ ] **Step 2: Run** `pnpm --dir web test`. Expected: FAIL.
- [ ] **Step 3: Implement** parity with `message_row`, `live_row`, `render_block`, `diff_lines` in `view.rs`, and `ui-juice-spec.md` Surface 1 (grouping, hover toolbar that reuses existing copy actions, code/diff headers with copy button, tool-call accent bar, fade-in under 200 ms). Highlight code with shiki, loaded lazily. Backend: `mark_read` uses `StateStore::set_last_read` and the `new_divider_anchor` logic in `discard_core::ui_state`.
- [ ] **Step 4: Run** `pnpm --dir web test`. Expected: PASS. Check a long streamed reply scrolls smoothly in `cargo run -p discard-app`.
- [ ] **Step 5: Commit** `feat: render messages and typed blocks`.

### Task 7: Composer, approval cards, notices

**Files:**
- Create: `web/src/components/{Composer,ApprovalCard,Notices}.vue`
- Modify: `desktop/core.rs` (banners), `desktop/commands.rs` (`confirm_resolve(accept: bool)`, `archive_confirm(accept: bool)`)
- Test: `web/src/components/composer.test.ts`, `web/src/components/protected.test.ts`

**Interfaces:**
- Consumes: `PromptDto`, `BannersDto`, `SnapshotDto.notice|archived|archiveBusy`.
- Produces: `mayStop`/`maySend(archived: boolean, text: string): boolean` (port of `may_send`); `Composer` emits `send` on Enter, Shift+Enter inserts a newline, Stop replaces Send while `turnRunning`; `ApprovalCard` has Approve and Deny buttons that call `decideApproval`; `Notices` draws the review banner, plugin problem line (`problemLines`, summary via `problem_summary` rules from `problems.rs`), confirm notice, archive notice, setup notice.

- [ ] **Step 1: Write the failing tests.** `maySend` table (`view.rs:1300`); Enter sends, Shift+Enter does not; composer disabled when archived or busy; `ApprovalCard` for `view: "stale"` shows no buttons; `Notices` renders the confirm notice with Confirm and Cancel; each protected component has `data-protected="true"`; a test mounts a plugin slot element (Task 8 fixture) next to them and asserts they stay in the DOM.
- [ ] **Step 2: Run** `pnpm --dir web test`. Expected: FAIL.
- [ ] **Step 3: Implement.** Protected components read colors from `--protected-*` variables only. Move `problems.rs` into `desktop/problems.rs` with its tests unchanged.
- [ ] **Step 4: Run** `pnpm --dir web test && cargo test -p discard-app`. Expected: PASS. Run one real approval end to end in `cargo run -p discard-app -- --provider claude`.
- [ ] **Step 5: Commit** `feat: add the composer, approval cards, and notices`.

### Task 8: Plugin slots and the element renderer

**Files:**
- Create: `web/src/plugin/{PluginElement.vue,props.ts}`, `crates/discard-app/src/desktop/slots.rs`
- Modify: `desktop/core.rs` (fill `slots`), `desktop/commands.rs` (`plugin_action(plugin: String, action: String)`), `web/src/App.vue` (slots `channel_header`, `member_list`, plus the others in `render_slot` usage)
- Test: `web/src/plugin/plugin.test.ts`, `slots.rs` (unit)

**Interfaces:**
- Consumes: `discard_lua::element::{Element, Node, Tag, Prop}`, `PluginManager::render_slot(slot, &state)`, `PluginManager::run_action`.
- Produces: `slots.rs` `pub fn render_slots(manager: &PluginManager, state: &serde_json::Value) -> BTreeMap<String, SlotDto>` with a cache keyed by `state.to_string()` (as in `plugin_slot`); the member slot root drops `width` and gets `grow` (as in `view.rs:419-445`). `PluginElement` props `{ element: Element; plugin: string }`. `props.ts` exports `PROP_WHITELIST` and `clampNumber(name, n)`. Mapping per tag matches `plugin_element.rs` (`row`, `column`, `text`, `markdown`, `button`, `avatar`, `badge`, `divider`, `spacer`, `dot`), with `button.on_click` calling `plugin_action`, and `markdown` going through `SafeMarkdown`.

- [ ] **Step 1: Write the failing tests.** For each of the 10 tags, one render test with the props that `plugin_element.rs` supports. Hostile cases: text `<img src=x onerror=alert(1)>` renders as text with no `img` element; props `style`, `class`, `onclick`, `dangerouslySetInnerHTML` are ignored; `gap: 1e9` clamps; a 2000-node tree renders without throwing; an unknown color name falls back to the default; a `button` with no `on_click` is inert. Rust: `member_slot_root_gets_grow_and_loses_width`; `slot_cache_hits_for_identical_state`.
- [ ] **Step 2: Run** `pnpm --dir web test` and `cargo test -p discard-app desktop::slots`. Expected: FAIL.
- [ ] **Step 3: Implement.** Clear the slot cache after `plugin_action`, after a plugin change, and after a revert, as `after_plugin_change` does. Port the `plugins.maintain()` revert-and-notice logic that runs in `render` (`view.rs:1072`) into a 1 s backend interval.
- [ ] **Step 4: Run** both test commands. Expected: PASS. Run `cargo run -p discard-app` with the bundled `default-ui` plugin and compare it with the GPUI build side by side.
- [ ] **Step 5: Commit** `feat: render plugin slots through fixed components`.

### Task 9: Native menus, keys, context menus, quit rules

**Files:**
- Create: `desktop/{menu.rs,plan.rs}`, `web/src/components/ContextMenuHost.vue`
- Modify: `desktop/commands.rs` (`run_action(action_id: String, target: Option<ContextTargetDto>)`, `context_menu(target: ContextTargetDto) -> Vec<MenuEntryDto>`, `plugin_menu_action(plugin: String, index: usize, target: ContextTargetDto)`), `desktop/windows.rs`
- Test: `plan.rs` (unit, ported), `menu.rs` (unit)

**Interfaces:**
- Consumes: `discard_core::action::{menu_bar, MenuContext, MenuSpec, MenuEntry, Action, needs_confirm}`, `discard_core::context_menu::context_menu`, `discard_core::keymap::Keymap`.
- Produces: move `Plan`, `Close`, `Confirm`, `plan`, `close_plan`, `prepare_exit`, `plan_targeted`, `archive_step` and every pure test from `crates/discard-ui/src/dispatch.rs` into `plan.rs`, with no GPUI type. `menu.rs`: `pub fn build_menu<R: Runtime>(app: &AppHandle<R>, ctx: &MenuContext, keymap: &Keymap) -> tauri::Result<Menu<R>>`, with accelerators from `display_chord`-compatible chords (`parse_chord` output converted to Tauri accelerator strings), disabled items for `FutureItem`, standard items mapped to predefined Tauri items. `run_action` executes a `Plan` (open Settings page, cancel turn, toggle member list, hide/minimize/zoom, quit/relaunch with confirm).

- [ ] **Step 1: Write the failing tests.** The ported `plan.rs` tests (`close_plan`, `plan`, `prepare_exit`, `plan_targeted`, `archive_step`, including "a failed relaunch never quits"). Menu: `each_core_action_has_a_menu_item_id`, `future_entry_becomes_a_disabled_item`, `plugin_items_are_listed_once_in_menu_order`, `chord_converts_to_a_tauri_accelerator` (`cmd+,` gives `CmdOrCtrl+,`). Close rule: the main window `CloseRequested` is prevented and replaced by the confirm notice when `close_plan` says `Ask`.
- [ ] **Step 2: Run** `cargo test -p discard-app desktop::plan desktop::menu`. Expected: FAIL.
- [ ] **Step 3: Implement** `menu.rs`, `plan.rs`, the context-menu host (shadcn-vue `context-menu`, entries come from the backend, plugin items carry the plugin name), copy actions through `resolve_copy` and the clipboard (use the Web Clipboard API; no Tauri clipboard plugin). Cancel a running turn on app exit best effort, as `cancel_running_turn` does.
- [ ] **Step 4: Run** `cargo test -p discard-app` and `pnpm --dir web test`. Expected: PASS. Manually check Cmd+Q, the red button, and Relaunch with a turn running.
- [ ] **Step 5: Commit** `feat: add native menus, key bindings, and context menus`.

---

## Phase 3: Settings window

### Task 10: Settings window, search, General, Appearance

**Files:**
- Create: `web/src/settings/{SettingsApp.vue,pages/General.vue,pages/Appearance.vue}`, `desktop/settings/{mod.rs,general.rs,appearance.rs}`
- Modify: `desktop/windows.rs` (`open_settings(page: SettingsPage)`), `web/src/main.ts` (route by `#/settings/<page>`), `desktop/menu.rs`
- Test: `web/src/settings/settings.test.ts`, `desktop/settings/*.rs` (unit)

**Interfaces:**
- Consumes: `discard_core::settings_page::{SettingsPage, search}`, `discard_core::settings::SettingsStore`, `LiveTheme`.
- Produces: one window labelled `settings`, single instance (a second open focuses it). Backend emits `discard://settings` when `settings.toml` changes (poll `reload_if_changed` every 700 ms, as `POLL_INTERVAL` does). Commands: `settings_get(page: String) -> serde_json::Value`, `settings_set_general(patch: GeneralPatch) -> Result<(), String>`, `settings_set_appearance(patch: AppearancePatch) -> Result<(), String>`. Each command returns the same validation error text the GPUI page shows. Sidebar entries and "Soon" tags use `SettingsPage::is_available`.

- [ ] **Step 1: Write the failing tests.** TS: search "theme" keeps the Appearance entry only; an unavailable page is not clickable; a rejected patch shows the error and keeps the old value. Rust: `second_open_focuses_the_existing_window` (plan-level function `settings_window_action(exists: bool) -> Create|Focus`); `invalid_general_patch_returns_the_validation_error`.
- [ ] **Step 2: Run** both test commands. Expected: FAIL.
- [ ] **Step 3: Implement** parity with `crates/discard-ui/src/settings/{mod,general,appearance}.rs`, including live preview of theme changes (they flow through the Task 4 snapshot).
- [ ] **Step 4: Run** both test commands. Expected: PASS. Edit `settings.toml` by hand while the window is open and see the page update.
- [ ] **Step 5: Commit** `feat: add the Settings window with General and Appearance`.

### Task 11: Providers, Agents, Workspaces, Memory pages

**Files:**
- Create: `web/src/settings/pages/{Providers,Agents,Workspaces,Memory}.vue`, `desktop/settings/{providers.rs,agents.rs,workspaces.rs,memory.rs}`
- Test: `web/src/settings/pages.test.ts`, unit tests in each Rust file

**Interfaces:**
- Consumes: `provider_page`, `agent_edit`, `workspace_page`, `memory_page` modules in `discard-core`; `Recheck`.
- Produces: commands `settings_providers() -> Vec<ProviderRowDto>`, `settings_recheck_provider(id: String)`, `settings_agent_get/save`, `settings_workspaces()`, `settings_memory_scopes()`, `settings_memory_read(scope: String) -> String`. Agent edits that change a permission policy go through the same Rust-owned approval as today.

- [ ] **Step 1: Write the failing tests.** For each page, the table of rows the matching GPUI file renders for a fixed fixture; `recheck` clears and refetches the provider status; an agent save with a looser policy returns the approval-required error.
- [ ] **Step 2: Run** both test commands. Expected: FAIL.
- [ ] **Step 3: Implement** parity with `crates/discard-ui/src/settings/{providers,agents,workspaces,memory}.rs`.
- [ ] **Step 4: Run** both test commands. Expected: PASS.
- [ ] **Step 5: Commit** `feat: add the Providers, Agents, Workspaces, and Memory pages`.

### Task 12: Keybindings, Plugins, Logs, Advanced pages

**Files:**
- Create: `web/src/settings/pages/{Keybindings,Plugins,Logs,Advanced}.vue`, `desktop/settings/{keybindings.rs,plugins.rs,logs.rs,advanced.rs}`
- Test: `web/src/settings/pages2.test.ts`, unit tests in each Rust file

**Interfaces:**
- Consumes: `keymap::{check_change, rows}`, `PluginService` (`pending`, approve, reject, rollback, trust), `log_view`, `advanced::AdvancedAction`.
- Produces: commands `settings_keymap_rows()`, `settings_keymap_set(action_id: String, chord: String) -> Result<(), String>` (rebuilds the native menu on success), `settings_plugins() -> PluginsDto`, `settings_plugin_approve|reject|rollback(review_id: String)`, `settings_plugin_trust(review_id: String, trusted: bool)`, `settings_logs(filter: LogFilterDto) -> Vec<LogRowDto>`, `settings_advanced_run(action: String, confirmed: bool) -> Result<String, String>`. Plugin approval screens show the diff and the capability list. Approve, reject, and rollback bump the plugin epoch so the main window re-renders slots.

- [ ] **Step 1: Write the failing tests.** Key capture: a chord that conflicts returns the `check_change` error and keeps the old binding; "trust future edits" toggle shows only when the plugin declares `ui` alone; a plugin in "modified outside review" state offers diff and rollback and no approve; an Advanced action without `confirmed: true` returns the confirm prompt and does nothing; log filters match `log_view` tests.
- [ ] **Step 2: Run** both test commands. Expected: FAIL.
- [ ] **Step 3: Implement** parity with `crates/discard-ui/src/settings/{keybindings,plugins,logs,advanced}.rs`. The plugin review UI uses fixed components and shows plugin text as plain text only.
- [ ] **Step 4: Run** both test commands. Expected: PASS. Walk through one agent-authored plugin review with `--builder`.
- [ ] **Step 5: Commit** `feat: add the Keybindings, Plugins, Logs, and Advanced pages`.

---

## Phase 4: Cutover

### Task 13: Remove GPUI and update the docs

**Files:**
- Delete: `crates/discard-ui/`
- Modify: `Cargo.toml` (drop member), `crates/discard-app/Cargo.toml` (drop `discard-ui`), `README.md`, `PLAN.md` (lines 7, 19, 85, 99 and anything naming GPUI), `docs/settings.md`, `docs/ui-juice-spec.md` (add a "Superseded by the Tauri build: Vue components in `web/`" note), `THIRD-PARTY-NOTICES`, `.gitignore`
- Test: none new. This task is verified by the full gate.

**Interfaces:**
- Consumes: everything above.
- Produces: a repo with no `gpui` or `gpui-kit` reference (`rg -i gpui` returns only the superseded note and git history).

- [ ] **Step 1: Run** `rg -il 'gpui' --glob '!target' --glob '!Cargo.lock'` and list every hit.
- [ ] **Step 2: Delete** `crates/discard-ui`, remove the dependency lines, update the docs, and regenerate `THIRD-PARTY-NOTICES` for the new dependency set (Rust and `web/` production dependencies).
- [ ] **Step 3: Run the full gate**: `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `pnpm --dir web lint`, `pnpm --dir web typecheck`, `pnpm --dir web test`, `pnpm --dir web build`, `cargo tauri build --debug` (or `cargo build -p discard-app --release` if the CLI is absent). Expected: all pass.
- [ ] **Step 4: Update the README** run and test sections: `cargo run -p discard-app` still works; add `pnpm --dir web dev` with `cargo tauri dev`; add the web test commands; update the layout table (`discard-ui` row removed, `web/` row added).
- [ ] **Step 5: Commit** `refactor: remove the GPUI front end`.

---

## Self-Review

- **Spec coverage:** main window (Tasks 5 to 8), Settings pages: General, Appearance, Providers, Agents, Workspaces, Memory, Keybindings, Plugins, Logs, Advanced (10 to 12, matching the 10 files in `settings/`), menus and keys (9), theme (4), window state (5), read marker (6), protected surfaces (7), plugin renderer (8), cutover (13). `discard-lua`'s `api_spec.rs` docs mention no GPUI-specific behavior I found: Task 13 Step 1 will confirm.
- **Type consistency:** `SnapshotDto`, `MessageDto`, `ThemeDto`, `BannersDto`, `SlotDto` are defined in Task 2 and used by name in Tasks 3 to 9. Commands use `snake_case` on the Rust side. TS wrappers use `camelCase`.
- **Proportion:** Settings tasks lean on "parity with the named GPUI file" instead of restating behavior, because those files are the spec.
- **Open choices the implementer may make:** shadcn-vue component set, the exact shiki theme mapping, and the debounce values.
- **Known limits:** `cargo-tauri` is not installed on this machine (`cargo install tauri-cli --version '^2'`). Window drag regions and the native traffic-light inset need a manual look.
