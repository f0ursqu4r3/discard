# Project Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A project opens in its own window with no project rail, each app window holds its own selection, and the app keeps running in the background with a tray icon.

**Architecture:** `AppCore` keeps one pool of loaded channels and one `View` per app window, keyed by window label. Commands act on the view of the calling window, and the pump sends each window the snapshot of its own view. The main window hides on close; a tray icon (Windows, Linux) or the Dock (macOS) brings it back.

**Tech Stack:** Rust, Tauri 2 (`tray-icon` feature), Vue 3, TypeScript, Vitest (`vp test`).

**Spec:** `docs/superpowers/specs/2026-11-16-project-window-design.md`. Decision numbers below (for example "spec 17a") refer to it. Read it before each task.

## Global Constraints

- Work on the branch `kyle/project-window`. Commit after each task. Do not add a `Co-Authored-By` line or any tool attribution to a commit.
- AGENTS.md: a new control is implemented, or it is a planned item with `Soon` and a `TODO`. This plan adds no planned item and no no-op handler.
- Web tests go in a `__tests__` directory beside the code they test.
- One agent session per channel. No task may load a channel twice.
- The main window behaves as today when no project window is open and the background setting is off.
- Exact copy from the spec: `Open in New Window`, `Main Window`, `Quit Discard`, `Keep running in the background when all windows are closed`, `This terminal is open in another window.`, `Show Here`.
- Window label of a project window: `project-<project ID>`. URL: `index.html#/project/<project ID>`. Start size 1100 × 750 logical points. Offset from the main window: 32 points on each axis.
- Proof commands, run after every task:
  - `cargo test -p discard-core -p discard-app`
  - `cargo clippy --workspace --all-targets`
  - `pnpm --dir web test` and `pnpm --dir web check` (for tasks that touch `web/`)

## Review Focus

Each line has a test in the task named in brackets.

1. Two windows show one channel and the user decides an approval in one: the prompt must leave the other window too. [Task 3]
2. A webview reload of a project window (dev hot reload): `get_snapshot` must return the view of that window, not the main view. [Task 3]
3. A project window closes while a project or channel switch for its view is in flight: no panic, no view created again, no change to another view. [Task 4]
4. A message read in one window: the unread marker must clear in every window that lists the channel. [Task 4]
5. A project is renamed or removed while its window is open: the title follows the name; the window closes on removal. [Task 5]

## File Structure

| File | Responsibility |
|---|---|
| `crates/discard-app/src/desktop/view.rs` (new) | `ViewId`: the mapping between window labels and views. |
| `crates/discard-app/src/desktop/core.rs` | Channel pool, `View`, per-view snapshot, guards. |
| `crates/discard-app/src/desktop/channels.rs` | Channel and project operations for a given view; routing to the main view. |
| `crates/discard-app/src/desktop/commands.rs` | Commands resolve the view from the calling window. |
| `crates/discard-app/src/desktop/pump.rs`, `mod.rs` | Per-view emit, window events, focus, close rule. |
| `crates/discard-app/src/desktop/windows.rs` | Open, show, and count app windows. |
| `crates/discard-app/src/desktop/plan.rs`, `actions.rs` | Close plan, `ShowMainWindow`, actions on the focused view. |
| `crates/discard-app/src/desktop/tray.rs` (new) | Tray icon, tray menu, background rule. |
| `crates/discard-app/src/desktop/terminal.rs`, `terminal/session.rs` | Terminal owner window and the move event. |
| `crates/discard-core/src/action.rs`, `settings.rs` | `Action::ShowMainWindow`, `keep_running_in_background`. |
| `web/src/lib/window-mode.ts` (new) | Parse the window mode from the URL hash. |
| `web/src/App.vue`, `components/workspace/NavigationPanel.vue`, `components/project-menu.ts`, `bridge/api.ts` | Project mode, menu item, bridge calls. |
| `web/src/settings/pages/General.vue` | Background setting. |

---

### Task 1: Channel pool

Merge `State.active` and `State.parked` into one map. No behavior change and no signature change.

**Files:**
- Modify: `crates/discard-app/src/desktop/core.rs`

**Interfaces:**
- Produces: `State.channels: BTreeMap<String, ChannelState>` (all loaded channels, keyed by channel ID) and `State.selected: String` (the ID of the selected channel; empty when none). Private helpers `fn selected(&State) -> Option<&ChannelState>` and a mutable form.

- [ ] **Step 1: Write the failing test** in the test module of `core.rs`, with the existing helpers for a core with two channels:

```rust
#[test]
fn a_switch_keeps_both_channels_in_the_pool() {
    let core = core_with_channels(&["a", "b"]); // selected: "a"
    core.switch_channel("b").unwrap();
    assert_eq!(core.active_channel_id(), "b");
    assert!(core.has_channel("a") && core.has_channel("b"));
    assert_eq!(core.loaded_channel_count(), 2);
}
```

