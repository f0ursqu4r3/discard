# Notification click Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A click on a desktop notification shows the main window and opens the channel the notification names. The 10-second follow rule goes away.

**Architecture:** Each notification carries its target channel ID as the plugin `extra` field `channelId`. One `on_action` handler, registered at startup, turns a `tap` into a channel select in the main view plus a window show. The follow code in `notify.rs`, `core.rs`, and the window-event handler is deleted.

**Tech Stack:** Rust, Tauri 2, `tauri-plugin-notification` 2.8.1.

**Spec:** `docs/superpowers/specs/2026-11-23-notification-click-design.md`

## Global Constraints

- The notification payload is the extra field `channelId` (a string). Its value is the follow ID used today: the channel itself, or the parent channel for a thread.
- The handler acts only on the action `tap` and only when the notification has a string `channelId`.
- The handler runs on a background thread: it spawns work on the app runtime and returns. It never blocks.
- If `Channels::select` fails, the handler still shows the main window and logs `tracing::warn!`.
- Deleted: `FOLLOW_WINDOW`, `FocusTracker::notified`, `FocusTracker::take_follow`, the stored last notification, `AppCore::take_notification_follow`, and the focus-event follow code in `desktop/mod.rs`, with their tests. The focused-window tracking stays.
- A click works only in a bundled macOS build and on Linux servers that report the `default` action. The docs say so.
- AGENTS.md: add no planned item and no no-op handler. Docs use short, direct sentences.
- Commit messages: no attribution lines.

## Review Focus

- The handler is registered before the first notification can show (the plugin reports nothing for a notification shown before a handler exists).
- A click for a channel that was removed or archived: the main window shows, nothing panics, one warning is logged.
- A `tap` on a notification with no `channelId`, a non-string `channelId`, or an action other than `tap`: ignored.
- A click on a thread notification opens the parent channel, not the thread ID.
- Two notifications in a row: each click opens its own channel (no shared "last notification" state remains).

---

### Task 1: Channel ID in the notification, and the pure click rule

**Files:**
- Modify: `crates/discard-app/src/desktop/notify.rs` (struct `Notification`, `compose`, `FocusTracker`, constants, tests)
- Modify: `crates/discard-app/src/desktop/core.rs` (`maybe_notify` ~line 2085, `take_notification_follow` ~line 2081, tests near lines 2979, 3059, 3080)

**Interfaces:**
- Produces:
  - `Notification { pub title: String, pub body: String, pub channel_id: String }`
  - `compose(trigger: &Trigger, channel: &str, agent: &str, muted: bool, channel_id: &str) -> Option<Notification>`: the new last argument becomes `Notification.channel_id`.
  - `pub fn channel_from_click(action_id: &str, extra: &std::collections::HashMap<String, serde_json::Value>) -> Option<String>`: `Some(id)` only when `action_id == "tap"` and `extra["channelId"]` is a non-empty string.
- Removes: `FOLLOW_WINDOW`, `FocusTracker::notified`, `FocusTracker::take_follow`, the `last` field, `AppCore::take_notification_follow`.

