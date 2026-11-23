# Desktop notifications Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Send an operating-system notification for a pending approval and for a finished turn when no Discard window is focused.

**Architecture:** A pure module (`notify.rs`) decides whether and what to notify, tracks window focus, and de-duplicates approvals. `AppCore::apply_runtime` derives a trigger from each update and calls the module. A small `Notifier` trait hides `tauri-plugin-notification`, so tests use a recording fake. One new setting, `general.desktop_notifications`, turns the feature off.

**Tech Stack:** Rust, Tauri 2, `tauri-plugin-notification` 2, Vue 3 + TypeScript, Vite+ tests.

**Spec:** `docs/superpowers/specs/2026-11-23-desktop-notifications-design.md`

## Global Constraints

- A notification fires only when no Discard window is focused. The Settings window and project windows count as Discard windows.
- Title: `<channel name> · <agent name>`. Body: `Needs approval: <tool name>`, `Finished`, or `Failed`. The body never holds message or tool-argument text.
- An approval request notifies at once. A finished turn notifies once per turn. A turn of a sub-agent thread never notifies.
- A muted channel sends no finished-turn notification. Approvals in a muted channel still notify.
- Setting: `general.desktop_notifications`, default `true`. An invalid value falls back to `true` and reports an issue.
- A focus within 10 seconds after a notification selects the channel of the newest notification. A later focus does not.
- AGENTS.md: the project menu item **Desktop Notifications** stays disabled, labelled Soon, with its existing `TODO`. Add no no-op handler.
- Web tests go in a `__tests__` directory beside the code they test.
- Commit messages: no attribution lines.

## Review Focus

- A window closes while it holds focus (no focus-lost event): the focus set must drop it, so a hidden app still notifies.
- The same approval ID arrives twice: one notification.
- A user cancels a turn: no notification.
- A channel or tool name with a newline or 500 characters: one line, truncated.
- Only the Settings window is focused: no notification.

---

### Task 1: The `general.desktop_notifications` setting (Rust)

**Files:**
- Modify: `crates/discard-core/src/settings.rs` (struct `GeneralSettings` near line 109, parser near line 446, `Change` near line 144, writer near line 564, tests near line 666)
- Modify: `crates/discard-app/src/desktop/settings/mod.rs:47` (`GeneralPatch`)
- Modify: `crates/discard-app/src/desktop/settings/general.rs` (`general_page`, `general_changes`, parity tests)
- Modify: `docs/settings.md` (TOML sample near line 16, key table near line 36)

**Interfaces:**
- Produces: `GeneralSettings.desktop_notifications: bool`; `Change::DesktopNotifications(bool)`; `GeneralPatch.desktop_notifications: Option<bool>`; the page JSON key `"desktopNotifications"`.

- [ ] **Step 1: Write failing tests in `settings.rs`**, copying the pattern of the three `keep_running_in_background_*` tests: `desktop_notifications_defaults_to_on`, `desktop_notifications_saves_each_value` (file text contains `desktop_notifications = {on}` and a reopened store reads it), `desktop_notifications_with_a_wrong_type_falls_back_to_on` (file `desktop_notifications = "no"` gives `true` and one issue with key `general.desktop_notifications`).
- [ ] **Step 2: Add a failing test in `general.rs`** `general_changes_maps_desktop_notifications`: a patch with `desktop_notifications: Some(false)` gives `vec![Change::DesktopNotifications(false)]`; and `general_page` output has `"desktopNotifications": true` for default settings.
- [ ] **Step 3: Run** `cargo test -p discard-core settings && cargo test -p discard-app settings::general`. Expected: compile errors, then FAIL.
- [ ] **Step 4: Implement** the field (default `true`), the parser branch with message `must be true or false`, the `Change` variant and its writer arm, the patch field, the `general_changes` push, and the page key. Follow `keep_running_in_background` line for line.
- [ ] **Step 5: Document** the key in `docs/settings.md`: TOML sample line `desktop_notifications = true  # notify when an approval waits or a turn ends and no window is focused`, and a table row (`true` or `false`, default `true`).
- [ ] **Step 6: Run** the commands of Step 3. Expected: PASS.
- [ ] **Step 7: Commit** `git commit -am "feat: add the desktop notifications setting"`.

### Task 2: The notification rules (`notify.rs`)

**Files:**
- Create: `crates/discard-app/src/desktop/notify.rs` (register with `pub mod notify;` in `desktop/mod.rs`)