- [ ] **Step 2:** Run `cargo test -p discard-app a_switch_keeps_both`. Expected: FAIL, `loaded_channel_count` not found.
- [ ] **Step 3:** Replace `active` and `parked` with `channels` and `selected`. Add `pub fn loaded_channel_count(&self) -> usize`. Each `once(&s.active).chain(s.parked.values())` becomes `s.channels.values()`. A read of the selected channel with no channel selected uses an empty `ChannelState`, as `clear_project` builds today.
- [ ] **Step 4:** Run the proof commands. Expected: all existing tests PASS.
- [ ] **Step 5:** Commit: `refactor: keep loaded channels in one pool`.

### Task 2: Views in the core

Move the selection into `View`. Only the `main` view exists after this task.

**Files:**
- Create: `crates/discard-app/src/desktop/view.rs`
- Modify: `crates/discard-app/src/desktop/core.rs`, `mod.rs` (module line), and every caller of a changed method (`channels.rs`, `commands.rs`, `actions.rs`, `pump.rs`, `threads.rs`, `settings/*.rs`, `main.rs`, `plugin_memory.rs`, `tests/*.rs`)

**Interfaces:**
- Produces, in `view.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewId(String);
impl ViewId {
    pub fn main() -> ViewId;                                  // label "main"
    pub fn project(id: &ProjectId) -> ViewId;                 // label "project-<id>"
    pub fn from_label(label: &str) -> Option<ViewId>;         // None for a label that is not an app window
    pub fn label(&self) -> &str;
    pub fn is_main(&self) -> bool;
    pub fn project_id(&self) -> Option<ProjectId>;
}
```

- Produces, in `core.rs`: `State.views: BTreeMap<ViewId, View>`. `View` holds: project name, ID, and path; `selected`; `open_thread`; the branch tree fields (`branches`, `branch_repositories`, `repositories`, `repositories_loading`, `current_branch`, `collapsed`); `branch_source`; `branch_generation`; `navigation_busy`; `panels`; `member_list_visible`; a slot cache.
- Rule for signatures: **a method whose result depends on the selected project, channel, thread, or branch tree takes `view: &ViewId` as its first parameter. A method that names its channel does not change.** This covers at least: `snapshot`, `history_snapshot`, `history`, `thread_history_snapshot`, `active_channel_id`, `active_agent_id`, `active_context`, `current_project`, `select_sidebar_project`, `switch_channel`, `switch_channel_for_project`, `can_switch`, `can_switch_for_project`, `replace_project`, `clear_project`, `open_thread`, `close_thread`, `open_thread_id`, `toggle_branch`, `set_branches`, `configure_branch_source`, `clear_branch_source`, `request_branch_refresh`, `refresh_branches`, `refresh_branches_in_background`, `reselect_current_branch`, `branch_refresh_pending`, `configure_targets`, `configure_read_marker`, `set_panels`, `set_member_list_visible`, `menu_context`, `resolve_copy`, `notice`, `plugin_action`, `run_plugin_menu`, `archive_started`, and the forms of `send`, `decide`, `cancel`, `mark_read` that take no channel.
- A method called with a view that does not exist returns `Err("This window is closed.")` when it returns a `Result`, and does nothing otherwise. It never creates a view.
- Guards (spec 21): `begin_project_navigation(view)` and new `end_project_navigation(view)` use `View.navigation_busy`. `begin_project_change`, `begin_write_approval`, and `end_project_change` keep an app-wide flag, `State.busy`. `SnapshotDto.archive_busy` is true when either flag or the archive flag of the channel is set.
- `State.rev`, `State.history_rev`, `confirm`, `window` (the saved state), read markers, threads, and `any_running` / `cancel_all` stay app-wide.
- In this task every caller passes `&ViewId::main()`.

- [ ] **Step 1: Write the failing tests** in `view.rs`:

```rust
#[test]
fn labels_round_trip() {
    let id = ProjectId("0193a1b2-aaaa-7bbb-8ccc-0123456789ab".into());
    let view = ViewId::project(&id);
    assert_eq!(view.label(), "project-0193a1b2-aaaa-7bbb-8ccc-0123456789ab");
    assert_eq!(ViewId::from_label(view.label()), Some(view.clone()));
    assert_eq!(view.project_id(), Some(id));
    assert!(ViewId::from_label("main").unwrap().is_main());
    assert_eq!(ViewId::from_label("settings"), None);
}
#[test]
fn a_project_label_is_a_valid_tauri_label() {
    let view = ViewId::project(&ProjectId("a b/c".into()));
    assert!(view.label().chars().all(|c| c.is_ascii_alphanumeric() || "-/:_".contains(c)));
    assert_eq!(view.project_id(), Some(ProjectId("a b/c".into())));
}
```

- [ ] **Step 2:** Run `cargo test -p discard-app labels_round_trip a_project_label`. Expected: FAIL to compile.
- [ ] **Step 3:** Implement `ViewId`. If a project ID has a character outside `[A-Za-z0-9-/:_]`, encode the ID so that the label is valid and `project_id()` returns the original.
- [ ] **Step 4:** Write the failing core test:

```rust
#[test]
fn a_missing_view_is_an_error_and_is_not_created() {
    let core = core_with_channels(&["a", "b"]);
    let gone = ViewId::project(&ProjectId("p".into()));
    assert_eq!(core.switch_channel(&gone, "b").unwrap_err(), "This window is closed.");
    assert_eq!(core.view_ids(), vec![ViewId::main()]);
    assert_eq!(core.active_channel_id(&ViewId::main()), "a");
}
```

- [ ] **Step 5:** Move the selection fields into `View`, apply the signature rule, add `pub fn view_ids(&self) -> Vec<ViewId>` and `pub fn has_view(&self, view: &ViewId) -> bool`, and update every caller to pass `&ViewId::main()`.
- [ ] **Step 6:** Run the proof commands. Expected: PASS.
- [ ] **Step 7:** Commit: `refactor: hold the selection in a view per window`.

### Task 3: Window-scoped commands and per-view emit

**Files:**
- Modify: `crates/discard-app/src/desktop/core.rs`, `channels.rs`, `commands.rs`, `threads.rs`, `pump.rs`, `mod.rs`, `tests/core_flow.rs`, `tests/threads.rs`, `tests/delegation.rs`

**Interfaces:**
- Consumes: `ViewId`, the view-taking core methods (Task 2).
- Produces:
  - `pub fn open_view(&self, view: ViewId, project: &Project) -> bool` on `AppCore`: adds a view fixed to `project`, with the saved panel widths and member list visibility. Returns false and changes nothing if the view exists. `pub fn drop_view(&self, view: &ViewId)`: removes a view; a no-op for `main`. It cancels no turn and unloads no channel.
  - `Channels` methods that use the selection take `view: &ViewId` first (`activate_project`, `select`, `select_inner`, `select_inner_mode`, `open_general`, `create`, `restore`, and the rest by the Task 2 rule).
  - In `commands.rs`: `fn view_of(window: &tauri::WebviewWindow, core: &AppCore) -> Result<ViewId, String>`. An app window label with a view gives that view. An app window label with no view gives `Err("This window is closed.")`. Any other label (the settings window) gives `core.focused_view()`, which returns `ViewId::main()` until Task 6.
  - Every command that used the selection takes `window: tauri::WebviewWindow` and uses `view_of`.
  - `Emit` (`pump.rs`): `fn snapshot(&self, view: &ViewId, s: &SnapshotDto)`, `fn history(&self, view: &ViewId, h: &HistoryDto)`, `fn thread_history(&self, view: &ViewId, t: &ThreadHistoryDto)`. `conversations()` does not change.
  - `emit_current` loops over `core.view_ids()` and tracks the history and thread revisions per view.
  - `WebEmit` sends the three events with `emit_to(view.label(), …)`. The native menu update runs for the `main` view only until Task 6.

- [ ] **Step 1: Write the failing tests** in `core.rs`:

```rust
#[test]
fn views_select_independently() {
    let core = core_with_project_channels("p", &["a", "b"]); // main selects "a"
    let other = ViewId::project(&ProjectId("p".into()));
    assert!(core.open_view(other.clone(), &project("p")));
    core.switch_channel(&other, "b").unwrap();
    assert_eq!(core.active_channel_id(&ViewId::main()), "a");
    assert_eq!(core.snapshot(&other).channel_id, "b");
    assert_eq!(core.snapshot(&ViewId::main()).channel_id, "a");
}
#[test]
fn a_dropped_view_stops_no_turn_and_unloads_no_channel() {
    let core = core_with_project_channels("p", &["a", "b"]);
    let other = ViewId::project(&ProjectId("p".into()));
    core.open_view(other.clone(), &project("p"));
    core.switch_channel(&other, "b").unwrap();
    core.send_to(Some("b"), "hello".into()).unwrap();
    core.drop_view(&other);
    assert!(core.channel_running("b"));
    assert_eq!(core.loaded_channel_count(), 2);
    core.drop_view(&ViewId::main());
    assert!(core.has_view(&ViewId::main()));
}
#[test]
fn a_decision_in_one_view_clears_the_prompt_in_the_other() { // Review Focus 1
    // Both views select "a". Raise an approval in "a" with the existing approval helper.
    // assert_eq!(core.snapshot(&main).prompts.len(), 1); assert_eq!(core.snapshot(&other).prompts.len(), 1);
    // core.decide_in(Some("a"), approval_id, "allow".into(), None);
    // assert!(core.snapshot(&main).prompts.is_empty() && core.snapshot(&other).prompts.is_empty());
}
```

and in `tests/core_flow.rs`, with a recorder that stores `(ViewId, SnapshotDto)` and `(ViewId, HistoryDto)`:

```rust
#[tokio::test]
async fn each_view_receives_its_own_snapshot_and_history() {
    // main selects "a", the project view selects "b"; run the pump once.
    // Every recorded snapshot for `main` has channel_id "a"; every one for the project view has "b".
    // Every recorded history for a view has the channel_id of that view.
}
```

