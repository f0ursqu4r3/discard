# Lua elements match the app style — design

**Date:** 2026-11-07
**Status:** Approved by the owner
**Builds on:** [Terminal visual direction](2026-11-07-terminal-visual-design.md)

## Intent

Lua plugins exist so that Discard is easy to extend. A user can ask an agent inside Discard to write a plugin, for example one that shows live token usage or graphs a git change history. Agents write these plugins from the generated plugin API docs (`crates/discard-lua/src/api_spec.rs`, which generates the Luau types and the markdown reference).

Elements that plugins draw must match the app's style. The terminal restyle did not reach plugin UI. The bundled `default-ui` plugin draws two slots, `channel_header` and `member_list`, and both still use the old look.

## Rule

Plugin elements render with the same components, utilities, and tokens as the Vue UI. A restyle of the app then reaches plugins with no plugin change. A few new properties let plugins express the new structure.

## Goals

### 1. Restyle the existing elements with no API change

Renderer: `web/src/plugin/PluginElement.vue`. Property filter: `web/src/plugin/props.ts`.

- `avatar` renders through `AgentAvatar`. It is a tinted circle in the identity color, like the Vue UI. Without an `agent` prop, use the `name` as the hash key, so different names get different colors.
- `badge` counts use `font-mono tabular-nums`, like the Vue sidebar counts.
- Element color tokens also accept `ansi_1` to `ansi_6`. Rust already accepts them in `COLOR_TOKENS`, but `props.ts` does not.

### 2. Add these API properties

| Addition | Applies to | Effect |
| --- | --- | --- |
| `agent = "<id>"` | `avatar` | Draws that agent's own avatar: its stored icon and identity color. The stored avatar comes from the shared `agentAvatars` catalog (`web/src/components/agents/useAgentAvatar.ts`). |
| `agent = "<id>"` | `row`, `column` | Sets that agent's identity for every element inside. Set `--agent-color` to the identity `mark` and `--agent-text` to the identity `text`, from `agentIdentity` in `agent-icon.ts`. |
| color token `agent` | `color` on `text`, `row`, `column`; `dot` | For text it is `var(--agent-text, var(--primary))`. For a `dot` it is `var(--agent-color, var(--primary))`. Do not add `agent` to `COLOR_TOKENS` in `discard-lua`, because that list is also the plugin theme token list. A plugin theme must not be able to set `agent`. |
| `variant = "label"` | `text` | A section label: the `section-label` utility from `tokens.css` (mono, uppercase, dashed rule). |
| `variant = "mono"` | `text` | Mono structure text: `font-mono text-[0.92em]`, like channel and branch names. |
| new tag `spinner` | anywhere | The shared braille spinner (`web/src/components/BrailleSpinner.vue`). It has one prop, `label` (string, default `"Working"`), which is its accessible name. It uses the agent color inside an `agent` container. Otherwise it uses primary. It stops under reduced motion, like every other spinner. |
| `agent.id` | slot `state` | The agent's id, so a slot can pass it to the props above. Add it in `slot_state` (`crates/discard-core/src/ui_state.rs`) through `SlotLabels`, and fill it in `crates/discard-app/src/desktop/core.rs`. |

An unknown `variant` value on `text` is ignored, and the text renders as plain text. An `agent` id that matches no agent still gets a hashed identity color, and its avatar shows initials.

### 3. Document every addition for plugin authors

Agents read these docs, so each entry must say what the property does, what values it takes, and where its value comes from.

- `api_spec.rs` `tag_props`: add `agent` to the layout props and to `avatar`; add `variant` and the `agent` color value to `text`; add the `spinner` tag with `label`; document `agent` as a value for `dot` `color`.
- Document the color token set once, everywhere it appears. The set is the theme tokens, `ansi_1` to `ansi_6`, and `agent`. `agent` is valid only for element colors.
- Update `SLOT_STATE_DOC` with `agent.id`.
- The Luau types and the markdown reference are generated from the spec. Check that both show the new tag and props.
- If `docs/plugins.md` lists element tags or props, update it to match.

### 4. Update the default UI

File: `crates/discard-lua/bundled/default-ui/init.lua`. Match the Vue components exactly.

- `channel_header`: the channel name uses `variant = "mono"`, like `ChannelHeader.vue` for channels.
- `member_list`: match `MemberList.vue`.
  - The column sets `agent = agent.id`.
  - The heading is `{ "text", "Agents", variant = "label" }`.
  - The avatar uses `agent = agent.id`.
  - The name uses `color = "agent"` and `weight = "semibold"`.
  - The status line keeps its current text.
  - While the status is `working`, show a `spinner` at the end of the row and give the avatar no status dot. In every other state, keep the status dot.
  - Build the children list without `nil` holes.

## Non-goals

- No new layout props, and no chart or graph elements. Data-display elements for plugins like token usage or heat graphs are a separate future design.
- No change to theme tokens, and no change to which tokens a plugin theme may set.
- No change to the protected surfaces, which stay Rust-rendered.
- No plugin API version change. `API_VERSION` stays `"1"`. Manifests must match it exactly, and every addition is backward compatible.

## Constraints

- Every color comes from a token, a `color-mix()` of tokens, or the identity functions in `agent-icon.ts`.
- Plugin text is still plain text. Do not render plugin strings as HTML.
- `PROP_WHITELIST` in `props.ts` still filters props. Add `agent`, `variant` (it already exists for buttons, so check `text` handles it), and `label`.
- Follow `AGENTS.md`.
- Do not commit.

## Proof

- **Rust tests.**
  - `element.rs` parses `{ "spinner", label = "Syncing" }` to `Tag::Spinner`.
  - The generated Luau types and markdown name `spinner`, `agent`, `variant`, and `agent.id`.
  - `slot_state` includes `agent.id`.
  - `bundled.rs` renders `member_list` with a spinner and no status dot while working, and with a status dot while idle.
- **Web tests** (`web/src/plugin/plugin.test.ts`):
  - `avatar` with `agent` renders `AgentAvatar` in that agent's identity color;
  - `avatar` without `agent` still renders a tinted circle;
  - `text` with `variant = "label"` has the `section-label` class;
  - `text` with `variant = "mono"` has `font-mono`;
  - an unknown `variant` adds no class;
  - `color = "agent"` inside an `agent` row resolves to `--agent-text`;
  - `spinner` renders `role="img"` with its label;
  - `badge` is mono;
  - `ansi_3` is accepted as a color.
- **All checks:**

  ```sh
  cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build
  ```

- **Visual check by the reviewer** in `cargo tauri dev`. The Agents card and the channel header must match the Vue components.