**Interfaces:**
- Produces:
  - `pub struct Notification { pub title: String, pub body: String }`
  - `pub enum Trigger { Approval { id: String, tool: String }, Finished(TurnOutcome) }` (`TurnOutcome` is `discard_core::driver::TurnOutcome`)
  - `pub fn trigger_of(update: &Update, is_thread: bool) -> Option<Trigger>`: `Update::Event` with `EventKind::ApprovalRequest` gives `Approval` (also in a thread); `Update::TurnFinished(outcome)` gives `Finished` unless `is_thread`; every other update gives `None`.
  - `pub fn compose(trigger: &Trigger, channel: &str, agent: &str, muted: bool) -> Option<Notification>`: `Finished(Cancelled)` gives `None`; `Finished` of a muted channel gives `None`; an approval always composes. Title and tool name become one line (whitespace runs collapse to one space) and are cut to 60 characters with a trailing `…`.
  - `pub const FOLLOW_WINDOW: Duration` (10 seconds)
  - `pub struct FocusTracker` with `new()`, `set_focus(&mut self, label: &str, focused: bool)`, `window_closed(&mut self, label: &str)`, `app_focused(&self) -> bool`, `notified(&mut self, channel_id: &str, now: Instant)`, `take_follow(&mut self, now: Instant) -> Option<String>`. `take_follow` returns the channel of the newest notification if it is at most `FOLLOW_WINDOW` old, and clears it in every case.
  - `pub struct Seen` with `new()` and `first(&mut self, key: &str) -> bool` (true the first time; remembers the last 128 keys).
  - `pub trait Notifier: Send + Sync { fn notify(&self, notification: &Notification); }`

- [ ] **Step 1: Write failing tests** in a `#[cfg(test)] mod tests` of `notify.rs`, named for the rules:
  - `an_approval_composes_title_and_body` (`"general · Ada"`, `"Needs approval: Bash"`)
  - `a_finished_turn_says_finished_and_a_failed_turn_says_failed`
  - `a_cancelled_turn_composes_nothing`
  - `a_muted_channel_drops_finished_but_keeps_approvals`
  - `a_thread_turn_gives_no_trigger_but_a_thread_approval_does`
  - `other_updates_give_no_trigger`
  - `names_with_newlines_or_500_characters_become_one_short_line`
  - `the_body_holds_only_the_tool_name_never_the_input` (an approval whose `input` JSON contains `secret` composes without `secret`)
  - `focus_tracker_reports_focus_while_any_window_is_focused` (two windows; one loses focus: still focused; both lose: not focused)
  - `a_closed_window_leaves_the_focus_set` (focus `project-a`, then `window_closed("project-a")` with no focus-lost event: `app_focused()` is false)
  - `a_focus_within_ten_seconds_follows_the_newest_notification_once` (two `notified` calls; `take_follow` at +9 s returns the second channel; a second call returns `None`)
  - `a_focus_after_ten_seconds_follows_nothing`
  - `seen_reports_a_key_once_and_forgets_after_128_newer_keys`
- [ ] **Step 2: Run** `cargo test -p discard-app notify`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement** the items above. `trigger_of` reads the tool from `EventKind::ApprovalRequest { approval_id, tool, .. }` and uses `approval_id` as `id`.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** `git add crates/discard-app/src/desktop && git commit -m "feat: add the desktop notification rules"`.

### Task 3: Wire the notifications into the app

**Files:**
- Modify: `crates/discard-app/Cargo.toml` (add `tauri-plugin-notification = "2"`)
- Modify: `crates/discard-app/capabilities/default.json` (add `"notification:default"` to `permissions`)
- Modify: `crates/discard-app/src/desktop/core.rs` (`AppCore` state, `apply_runtime` near line 1932, new setter)
- Modify: `crates/discard-app/src/desktop/mod.rs` (plugin init near line 151, `on_window_event` near line 175, `setup`)
- Test: `crates/discard-app/src/desktop/core.rs` (existing `#[cfg(test)]` modules; reuse the `apply_channel` approval fixture near line 2485)

**Interfaces:**
- Consumes: everything produced by Task 2; `GeneralSettings.desktop_notifications` from Task 1; `Channels::select(&self, view: &ViewId, id: &str) -> Result<ViewId, String>` at `channels.rs:154`.
- Produces: `AppCore::set_notifier(&self, notifier: Arc<dyn Notifier>, enabled: Arc<dyn Fn() -> bool + Send + Sync>)`; `AppCore::window_focus(&self, label: &str, focused: bool)`; `AppCore::window_closed(&self, label: &str)`; `AppCore::take_notification_follow(&self) -> Option<String>`.