- [ ] **Step 2:** Run them. Expected: FAIL to compile (`open_view`, the `Emit` signature).
- [ ] **Step 3:** Implement the interfaces above.
- [ ] **Step 4:** Add a `commands.rs` test for Review Focus 2 on the pure part of `view_of`: extract `fn resolve_view(label: &str, has_view: impl Fn(&ViewId) -> bool, focused: ViewId) -> Result<ViewId, String>` and assert: `"project-p"` with a view gives that view; `"project-p"` with no view gives `Err("This window is closed.")`; `"settings"` gives `focused`.
- [ ] **Step 5:** Run the proof commands. Expected: PASS.
- [ ] **Step 6:** Commit: `feat: scope commands and events to the calling window`.

### Task 4: Rules of a project view

**Files:**
- Modify: `crates/discard-app/src/desktop/core.rs`, `channels.rs`, `commands.rs`, `slots.rs`

**Interfaces:**
- Consumes: `open_view`, `drop_view`, view-taking `Channels` methods (Task 3).
- Produces:
  - `pub fn route(&self, view: &ViewId, target_project: Option<&ProjectId>) -> ViewId` on `AppCore`: returns `view` if it is `main` or if `target_project` is the project of the view; otherwise `ViewId::main()` (spec 17, 17b).
  - `Channels::select(&self, view: &ViewId, id: &str) -> Result<ViewId, String>` and `Channels::activate_project(&self, view: &ViewId, id: &ProjectId) -> Result<(ViewId, String), String>`: they apply `route` and return the view that changed.
  - `pub fn open_project_view(&self, project: &ProjectId) -> Result<ViewId, String>` on `Channels`: opens the view and selects the saved channel of the project (`project_channel_or_fallback`), spec 3. If the view exists it returns it unchanged.
  - `pub fn views_selecting(&self, channel: &str) -> Vec<ViewId>` on `AppCore`.
  - `choose_project_folder` uses the project of the calling view, not `store.selected_project()` (spec 21b).

**Rules to implement:**
- Spec 20: `replace_project` and `clear_project` change one view. They remove no channel that another view selects.
- Spec 21a: `snapshot(view)` of a project view lists only the channels and thread approvals of its project. The main view lists all, as today.
- Spec 22: where the code opens `#general` because the selected channel became unavailable (`channels.rs`, the `open_general` fallback), do it for every view in `views_selecting(channel)`.
- Spec 24 to 26: only the main view writes `selected_conversation`, bounds, panel widths, member list visibility, and `store.select_project`. A project view keeps panel changes in memory. A channel selection in any view saves the selected channel of the project (`select_project_channel`).
- Spec 23: read markers stay shared.
- Spec 40: one slot cache per view. `clear_slots()` clears all of them.

- [ ] **Step 1: Write the failing tests** (`core.rs` and the test module of `channels.rs`, with its existing fixture):

```rust
#[test]
fn a_target_outside_the_project_routes_to_the_main_view() {
    // Projects "p" and "q"; a project view on "p".
    assert_eq!(core.route(&p_view, Some(&ProjectId("p".into()))), p_view);
    assert_eq!(core.route(&p_view, Some(&ProjectId("q".into()))), ViewId::main());
    assert_eq!(core.route(&p_view, None), ViewId::main()); // a direct message
    // channels.select(&p_view, q_channel) returns ViewId::main(); main selects q_channel; p_view is unchanged.
}
#[test]
fn a_project_view_lists_only_its_project() {
    // snapshot(&p_view).channels all have project_id Some("p"); snapshot(&main) has channels of "p" and "q".
}
#[test]
fn an_unavailable_channel_falls_back_in_every_view() {
    // main and p_view select text channel "notes" of "p". Archive "notes".
    // Both views then select #general of "p".
}
#[test]
fn a_project_view_does_not_change_saved_window_state() {
    // p_view: switch channel, set_panels, set_member_list_visible.
    // core.saved_conversation() and the saved panels are unchanged; snapshot(&p_view).panels has the new widths.
}
#[test]
fn a_read_in_one_view_clears_unread_in_the_other() { // Review Focus 4
    // "a" has an unread message. main selects "b"; p_view selects "a".
    // core.mark_read_in(Some("a"), last_message_id).unwrap();
    // The ChannelDto of "a" in snapshot(&main).channels has unread == false.
}
#[test]
fn a_view_dropped_during_navigation_stays_dropped() { // Review Focus 3
    // core.begin_project_navigation(&p_view).unwrap(); core.drop_view(&p_view);
    // channels.activate_project(&p_view, &p) returns Err("This window is closed.");
    // core.end_project_navigation(&p_view) does not panic; core.view_ids() == [main]; main is unchanged.
}
```

- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement the interfaces and rules.
- [ ] **Step 4:** Run the proof commands. Expected: PASS.
- [ ] **Step 5:** Commit: `feat: fix a project view to its project`.

