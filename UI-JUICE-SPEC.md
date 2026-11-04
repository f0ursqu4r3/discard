# UI Juice — Spec

Goal: make the main window look and feel closer to the Discord desktop client. This is polish and layout work in `discard-ui`. It adds no new backend.

## Ground rules

- Main window only (`crates/discard-ui/src/view.rs`). Do not change the Settings window.
- UI only. Do not change the engine, SQLite, or the `Message` type. `Message` has no timestamp, so show none.
- One project, one channel, one agent exist today. Do not fake more. Extra servers or channels, if drawn at all, show greyed with a "Soon" tag and no handler. Use the existing "Soon" list in `crates/discard-core/src/action.rs`.
- Every color, radius, and spacing value comes from a theme token (`cx.theme()`, `self.space(..)`). No hard-coded colors. Plugin themes and the Appearance page must still work.
- Use no Discord logo, icon, font, or other asset. Use the layout idea only.
- Protected surfaces keep their contrast and cannot be hidden: approval card (`render_approval`), review banner, plugin problem line, confirm notice, archive notice. Do not restyle them. Only confirm they still read well.
- Interactive surfaces stay in Rust. Do not add Lua slots or Lua elements unless a surface below needs one. If you add one, update `crates/discard-lua/src/api_spec.rs` docs and tests.
- Animations: each lasts 200 ms or less. They stop when the user wants reduced motion. If GPUI exposes no reduce-motion query without a new dependency, add one bool `appearance.reduce_motion` (default `false`) to `settings.toml` with validation and a failure test, and report this in your output.
- Follow the style of the surrounding code: small functions, doc comments in plain English, tests next to the code.
- Do not commit. Do not touch git config or branches.

## Read state (new, UI-level)

- Store the last-read message ID per channel in the window `StateStore` (`crates/discard-core/src/window_state.rs`), saved in `state.toml`. `WindowState` is `Copy` today. Change it as needed (for example, a map field) and fix all users.
- A bad or missing value falls back to "everything read". A bad file never blocks startup. Add unit tests in the same style as the existing ones.
- A channel is read when its window is focused and the message pane is at the bottom (no "Jump to latest" button). Otherwise new agent messages count as unread.
- Put the pure decision logic (`is_unread`, first unread index) in `discard-core` with unit tests. It has no GPUI dependency.

## Surfaces

Implement only the surface named in the task prompt. Each surface is one reviewable change.

### Surface 1 — Message rows

File: `view.rs` (`message_row`, `live_row`, `render_block`).
- Group consecutive messages from the same role: the first shows avatar and name, later ones show only the blocks, aligned with the text column. Put the grouping logic in a pure function with tests.
- Hover state on a row (subtle background token).
- A hover toolbar at the row's top-right. It reuses actions the context menu already has (for example Copy message). It adds no new actions. Route clicks through `dispatch::run` or the existing context-menu handlers.
- Clearer typed blocks: code and diff blocks get a header line with the language and a copy button (existing copy action); tool-call blocks get a left accent bar and a compact header. Keep `with_menu` context menus working.
- A "New" divider above the first unread message (uses Read state). It clears when the channel becomes read.
- A streaming cursor in the live row while a turn runs.
- A short fade-in for new rows.

### Surface 2 — Composer

File: `view.rs` (the composer block in `render`).
- Rounded input container with a border token, a focus ring, and padding from the spacing scale.
- Send is disabled and muted when the text is empty. Stop shows while a turn runs. Both keep their current behavior and guards (`may_send`, archived, archive busy).
- Placeholder stays `Message #<channel>`.

### Surface 3 — Server rail

File: `view.rs` (`server_rail`).
- A left-edge pill beside the server icon: tall for selected, small for unread, hidden otherwise.
- On hover the icon changes from round to rounded-square.
- A count badge on the icon for pending approvals (warning token). It outranks the unread pill.
- Keep the existing server context menu.

### Surface 4 — Channel list

File: `view.rs` (`channel_list`).
- Row states: hover, selected, unread (bold name plus a small dot on the left edge).
- A count badge for pending approvals (warning token). It outranks the unread dot.
- Keep the Plugins and Settings buttons and the channel context menu.

### Surface 5 — Member list

File: `view.rs` (`member_list`) and `crates/discard-lua/bundled/default-ui/init.lua` (`member_list` slot).
- Presence dot on the avatar corner instead of next to the status text.
- Keep the Lua slot and the Rust fallback in step, using only existing Lua elements. If the Lua element set cannot draw a corner dot, keep the dot beside the text in Lua and note it in your report.

## Proof

```
cargo build --workspace && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```

Every new pure function has unit tests, including edge cases: empty history, one message, all read, all unread, a missing or corrupt `state.toml`.

## Non-goals

Timestamps. Unread for more than one channel in the engine. DMs, threads, voice. New backend. Settings window changes. Lua slots for interactive parts. New dependencies (unless the reduce-motion fallback above needs none).