- [ ] **Step 1: Write failing tests** in `core.rs` with a `RecordingNotifier` (a `Mutex<Vec<Notification>>` implementing `Notifier`). Names:
  - `an_approval_notifies_when_no_window_is_focused` (a pending approval in channel `a`; one notification with the channel and agent name in the title)
  - `nothing_notifies_while_a_window_is_focused` (call `window_focus("main", true)` first)
  - `nothing_notifies_while_only_the_settings_window_is_focused`
  - `nothing_notifies_when_the_setting_is_off`
  - `a_finished_turn_notifies_once` (`Update::TurnFinished(Completed)`)
  - `a_duplicate_approval_id_notifies_once`
  - `a_cancelled_turn_does_not_notify`
  - `a_sub_agent_thread_turn_does_not_notify`
  - `a_muted_channel_notifies_for_an_approval_but_not_for_a_finished_turn`
  - `the_focus_that_follows_a_notification_returns_its_channel_id` (`take_notification_follow` right after a notification returns the channel's `channel_id`; a second call returns `None`)
- [ ] **Step 2: Run** `cargo test -p discard-app core::`. Expected: FAIL (methods missing).
- [ ] **Step 3: Implement the state in `AppCore`:** one `Mutex` field holding `FocusTracker`, `Seen`, and an `Option<(Arc<dyn Notifier>, Arc<dyn Fn() -> bool + Send + Sync>)>`. Implement the four methods on it. In `apply_runtime`, compute `trigger_of(&update, thread_parent.is_some())` before `update` moves, and read the channel's `channel` name, `agent`, `muted`, and `channel_id` under the existing state lock. After `drop(s)`, call a private `maybe_notify` that returns early when no notifier is set, when `enabled()` is false, when `app_focused()` is true, or when an approval ID is not `Seen::first`; otherwise it calls `compose`, then `FocusTracker::notified(channel_id, Instant::now())` and `Notifier::notify`. Never hold the state lock while notifying.
- [ ] **Step 4: Implement `TauriNotifier`** in `notify.rs` or `mod.rs`: holds a `tauri::AppHandle`; `notify` uses `tauri_plugin_notification::NotificationExt` (`app.notification().builder().title(..).body(..).show()`). A failed `show` (for example a denied permission) logs `tracing::warn!` and returns. It never panics or returns an error. Add `.plugin(tauri_plugin_notification::init())` and the capability entry.
- [ ] **Step 5: Wire the windows.** In `on_window_event`: call `event_core.window_focus(window.label(), focused)` for `WindowEvent::Focused(focused)` and `event_core.window_closed(window.label())` for `WindowEvent::Destroyed`, for every window label. On `Focused(true)`, when `ViewId::from_label(window.label())` is `Some(view)` and `event_core.take_notification_follow()` is `Some(channel_id)`, call `Channels::select(&view, &channel_id)` and ignore an `Err` with a `tracing::warn!`. Clone the `Arc<Channels>` before `.manage(channels)` to reach it. In `setup`, build the notifier from `app.handle().clone()` and an `enabled` closure that reads `desktop_notifications` from the settings store (reload outside edits first, as `DesktopActions::keeps_running` does).
- [ ] **Step 6: Run** `cargo test -p discard-app && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS, no warnings.
- [ ] **Step 7: Check live** with `cargo tauri dev` (temp `DISCARD_HOME`, per project practice). Hide the window, trigger an approval, and confirm one notification. Focus the app within 10 s and confirm the channel opens. On macOS, accept the permission prompt. State in the report what was not checked (Linux).
- [ ] **Step 8: Commit** `git add -A crates/discard-app && git commit -m "feat: send desktop notifications for approvals and finished turns"`.

### Task 4: The Settings switch and docs

**Files:**
- Modify: `web/src/settings/pages/General.vue` (new card after the Background card, near line 88)
- Test: `web/src/settings/pages/__tests__/General.test.ts`
- Modify: `docs/settings.md` (a short "Desktop notifications" section)

**Interfaces:**
- Consumes: page key `desktopNotifications: boolean` and patch key `desktopNotifications` (Task 1).

- [ ] **Step 1: Write failing tests** in `General.test.ts`, extending the `page.state` fixture with `desktopNotifications: true`: `shows the Desktop notifications switch on by default` (role `switch`, aria-label `Desktop notifications`, `aria-checked` `true`) and `sends a patch when the user turns it off` (invoke `settings_set_general` with `{ patch: { desktopNotifications: false } }`). Reset the field in `beforeEach`.
- [ ] **Step 2: Run** `pnpm --dir web test General`. Expected: FAIL.
- [ ] **Step 3: Implement** a `SettingsCard title="Notifications"` with one `SettingsRow` and `Switch`, copying the Background card. Label `Desktop notifications`. Help text: `Notify when an approval waits or a turn ends while no Discard window is focused`. Add `desktopNotifications: boolean` to the page type at line 14.
- [ ] **Step 4: Document** in `docs/settings.md`: what fires, the one-line title and body, that a click only raises the app plus the 10-second rule, that a denied system permission means no notifications, and that per-project modes remain Soon.
- [ ] **Step 5: Run** `pnpm --dir web check && pnpm --dir web test`. Expected: PASS.
- [ ] **Step 6: Commit** `git add web docs && git commit -m "feat: add the desktop notifications switch"`.

## Self-review notes

- Spec coverage: decisions 1 to 6 sit in Tasks 2 and 3 (focus rule, events, content, mute, follow); decision 4's switch is Tasks 1 and 4; decision 7 changes no file, because the project menu stays as it is.
- Calls the spec left open, made here: a cancelled turn does not notify; an approval inside a sub-agent thread does notify; names are cut to 60 characters; approval IDs are remembered for the last 128.