### Task 5: Project window

**Files:**
- Modify: `crates/discard-app/src/desktop/windows.rs`, `commands.rs`, `mod.rs` (command registration, window events), `crates/discard-app/capabilities/default.json`

**Interfaces:**
- Consumes: `Channels::open_project_view`, `AppCore::drop_view`, `ViewId` (Tasks 2 to 4).
- Produces:
  - `pub fn project_window_position(main: Option<Bounds>) -> Option<(f32, f32)>`: the position of the main window plus 32 on each axis; `None` when the main bounds are unknown.
  - `pub fn open_project_window(app: &tauri::AppHandle, channels: &Arc<Channels>, project: &ProjectId) -> Result<(), String>`: if the window exists, show and focus it. Otherwise call `open_project_view`, then build the window: label `ViewId::project(project).label()`, URL `index.html#/project/<id>`, title = project name, inner size 1100 × 750, and the options of the main window in `tauri.conf.json` (`titleBarStyle: Overlay`, hidden title, traffic light position x 12 y 20, drag and drop disabled). If the build fails, drop the view.
  - Command `open_project_window(project_id: String) -> Result<(), String>`.
  - `pub fn app_window_labels(app: &tauri::AppHandle) -> Vec<String>`: labels for which `ViewId::from_label` is `Some`.
  - `pub fn show_view_window(app: &tauri::AppHandle, view: &ViewId)`: show and focus the window of the view.
- Window events in `mod.rs`: `WindowEvent::Destroyed` of a project window calls `core.drop_view`. A close request of a project window is allowed without confirmation (the full close rule is Task 6).
- `update_project_profile`: after a rename, set the title of the project window of that project.
- `remove_project`: close the project window of that project before the removal.
- Capability file: `"windows": ["main", "settings", "project-*"]`.

- [ ] **Step 1: Write the failing tests** in `windows.rs`:

```rust
#[test]
fn a_project_window_opens_32_points_from_the_main_window() {
    let main = Bounds { x: 100.0, y: 80.0, width: 1200.0, height: 800.0 };
    assert_eq!(project_window_position(Some(main)), Some((132.0, 112.0)));
    assert_eq!(project_window_position(None), None);
}
```

and in `channels.rs`:

```rust
#[test]
fn a_project_view_opens_on_the_saved_channel_of_the_project() {
    // Project "p" with saved channel "notes"; main selects a channel of "q".
    // let view = channels.open_project_view(&p).unwrap();
    // core.active_channel_id(&view) == notes; core.current_project(&view).id == p; main is unchanged.
    // A second open_project_view(&p) returns the same view and changes no selection.
}
```

- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement the interfaces, the window events, the title update, the close on removal, and the capability change.
- [ ] **Step 4:** Add tests for Review Focus 5 on pure helpers: `pub fn project_window_title(name: &str) -> String` returns the name; and a `channels.rs` test that `core.current_project(&view).name` follows `update_project_name`.
- [ ] **Step 5:** Run the proof commands. Expected: PASS.
- [ ] **Step 6:** Commit: `feat: open a project in its own window`.

### Task 6: Focus, close rule, and the Main Window action

**Files:**
- Modify: `crates/discard-core/src/action.rs` (and `keymap.rs` if it lists actions), `crates/discard-app/src/desktop/plan.rs`, `actions.rs`, `mod.rs`, `windows.rs`, `commands.rs`, `core.rs`, `settings/workspaces.rs`, `settings/agents.rs`, `crates/discard-app/src/main.rs`

**Interfaces:**
- Produces:
  - `AppCore::set_focused_view(&self, view: &ViewId)` and `AppCore::focused_view(&self) -> ViewId` (the last focused view that still exists, else `main`).
  - `Action::ShowMainWindow`: ID `show_main_window`, label `Main Window`, no default chord, in the Window menu after `Zoom` with a separator before it. Available on all platforms.
  - In `plan.rs`:

```rust
pub enum WindowKind { Main, Project, Other }
pub enum Close { Allow, Hide, Quit, Ask(Confirm) }
pub fn close_plan(kind: WindowKind, other_app_windows: usize, background: bool, turn_running: bool, confirm_while_working: bool) -> Close;
```

  Rules: `Other` → `Allow`. If `other_app_windows > 0 || background`: `Main` → `Hide`, `Project` → `Allow`. Otherwise (the last app window, no background): `Ask(Confirm::Quit)` when `needs_confirm(Action::Quit, …)`, else `Quit`.
  - `windows::visible_app_windows(app) -> usize`, `windows::show_main_window(app)` (show and focus `main`), `windows::show_conversation(app, view: &ViewId)`.
  - Command `show_main_window(page: Option<String>)`: shows the main window; if `page` is set, emits `discard://navigate` with the page to `main`.
  - `actions::run(app, core, actions, view: &ViewId, action, target)`.