- [ ] **Step 1: Write failing tests** in `notify.rs`: `channel_from_click_returns_the_id_for_a_tap`, `channel_from_click_ignores_another_action`, `channel_from_click_ignores_a_missing_or_non_string_id` (missing key, number, null, empty string), and `compose_carries_the_channel_id` (an approval and a finished turn both give `channel_id` equal to the argument). Update every existing `compose(...)` call in the tests to pass a channel ID.
- [ ] **Step 2: Delete the follow tests and code.** In `notify.rs` remove the tests `a_focus_within_ten_seconds_follows_the_newest_notification_once` and `a_focus_after_ten_seconds_follows_nothing` (named as in the file; confirm by reading), `FOLLOW_WINDOW`, `notified`, `take_follow`, and `last`. Keep the focus-set tests.
- [ ] **Step 3: Update `core.rs` tests.** Delete `the_focus_that_follows_a_notification_returns_its_channel_id` and every other use of `take_notification_follow` (lines ~2979, ~3059, ~3080-3083). Where a test checked the follow ID for a thread, assert instead that the recorded notification's `channel_id` is the parent channel's ID. Add `a_sent_notification_carries_the_channel_id`: an approval in channel `a` gives `channel_id == "a"`, and a finished turn of a question thread gives the parent's ID.
- [ ] **Step 4: Run** `cargo test -p discard-app notify core::`. Expected: compile errors, then FAIL.
- [ ] **Step 5: Implement** the struct field, the `compose` argument, `channel_from_click`, and the deletions. In `maybe_notify`, remove the `focus.notified(...)` line and pass `channel_id` to `compose`. Delete `take_notification_follow`.
- [ ] **Step 6: Run** `cargo test -p discard-app notify core::` then `cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS, no warnings. (`mod.rs` still calls the deleted follow method: Task 2 removes that call, so make `mod.rs` compile first by deleting only the focus-follow `if` block there if the build fails, and leave everything else in `mod.rs` to Task 2.)
- [ ] **Step 7: Commit** `git add crates/discard-app && git commit -m "feat: carry the channel in a notification and drop the follow rule"`.

### Task 2: The click handler and docs

**Files:**
- Modify: `crates/discard-app/src/desktop/mod.rs` (`TauriNotifier` ~line 438, `setup` ~line 224, `on_window_event` ~lines 183-205)
- Modify: `docs/settings.md` (the "Desktop notifications" section, line ~206)

**Interfaces:**
- Consumes: `Notification.channel_id`, `channel_from_click` (Task 1); `Channels::select(&self, view: &ViewId, id: &str) -> Result<ViewId, String>`; `Channels::in_project(&self, id: &str) -> bool`; `windows::show_conversation(app: &AppHandle, view: &ViewId)`; `windows::show_main_window(app: &AppHandle)`; `ViewId::main()`.

- [ ] **Step 1: Remove the follow code from `on_window_event`.** Keep the `window_focus` and `window_closed` calls. Delete the `take_notification_follow` branch and the `event_channels` clone if nothing else uses it (the handler in Step 3 needs a `Channels` clone, so keep one clone for `setup`).
- [ ] **Step 2: Attach the payload.** In `TauriNotifier::notify` call `.extra("channelId", notification.channel_id.clone())` on the builder before `.show()`.
- [ ] **Step 3: Register the handler in `setup`,** before `core.set_notifier(...)`, with `app.notification().on_action(move |performed| { ... })` (`tauri_plugin_notification::NotificationExt`). Inside: read `performed.action_id()` and `performed.notification().map(|n| n.extra())`, call `notify::channel_from_click`, and return when it gives `None`. When it gives `Some(channel_id)`, spawn on the runtime (`event_runtime`) the same steps `switch_channel` takes (`commands.rs` ~line 479): `tokio::task::spawn_blocking` calling `channels.select(&ViewId::main(), &channel_id)` and `channels.in_project(&channel_id)`; on `Ok`, emit `discard://conversations` when the channel is not in a project, then `windows::show_conversation(&app, &changed)`; on `Err` or a join error, `tracing::warn!` and call `windows::show_main_window(&app)`. A handler registration error is logged with `tracing::warn!` and does not stop startup.
- [ ] **Step 4: Update the docs.** In `docs/settings.md` replace the click bullets: a click opens the channel of that notification in the main window and shows the window when it is hidden; a click works in a bundled macOS build and on Linux servers that report the `default` action, and elsewhere it only raises the app; a channel that no longer exists still shows the main window. Delete the "within 10 seconds" statements and the line that says a click with every window hidden does not open a window.
- [ ] **Step 5: Run** `cargo test -p discard-app && cargo test -p discard-core && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS, no warnings. Strip the Claude environment variables from the test run, as before.
- [ ] **Step 6: Live check (not for subagents).** The controller or the user runs a bundled macOS build, hides every window, triggers an approval, clicks the notification, and confirms the channel opens in the main window. Record that this was not done if no one ran it.
- [ ] **Step 7: Commit** `git add crates/discard-app docs && git commit -m "feat: open the channel when a notification is clicked"`.

## Self-review notes

- Spec coverage: decisions 1 and 5 in Task 1; decisions 2, 3, 4, and 7 in Task 2; decision 6 (Linux) in the Task 2 docs.
- Decision made here: the plan reuses the `switch_channel` steps for the select, so a project channel and a direct message both work. The click always uses the main view.
