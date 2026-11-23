# Desktop notifications design

Date: 2026-11-23. Size: small change. Status: awaiting approval.

## Intent

Discard can keep running in the background with a tray icon. Today nothing tells the user when an agent needs an approval or finishes a turn while no window is focused. The user must look at the app to find out. This change sends an operating-system notification for those two events.

## Goals

- Notify for a pending approval and for a finished turn when no Discard window is focused.
- Show enough to know which channel and what kind of event, and nothing more.
- Add one global switch, on by default.

## Non-goals

- Per-project notification modes, mute timers, and per-project delivery settings. These stay planned items.
- Notifications for agent mentions, summaries, scheduled tasks, or sub-agent threads.
- Click actions that name a target. The desktop plugin cannot report a click.

## Anti-goals

- A notification that leaks message text into the notification center or onto the lock screen.
- A notification while the user already looks at Discard.
- A second notification for one event, or a burst for one long turn.

## Approaches

- **A. Rust fires them with `tauri-plugin-notification` (chosen).** It works while the webview is hidden. Limit: the desktop plugin cannot report a click.
- **B. The webview `Notification` API (rejected).** A hidden webview is unreliable, most of all on Linux.

## Decisions

1. **When.** A notification fires only when no Discard window is focused.
2. **Events.** An approval request notifies at once. A finished turn notifies once per turn, and a failed turn counts as finished. A turn of a sub-agent thread does not notify. Its parent channel reports the result.
3. **Content.** The title is the channel name and the agent name. The body is one of: `Needs approval: <tool name>`, `Finished`, `Failed`. The body never holds message or tool-argument text.
4. **Switch.** Settings, General has one switch, **Desktop notifications**. It is on by default and saved in `settings.toml` with the other general settings. An invalid value falls back to on, with the usual settings error. The macOS permission prompt appears at the first notification. If the user denies it, nothing fires and nothing breaks.
5. **Muted channels.** A muted channel sends no finished-turn notification. Approvals still notify, as badges do.
6. **Click.** A click only brings the app to the front. If a window gains focus within 10 seconds after a notification, it selects the channel of the newest notification in that time. A focus change after that does not move the selection.
7. **Planned item.** The project menu item **Desktop Notifications** stays visible, disabled, and labelled Soon, with its existing `TODO`. This change adds no planned item and no no-op handler.

## Constraints

- Core logic that decides whether to notify has no GUI dependency. It lives in `discard-core` or as a pure function in `discard-app`, with unit tests. The plugin call stays in the Tauri layer behind a small trait, so tests use a recording fake.
- Add `tauri-plugin-notification` to `discard-app` and its capability file. Linux needs no extra package beyond the existing set; check this on a Linux build.
- Web tests go in a `__tests__` directory beside the code they test (AGENTS.md).

## Left to the builder

- The exact hook. Updates reach the app through `apply_runtime` in `AppCore`; approval requests and `TurnFinished` pass there. The builder confirms that every open channel, not only the one selected at startup, reaches the hook.
- How to read window focus across all windows, including project windows.
- De-duplication of an approval that is replayed after a restart. A stale or replayed approval must not notify.
- Docs: update `docs/settings.md` with the new switch and its limits.

## Tests

- Decision function: focused window blocks; each event kind produces the right title and body; sub-agent turns and muted-channel finished turns produce nothing; an approval in a muted channel still notifies; the switch off blocks all; the body never contains message text.
- Focus rule: a focus within 10 seconds selects the newest notified channel; later focus does not.
- Settings: default on; an invalid value falls back to on and reports an issue.
- UI: the switch shows in General and writes the setting.