**Rules to implement:**
- `WindowEvent::Focused(true)` on an app window sets the focused view and wakes the pump.
- Close request: `Hide` prevents the close and hides the main window. `Quit` runs the existing exit path (`cancel_all`, set `exiting`, `app.exit(0)`). `Ask` prevents the close and asks. In this task `background` is `false`.
- Spec 34, 35: the native menu context comes from the focused view. Menu events run on the focused view. Window actions (Close Window, Minimize, Zoom, Toggle Terminal, Toggle Member List, agent profile) use the window of the view, not the label `main`.
- Spec 36: `banners.confirm` is set only in the snapshot of the focused view.
- Spec 30: `discard://conversation-opened` goes to the window of the view that changed. When `Channels::select` or `activate_project` returns a view that is not the view of the caller, call `show_view_window` for it.
- Spec 27: a Settings action that opens a conversation uses the main view and `show_main_window`.
- Spec 37, 41: `settings/workspaces.rs`, `settings/agents.rs`, the plugin UI state, and the project source of plugin memory (`main.rs`) read the focused view.
- Spec 7b: `Plan::ShowMainWindow` calls `show_main_window`. On macOS, `RunEvent::Reopen` with no visible window does the same.

- [ ] **Step 1: Write the failing tests** in `plan.rs`:

```rust
#[test]
fn a_window_that_is_not_the_last_closes_alone() {
    assert_eq!(close_plan(WindowKind::Main, 1, false, true, true), Close::Hide);
    assert_eq!(close_plan(WindowKind::Project, 1, false, true, true), Close::Allow);
    assert_eq!(close_plan(WindowKind::Other, 0, false, true, true), Close::Allow);
}
#[test]
fn the_last_app_window_follows_the_quit_rule() {
    for kind in [WindowKind::Main, WindowKind::Project] {
        assert_eq!(close_plan(kind, 0, false, true, true), Close::Ask(Confirm::Quit));
        assert_eq!(close_plan(kind, 0, false, true, false), Close::Quit);
        assert_eq!(close_plan(kind, 0, false, false, true), Close::Quit);
    }
}
#[test]
fn the_last_app_window_closes_alone_in_background_mode() {
    assert_eq!(close_plan(WindowKind::Main, 0, true, true, true), Close::Hide);
    assert_eq!(close_plan(WindowKind::Project, 0, true, true, true), Close::Allow);
}
```

in `action.rs`:

```rust
#[test]
fn the_window_menu_has_main_window() {
    let window = menu_bar(&MenuContext::default()).into_iter().find(|m| m.title == "Window").unwrap();
    assert!(window.entries.iter().any(|e| matches!(e, MenuEntry::Action { action: Action::ShowMainWindow, label, enabled: true, .. } if label == "Main Window")));
    assert_eq!(Action::from_id("show_main_window"), Some(Action::ShowMainWindow));
}
```

and in `core.rs`:

```rust
#[test]
fn the_confirmation_shows_in_the_focused_view_only() {
    // main and p_view exist. core.set_focused_view(&p_view); core.ask(Confirm::Quit);
    // snapshot(&p_view).banners.confirm.is_some(); snapshot(&main).banners.confirm.is_none();
    // core.drop_view(&p_view); core.focused_view() == ViewId::main().
}
```

- [ ] **Step 2:** Run them. Expected: FAIL to compile.
- [ ] **Step 3:** Implement the interfaces and rules. Update the existing `close_plan` tests and callers to the new signature.
- [ ] **Step 4:** Run the proof commands. Expected: PASS.
- [ ] **Step 5:** Commit: `feat: follow the focused window and hide the main window on close`.

### Task 7: Web project mode

**Files:**
- Create: `web/src/lib/window-mode.ts`, `web/src/lib/__tests__/window-mode.test.ts`
- Modify: `web/src/App.vue`, `web/src/components/workspace/NavigationPanel.vue`, `web/src/components/project-menu.ts`, `web/src/bridge/api.ts`
- Test: `web/src/components/__tests__/project-menu.test.ts`, `web/src/components/workspace/__tests__/NavigationPanel.test.ts`, `web/src/__tests__/App.project-window.test.ts` (new)

**Interfaces:**
- Consumes: commands `open_project_window`, `show_main_window`; event `discard://navigate` (Tasks 5, 6).
- Produces:

```ts
// window-mode.ts
export type WindowMode = { kind: 'main' } | { kind: 'project'; projectId: string }
export function windowMode(hash: string): WindowMode
// api.ts
export const openProjectWindow = (projectId: string) => invoke<void>('open_project_window', { projectId })
export const showMainWindow = (page?: string) => invoke<void>('show_main_window', { page })
```

  - `ProjectMenuAction` gains `'open-window'`. `projectMenu` has `{ kind: 'action', label: 'Open in New Window', action: 'open-window' }` directly above `Reveal in Finder`. It has no `disabled` field.
  - `NavigationPanel` gains the prop `rail?: boolean` (default `true`). With `rail === false` it does not mount `ServerRail`; in the compact layout the strip holds `UserBar` only.

