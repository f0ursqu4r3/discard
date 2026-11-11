# Terminal visual direction — design

**Date:** 2026-11-07
**Status:** Draft for approval
**Reference mockup:** [`docs/ui-design-reference/terminal-mix.html`](../../ui-design-reference/terminal-mix.html). Open it in a browser. The switches above the window turn each piece on and off. **Today** turns all of them off and approximates the current app. The mockup is static HTML. When this document and the mockup differ, this document wins. The known difference: the mockup draws agent avatars as rounded squares; agents get circles (see Shapes).

## Intent

Discard looks generic. It uses flat neutral surfaces, one accent, the system font, and stock shadcn-vue controls. Nothing in the look belongs only to Discard.

Discard is a harness for CLI agents. Its users live in a terminal (the owner uses Ghostty with SF Mono). The new look leans into that: terminal type, terminal glyphs, and terminal colors, added to today's layout. The app must still read as a desktop app, not as a terminal emulator.

## Goals

Keep today's layout, navigation, tabs, panels, and controls. Add these eight pieces:

1. Mono structure
2. Agent colors
3. Spinners and a block cursor
4. Terminal tool calls and diffs
5. Branch tree
6. Dotted key-values
7. Sharper corners and rules
8. Status line

Also apply the avatar shapes in [Shapes](#shapes).

## Non-goals

- Do not move panels, change navigation, or add or remove views.
- Do not add backend data only to fill the status line or tool-call results. The one exception is the tool-call `is_error` flag (see section 4), which the backend already parses.
- Do not add new themes or change what the existing 14 theme tokens mean.
- Do not redesign the Settings window layout. It picks up shared token and component changes only.
- Do not add a composer key hint line. An earlier version removed it on purpose (`composer.test.ts` checks that it is absent).

## Anti-goals

The work fails, even if it runs, when any of these is true:

- A color is hard-coded and breaks a Ghostty palette or a plugin theme.
- The status line or a tool call shows a value with no real data behind it.
- A protected surface loses contrast or changes its protected styling.
- Message prose renders in the mono font.
- Motion continues when the system asks for reduced motion.
- A light theme (for example Chalk) becomes hard to read.

## Constraints

- **shadcn-vue stays.** Restyle through the CSS variables in `web/src/styles/tokens.css`. The components are copies in `web/src/components/ui`. Edit their classes only where a variable cannot reach.
- **Theme tokens.** Every color comes from a theme token or a `color-mix()` of tokens. Ghostty palettes and plugin themes must keep working in dark and light modes. The owner uses `light:Chalk,dark:Atom One Dark`.
- **Protected surfaces.** The approval card, review banner, plugin problem line, confirm notice, and archive notice keep the protected tokens (`--protected-*`) and their current styling. `protected.test.ts` must still pass.
- **Reduced motion.** Every animation stops when `prefers-reduced-motion: reduce` matches. `tokens.css` already has a global rule for CSS animations. JavaScript-driven animation (the spinner) must check the media query itself.
- **Planned items.** The `AGENTS.md` rule for unimplemented actions applies to any new control. This design adds no new actions.
- **Tests.** `pnpm --dir web check`, `pnpm --dir web test`, and `pnpm --dir web build` pass. Rust changes pass `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.

## Decisions

### Shapes

- **Projects get square avatars.** This covers the server rail icons, the project header in the channel list, and `ProjectAvatar`. Today they use `rounded-[25%]` and `rounded-xl` (squircles). Use the radius token instead, which makes them squares with small rounded corners. The rail selection pill and hover rules stay.
- **Agents get circle avatars.** `AgentAvatar` is already round. Keep it round everywhere: messages, member list, agents navigation, profile dialogs, and the friends list.
- **People.** The user's own avatar stays a circle in today's style. *(Claude's call.)*

### 1. Mono structure

- Add a `--font-mono` token. Its value is the first family in the Ghostty `font-family` setting, followed by `'SF Mono', ui-monospace, Menlo, monospace`. Without a Ghostty setting, use only the fallback stack. *(Claude's call.)* `LiveTheme` in `crates/discard-core/src/ghostty/live.rs` already finds and watches the Ghostty config files. Read `font-family` from those same files and send it to the web app with the theme.
- **Mono text:** channel, branch, and worktree names; paths; timestamps; counts and badges; key-value rows; tool calls; code and diff blocks; the status line; and the composer placeholder's channel name.
- **Sans text (system font):** message prose, navigation labels (Overview, Agents, Memory, and so on), card titles, buttons, tab labels, dialog text, and settings text.
- Set mono text at about 0.92 of the surrounding font size so that it matches the visual weight of the sans text.
- The message header shows the agent's name and the time. It shows no `[agent · <provider>]` tag. *(An earlier version of this spec added that tag. The tag was removed.)*
- Put a `›` prompt glyph in the primary color before the composer placeholder.

### 2. Agent colors

- **Source.** The color in the agent's profile drives the agent's identity color. Today the avatar field stores `lucide:<icon>:#rrggbb` (`web/src/components/agents/agent-icon.ts`).
- **Palette slots.** Add a second stored form, `lucide:<icon>:ansi-<n>`, where `n` is 1 to 6: red, green, yellow, blue, magenta, and cyan. A slot follows the theme, so the color changes when the theme changes. The `#rrggbb` form stays valid for custom colors.
- **Theme tokens.** Expose the six palette colors as tokens `ansi_1` to `ansi_6` (CSS `--ansi-1` to `--ansi-6`). Ghostty themes take them from palette entries 1 to 6. Ghostty already uses `DEFAULT_PALETTE` (`crates/discard-core/src/ghostty/map.rs`) for an entry a theme does not set. Plugin themes may set these tokens. When a plugin theme does not set one, keep the active Ghostty theme's value, the same way `layer` in `crates/discard-app/src/desktop/theme.rs` treats the other tokens.
- **Picker.** In `AgentIconPicker`, replace the eight fixed hex presets with six swatches that show the current theme's palette colors. Label them by name (Red, Green, and so on). Keep the custom color input.
- **Projects use the same picker.** `ProjectIdentitySettings` shares `AgentIconPicker`, and `ProjectAvatar` shares `parseAgentIcon`, so projects also get the six swatches and the `ansi-<n>` form. Change the validation in `Store::update_project_profile`, which accepts only `#rrggbb` today, to accept the new form too. *(Claude's call.)*
- **Stored hex colors** do not change. There is no migration. *(Claude's call.)*
- **Agents without a color.** Most agents today have no stored avatar (`avatar = null`). Bootstrap and the Add Agent dialog do not set one. Some agents store an image URL. For these agents, pick a palette slot from a stable hash of the agent ID. Compute it at display time, and do not save it. The color stays the same when other agents are added. Two agents can get the same color, and the user can change either one in the profile. An agent with an image URL keeps its image, and the hashed color drives its name, spinner, and tool-call edge. *(Claude's call.)*
- **Where the color shows:**
  - the agent's name in message headers and the member list;
  - the agent's avatar: a circle with a tint of the color over the background (about 15%), the icon in the color, and a ring in the color (about 40%);
  - the agent's spinner;
  - the left edge of the agent's tool calls.
- **Contrast.** When the color is too close to the surface, move it toward the foreground until names reach 4.5:1 and icons, edges, and spinners reach 3:1. This keeps custom colors and light themes readable.

### 3. Spinners and the block cursor

- **Spinner.** Show a braille spinner (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`, about 80–100 ms per frame) in the agent's color wherever an agent is working:
  - the member list;
  - the agents navigation;
  - the channel or worktree row whose agent is working;
  - a running tool call;
  - the status line;
  - a short activity note at the right end of the channel tabs row, for example `⠹ builder is working`. Use only status text that the snapshot already has (`agentStatus`, `turnRunning`).
- Drive all spinners from one shared timer, not one timer per spinner.
- Where today's UI shows a static presence dot, keep the dot for idle and ready states. The spinner replaces it only while the agent works.
- **Block cursor.** While a turn streams, end the live text with a block cursor in the caret color. It blinks at about 1 s.
- **Reduced motion.** The spinner shows one static frame, and the cursor stays solid.

### 4. Terminal tool calls and diffs

- **Tool calls (`ToolCallBlock`).** Today a tool call is a compact row that is collapsed by default. Commit `0aeb6bf` made it compact on purpose. Keep that behavior.
  - The collapsed row becomes one mono line: `$ <name> <main argument>`.
  - Add a 2 px left edge in the agent's color and a faint inset background (a mix of the canvas and background tokens).
  - The expanded content (input and output) stays as it is today, restyled in dim mono.
  - Context menus and copy actions stay as they are today.
- **Result mark** at the right end of the line:
  - While `output` is null and the turn is running, show the spinner.
  - When the call succeeded, show `✓` in the success color. When it failed, show `✗` in the danger color. *(Owner's decision.)*
  - To know which, carry the `is_error` flag through to the UI. `EventKind::ToolCallResult` already has it (`crates/discard-core/src/event.rs`). `crates/discard-core/src/timeline.rs` drops it when it builds `Block::ToolCall`. Add it to `Block::ToolCall` and to the `tool_call` variant of `BlockDto`. This is the one backend change this design allows.
  - If a result has no flag (for example, data that cannot supply one), show a dim neutral mark that means only "returned". Do not show `✓`.
  - Remove the code comment and the "Returned" status that tell the UI not to infer success. Replace them with the rule above.
  - Show no duration. The block has no timing data, and this work adds none.
- **Diffs (`DiffBlock`).** The header bar is mono and has a muted background. It shows the file path, `+N` in the success color, `−M` in the danger color, and a lowercase `copy` action.
  - Add a line-number gutter.
  - Tint added and removed lines with the diff tokens at about 11%, and color the `+` and `−` signs.
  - Give hunk headers (`@@ … @@`) a faint cyan (`--ansi-6`).
  - Keep the existing diff tokens and their contrast logic.
- **Code blocks (`CodeBlock`).** Use the same mono header style as diffs: language on the left, `copy` on the right.

### 5. Branch tree

This section describes today's sidebar. The mockup shows an older folder-and-branches structure. Do not go back to it.

- In the "Branch channels" section (`ChannelList`, `groupBranchRows`), a branch row (`Hash` icon) can have worktree children (`SidebarWorktree`).
- On those worktree children, box characters in dim mono replace the `CornerDownRight` icon: `├─` for each child, and `└─` for the last child.
- Children keep their `Monitor` icon and the "Main" badge.
- Parent branch rows keep their `Hash` icon.
- The tree has only one level, so there are no `│ ` columns.
- The "Detached worktrees" section and the repository group get no tree characters. *(Claude's call.)*
- The working indicator (spinner) or presence dot sits at the right end of the row.

### 6. Dotted key-values

- In the Context panel (`ContextPanel`, `ProjectChannelDetails`), key-value rows become mono. The key is lowercase and dim. A dotted leader runs from the key to the value, and the value sits at the right.
- Remove the per-row icons.
- Apply this to these two detail lists only. *(Claude's call.)*

### 7. Sharper corners and rules

- The default `--radius` changes from 10 px to 5 px. A radius from the theme or the Appearance settings still wins.
- **Section labels** (Channels, Branches and worktrees, Integrations, and the other labels in the navigation and member list) become mono, uppercase, about 10.5 px, with about 0.08 em letter spacing, and a dashed rule in the border color fills the rest of the row. Collapsible sections keep a small chevron so that collapse stays discoverable. *(Claude's call. The mockup hides the chevron.)*
- The active navigation item gets a 2 px inset edge in the primary color on its left side.
- The active tab underline uses the primary color.

### 8. Status line

- A 24 px bar sits under the window panels across the full window width. It uses the mono font at about 11.5 px, on the background token, with a top border.
- **Left segment:** the project name on the primary color. *(Claude's call. The mockup shows "DISCARD".)*
- **Items,** each shown only when real data exists:
  - the current channel's branch;
  - working agents, each with its spinner in the agent's color;
  - the pending approval count in the warning color, when it is above zero;
  - provider CLI connection state, when the app has that data.
- Show ahead and behind counts only if the snapshot already has them. Add no backend data for them.
- The status line only displays state. It has no clickable items, it is not a plugin slot, and it is always visible. *(Claude's call.)*

## Left to the builder

- Exact spacing, sizes, and timings within the ranges above.
- How to read the Ghostty `font-family` and carry it to the web app.
- How to compute the contrast adjustment for agent colors (the web app or Rust).
- Which snapshot fields feed each status line item.
- Component structure and test layout.

## Proof

- All test commands in [Constraints](#constraints) pass.
- New unit tests cover these:
  - the `ansi-<n>` avatar format: parse, format, and fallback;
  - the hashed slot choice: it is stable for an agent ID;
  - project profile validation with the `ansi-<n>` form;
  - `is_error` reaches `Block::ToolCall` and the DTO, and the UI shows `✓`, `✗`, or the neutral mark correctly;
  - the contrast adjustment;
  - the branch-tree prefix logic;
  - the spinner static frame under reduced motion.
- Visual check in the running app:
  - with Atom One Dark;
  - with Chalk (light);
  - with a plugin theme that does not set `ansi_*` tokens;
  - with reduced motion on.
- Protected surfaces look the same as before the change.
