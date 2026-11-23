# Notification click design

Date: 2026-11-23. Size: small change. Status: awaiting approval. Amends `2026-11-23-desktop-notifications-design.md` decision 6.

## Intent

A click on a desktop notification should open the channel the notification names, even when every Discard window is hidden. Today a click only raises the app. With all windows hidden it shows no window. The 10-second follow rule guesses at a click and never helps in that case.

## Goals

- A click on a notification shows the main window and opens the notification's channel in it.
- The 10-second follow rule goes away.

## Non-goals

- Notification action buttons (Approve, Deny). Approval decisions stay in the channel.
- Windows behavior.
- Opening the channel in a project window.
- Notifications that a notification server shows without reporting clicks. A click there only raises the app.

## Anti-goals

- A channel switch that the user did not ask for (the old rule moved the selection on any return within 10 seconds).
- A click handler that blocks, panics, or fails silently for a channel that no longer exists.

## Approaches

- **A. Rust `on_action` handler (chosen).** `tauri-plugin-notification` 2.8.1 reports a click as the action `tap` to handlers registered with `Notification::on_action`. This works with no webview.
- **B. A JavaScript `onAction` listener (rejected).** It needs a live webview. No webview runs while every window is hidden.

## Decisions

1. **The payload.** Each notification carries its target channel ID as the extra field `channelId`. The ID is the one the follow uses today: the channel itself, or the parent channel for a thread.
2. **The handler.** `setup` registers one `on_action` handler before any notification can show (the plugin reports nothing for a notification shown before a handler exists). It ignores every action except `tap` and every click whose notification has no string `channelId`.
3. **The click.** The handler runs on a background thread, so it only spawns work on the app runtime and returns. The work selects the channel in the main view with `Channels::select`, off the event thread, and then shows the changed view with `windows::show_conversation`, like `switch_channel` does.
4. **A channel that is gone.** If `select` fails (the channel was removed or archived), the handler shows the main window anyway and logs a `tracing::warn!`.
5. **The removal.** `FOLLOW_WINDOW`, `FocusTracker::notified`, `FocusTracker::take_follow`, the stored last notification, `AppCore::take_notification_follow`, and the focus-event follow code in `desktop/mod.rs` are deleted, with their tests. The "focused windows" part of `FocusTracker` stays.
6. **Linux.** With a handler registered, the plugin adds the `default` action to each notification. A server that does not report it behaves as before: a click only raises the app. The docs say so.
7. **Docs.** `docs/settings.md`: remove the 10-second statements, state the click behavior, and state the Linux limit and the rule that a click works only in a bundled app on macOS.

## Constraints

- The click-to-channel decision is a pure function in `notify.rs` with unit tests. The Tauri calls stay in `desktop/mod.rs`.
- The check needs a bundled macOS build. In `cargo tauri dev` the notification posts as Terminal, so a click cannot be verified there.
- AGENTS.md: no planned item and no no-op handler are added.

## Left to the builder

- Where the channel ID enters the `Notification` struct (a new field set by the caller of `compose`, or a `compose` parameter).
- How to build the `ViewId` for the main view and how the handler reaches `Channels` (clone the `Arc` before `.manage(channels)`, as the removed focus code did).

## Tests

- `channel_from_click`: `tap` with a string `channelId` gives the ID; another action ID gives `None`; a missing or non-string `channelId` gives `None`.
- `AppCore` notification tests: a sent notification carries the channel ID (the parent's ID for a thread). The follow tests are deleted.
- `FocusTracker`: the focus-set tests stay and pass with the follow code removed.