**Rules to implement:**
- `App.vue` reads `windowMode(location.hash)` once. In project mode it passes `:rail="false"`, does not call the rail layout loader, and does not listen for rail changes.
- `projectAction`: `open-window` calls `openProjectWindow(project.id)`. It does not select the project in the calling window.
- Spec 17a, web part: in project mode, a navigation to the page `direct-messages` (from `show_friends` and from `discard://agent-profile` with no agent ID) calls `showMainWindow('direct-messages')` and leaves the page of the project window unchanged. Put this check in the one `navigate` function.
- In main mode, `App.vue` listens for `discard://navigate` and navigates to the page if it is a known page.

- [ ] **Step 1: Write the failing tests:**

```ts
// window-mode.test.ts
expect(windowMode('')).toEqual({ kind: 'main' })
expect(windowMode('#/project/abc-123')).toEqual({ kind: 'project', projectId: 'abc-123' })
expect(windowMode('#/project/')).toEqual({ kind: 'main' })

// project-menu.test.ts
const labels = projectMenu(true).map((e) => ('label' in e ? e.label : e.kind))
expect(labels.indexOf('Open in New Window')).toBe(labels.indexOf('Reveal in Finder') - 1)
const entry = projectMenu(true).find((e) => e.kind === 'action' && e.action === 'open-window')
expect(entry).toMatchObject({ kind: 'action', label: 'Open in New Window' })
expect(entry && 'disabled' in entry && entry.disabled).toBeFalsy()

// NavigationPanel.test.ts
// mount with rail: false → wrapper.find('[data-ui-id="server-rail"]').exists() is false (use the rail's real data-ui-id);
// with no rail prop → it exists.

// App.project-window.test.ts (mock the bridge as App.project-menu.test.ts does; set location.hash = '#/project/p1')
// 1. No rail, no direct messages entry, no Add Project control in the mounted app.
// 2. getRailLayout is not called.
// 3. Emitting the `show_friends` action calls showMainWindow('direct-messages') and the page stays.
// And in main mode: the project action 'open-window' calls openProjectWindow('p1') and does not call selectProject.
```

- [ ] **Step 2:** Run `pnpm --dir web test`. Expected: the new tests FAIL.
- [ ] **Step 3:** Implement the interfaces and rules.
- [ ] **Step 4:** Run `pnpm --dir web test` and `pnpm --dir web check`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat: add the project window mode and its menu item`.

### Task 8: Background mode and tray icon

**Files:**
- Create: `crates/discard-app/src/desktop/tray.rs`
- Modify: `crates/discard-app/Cargo.toml` (`tauri` features: `["image-png", "tray-icon"]`), `crates/discard-core/src/settings.rs`, `crates/discard-app/src/desktop/settings/mod.rs`, `mod.rs`, `actions.rs`, `app_icon.rs`, `web/src/settings/pages/General.vue`
- Test: the General page test in `web/src/settings/pages/__tests__/` (create it if the directory has none)

**Interfaces:**
- Consumes: `close_plan(.., background, ..)`, `show_main_window`, `visible_app_windows` (Task 6).
- Produces:
  - `GeneralSettings.keep_running_in_background: bool`, default `true`, TOML key `general.keep_running_in_background`. `Change::KeepRunningInBackground(bool)`. `GeneralPatch.keep_running_in_background: Option<bool>`. The `general` settings DTO gains `keepRunningInBackground: boolean` and `backgroundUnavailable: string | null`.
  - In `tray.rs`:

```rust
/// Whether the app stays alive with no app window (spec T1, T3, T7, T8).
pub fn background_enabled(setting: bool, macos: bool, tray_ok: bool) -> bool { setting && (macos || tray_ok) }
/// The line the General page shows when the tray icon failed.
pub fn unavailable_line(error: &str) -> String; // "The tray icon is not available: {error}. Discard quits when the last window closes."
/// Managed state: the result of the tray creation. `Ok(())` on macOS, which has no tray icon.
pub struct TrayState(pub Result<(), String>);
#[cfg(not(target_os = "macos"))]
pub fn create(app: &tauri::AppHandle) -> Result<(), String>; // tray ID "main"
```

**Rules to implement:**
- Spec T2, T4: on Windows and Linux, create the tray at setup. Menu: `Main Window`, a separator, `Quit Discard`. `Main Window` calls `show_main_window`. `Quit Discard` runs `Action::Quit` through `actions::run` on the focused view. On Windows a left click on the icon calls `show_main_window`.
- Spec T5: `app_icon::apply` also sets the tray icon when a tray exists.
- Spec T6: when `Action::Quit` plans `Ask` and `visible_app_windows(app) == 0`, call `show_main_window` and set the focused view to `main` before the ask.
- The close handler passes `background_enabled(setting, cfg!(target_os = "macos"), tray_state.0.is_ok())` to `close_plan`.
- General page: a switch with the label `Keep running in the background when all windows are closed`. When `backgroundUnavailable` is not null, show that line under the switch.

- [ ] **Step 1: Write the failing tests** in `tray.rs`:

```rust
#[test]
fn background_needs_the_setting_and_a_way_back() {
    assert!(background_enabled(true, true, false));   // macOS: the Dock
    assert!(background_enabled(true, false, true));   // tray icon
    assert!(!background_enabled(true, false, false)); // no tray icon
    assert!(!background_enabled(false, true, true));  // setting off
}
#[test]
fn the_unavailable_line_names_the_error() {
    assert_eq!(unavailable_line("no appindicator"), "The tray icon is not available: no appindicator. Discard quits when the last window closes.");
}
```

in `settings.rs`:

```rust
#[test]
fn keep_running_in_background_defaults_to_on_and_saves() {
    // Default settings: general.keep_running_in_background == true.
    // apply(Change::KeepRunningInBackground(false)) writes `keep_running_in_background = false` to the file.
}
```

and the web test: the General page shows the switch with the exact label, emits a patch `{ keepRunningInBackground: false }` on toggle, and shows the `backgroundUnavailable` line when it is set.

- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement the interfaces and rules.
- [ ] **Step 4:** Run the proof commands. Expected: PASS.
- [ ] **Step 5:** Commit: `feat: keep running in the background with a tray icon`.

### Task 9: Terminal in project windows

**Files:**
- Modify: `crates/discard-app/src/desktop/terminal.rs`, `terminal/session.rs`, `web/src/bridge/terminal.ts`, the terminal panel component that `App.vue` mounts
- Test: the test modules of `terminal.rs` and `session.rs`, `web/src/__tests__/App.terminal.test.ts` or the test beside the terminal component

**Interfaces:**
- Produces:
  - `Session::attach(&self, owner: &str, sink: Sink) -> Attached`: `owner` is the window label. If a listener of another owner exists, it receives `Event::Moved` before the replacement. A new attach by the same owner sends no `Moved`.
  - `Session::owner(&self) -> Option<String>`.
  - `TerminalEvent::Moved` (serialized like the other variants).
  - `fn app_window_only(window: &tauri::WebviewWindow) -> Result<(), String>` replaces `main_window_only`. Error: `The terminal is available in app windows only.`
  - `terminal_write`, `terminal_resize`, `terminal_detach`, and `terminal_close` from a window that is not the owner return `Err("This terminal is open in another window.")` and change nothing. `terminal_status` works from any app window.
- Web: on a `moved` event the terminal panel shows the notice `This terminal is open in another window.` and a button `Show Here`. The button opens the terminal again (`terminal_open`), which moves the session to this window.

- [ ] **Step 1: Write the failing tests** in `session.rs`:

```rust
#[test]
fn a_second_window_takes_the_session_and_the_first_is_told() {
    // attach("main", sink_a); attach("project-p", sink_b);
    // sink_a received exactly one Event::Moved; session.owner() == Some("project-p").
    // Output after that reaches sink_b only.
}
#[test]
fn the_same_window_attaches_again_without_a_move() {
    // attach("main", sink_a); attach("main", sink_a2); sink_a received no Event::Moved.
}
```

and the web test: a `moved` event shows the notice and the `Show Here` button; a click calls `terminal_open` for the channel and removes the notice.

- [ ] **Step 2:** Run them. Expected: FAIL.
- [ ] **Step 3:** Implement the interfaces.
- [ ] **Step 4:** Run the proof commands. Expected: PASS.
- [ ] **Step 5:** Commit: `feat: allow the terminal in project windows`.

### Task 10: Verify in the app

**Files:** none, unless a check fails.

- [ ] **Step 1:** Run `cargo tauri dev` from `crates/discard-app`.
- [ ] **Step 2:** Check each line. Record the result of each.
  1. Project menu on a rail tile → `Open in New Window` opens a window with no rail, titled with the project name.
  2. The same action again focuses the existing window.
  3. Select another project in the main window: the project window does not change. Select another channel in the project window: the main window does not change.
  4. Start a turn in each window: both run. Close the project window: its turn continues (the rail shows it running).
  5. Both windows on one channel: a message and an approval show in both; a decision clears both.
  6. In the project window, `Agents` and `Message` on an agent profile open in the main window.
  7. Close the main window with the project window open: the app runs; Window → `Main Window` (and a Dock click on macOS) brings it back with its selection.
  8. Close all windows with a turn running: the turn continues. Quit asks for confirmation and shows the main window.
  9. Turn the General setting off: a close of the last window follows the Quit rule.
  10. Terminal: open in the main window, then in the project window on the same channel: the main window shows the notice; `Show Here` moves it back.
  11. Rename the project: the window title changes. Remove the project: its window closes.
  12. Cmd+W, Minimize, Zoom, Toggle Terminal, and Toggle Member List act on the focused window.
- [ ] **Step 3:** Fix any failure in the task that owns the code, with a test, then run the proof commands.
- [ ] **Step 4:** Commit any fix: `fix: <what failed>`.
