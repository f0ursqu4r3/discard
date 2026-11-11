# Terminal Visual Direction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle the Discard main window in the approved terminal direction. Keep today's layout and shadcn-vue components, and add eight terminal pieces.

**Architecture:**
- **Rust.** Two small changes:
  - Carry the tool-call `is_error` flag into `Block::ToolCall`.
  - Expose six palette tokens (`ansi_1`–`ansi_6`) and the Ghostty mono font through the theme DTO.
- **Web.** Most of the work:
  - New tokens and Tailwind utilities in `tokens.css`.
  - A pure agent-identity color module.
  - A shared braille spinner.
  - Restyled transcript blocks, sidebar rows, and Context panel rows.
  - A new status line.

**Tech Stack:** Rust (discard-core, discard-app, discard-lua), Vue 3, Tailwind 4, shadcn-vue, Vitest (`vp test`), `@lucide/vue`.

**Spec:** `docs/superpowers/specs/2026-11-07-terminal-visual-design.md`. Read it first. Section numbers below (§1–§8) refer to it. The visual reference is `docs/ui-design-reference/terminal-mix.html`. When the spec and the mockup differ, the spec wins. Agent avatars are circles. The branch tree uses today's sidebar.

## Global Constraints

- Every color comes from a theme token or a `color-mix()` of tokens. Do not hard-code hex values in components. The only exception is the fallback token values in `tokens.css`.
- Ghostty palettes and plugin themes must keep working in dark and light modes.
- Protected surfaces keep the `--protected-*` tokens and their current styling: the approval card, review banner, plugin problem line, confirm notice, and archive notice. `web/src/components/protected.test.ts` must pass unchanged.
- Every animation stops under `prefers-reduced-motion: reduce`. CSS animations are covered by the global rule in `tokens.css`. JavaScript timers check the media query.
- Message prose stays in the system sans font. Do not set mono on prose.
- Do not add a composer key-hint line. `composer.test.ts` asserts it is absent.
- Add no backend data except the `is_error` flag (Task 1).
- Follow `AGENTS.md`. This plan adds no new actions or menu items.
- **Do not commit.** The reviewer commits after the owner approves the diff.
- **Gate for every task:**
  - `pnpm --dir web check`
  - `pnpm --dir web test`
  - `cargo test --workspace` when the task touches Rust
  - `cargo clippy --workspace --all-targets -- -D warnings` when the task touches Rust
- **The final gate** also runs `pnpm --dir web build`.
- Existing tests that assert old visuals change only where this plan changes the visual. Examples: the "Returned" status, the `#789cf5` presets, `rounded-[25%]`, `CornerDownRight`, and the `▍` cursor. Keep each test's behavioral intent.

## Review Focus

1. **A custom hex agent color equal to the background** (for example `#21252b` on Atom One Dark) must still give a readable name, at 4.5:1 or better. Task 3 tests this.
2. **Palette yellow on a light theme** (`#f0c674` on `#ffffff`, which is 1.61:1 raw) must darken until it reaches 4.5:1. Task 3 tests this.
3. **A Ghostty `font-family` value with CSS-breaking characters** (`"`, `\`, `;`, `{`, `}`, a control character) must be rejected and fall back to the default stack. Task 2 tests this.
4. **A tool-call block stored before this change** has no `is_error`. It must deserialize and show the neutral "Returned" mark, never `✓`. Tasks 1 and 5 test this.
5. **A diff without hunk headers, or a new file** (`--- /dev/null`), must not throw. It gets null line numbers and the path from `+++ b/…`. Task 5 tests this.

---

### Task 1: Carry the tool-call error flag

**Files:**
- Modify: `crates/discard-core/src/model.rs` (`Block::ToolCall`, around line 147)
- Modify: `crates/discard-core/src/timeline.rs` (the `ToolCallStart` and `ToolCallResult` arms, around lines 71–89, and the tests at the end of the file)
- Modify: `web/src/bridge/types.ts` (the `tool_call` variant of `BlockDto`)
- Modify: `web/src/components/BlockView.vue` (pass the flag to `ToolCallBlock`)

**Interfaces:**
- Produces: `Block::ToolCall { call_id, name, input, output, is_error: Option<bool> }`. Serde attributes: `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- Produces: TypeScript `{ type: 'tool_call'; call_id: string; name: string; input: unknown; output: string | null; is_error?: boolean }`.
- Produces: `ToolCallBlock` prop `isError?: boolean`, passed by `BlockView` as `block.is_error`. Task 5 renders it.

- [ ] **Step 1: Write failing Rust tests in `timeline.rs`**

```rust
#[test]
fn tool_result_error_flag_reaches_the_block() {
    // Feed ToolCallStart { call_id: "c1", .. } then ToolCallResult { call_id: "c1", output: "boom", is_error: true }.
    // Assert: blocks()[0] is Block::ToolCall { is_error: Some(true), output: Some("boom"), .. }
}
#[test]
fn a_started_tool_call_has_no_error_flag() {
    // Feed only ToolCallStart. Assert: is_error == None.
}
#[test]
fn a_stored_tool_call_without_the_flag_deserializes() {
    let block: Block = serde_json::from_str(
        r#"{"type":"tool_call","call_id":"c","name":"n","input":null,"output":"x"}"#).unwrap();
    assert!(matches!(block, Block::ToolCall { is_error: None, .. }));
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p discard-core timeline`
Expected: compile error (no field `is_error`).

- [ ] **Step 3: Add the field and set it**

`ToolCallStart` sets `is_error: None`. `ToolCallResult` sets `*flag = Some(*is_error)` next to the output.

- [ ] **Step 4: Update the TypeScript type and `BlockView`**

Add `is_error?: boolean` to the TypeScript type. In `BlockView.vue`, pass `:is-error="block.is_error"` to `ToolCallBlock`. Add the `isError?: boolean` prop to `ToolCallBlock` now. Task 5 renders it.

- [ ] **Step 5: Run the gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test`
Expected: all pass.

---

### Task 2: Theme pipeline (palette tokens, mono font, base tokens)

**Files:**
- Modify: `crates/discard-core/src/ghostty/map.rs`. Add `pub ansi: [Rgb; 6]` to `UiColors`, from `palette(1)` to `palette(6)`.
- Create: `crates/discard-core/src/ghostty/font.rs`, and register it in `ghostty/mod.rs`.
- Modify: `crates/discard-core/src/ghostty/resolve.rs`:
  - Add `pub mono_font: Option<String>` to `ResolvedTheme`.
  - Fill it for every `ThemeSource`.
  - Make sure `watched` includes `ghostty_configs` for every source.
- Modify: `crates/discard-app/src/desktop/theme.rs`:
  - `TOKENS` becomes 20 entries, adding `ansi_1` to `ansi_6`.
  - Add the `token_mut` arms `"ansi_1" => &mut ui.ansi[0]` through `ansi_6`.
  - `theme_dto` sets `mono_font`.
- Modify: `crates/discard-app/src/desktop/dto.rs`. Add `pub mono_font: Option<String>` to `ThemeDto`. It serializes as `monoFont`.
- Modify: `crates/discard-lua/src/theme.rs`. Add `ansi_1` to `ansi_6` to `COLOR_TOKENS`. `api_spec.rs` generates the docs from this list.
- Modify: `web/src/bridge/types.ts`. Add `monoFont?: string | null` to `ThemeDto`.
- Modify: `web/src/bridge/theme.ts`
- Modify: `web/src/styles/tokens.css`
- Test: `crates/discard-core/src/ghostty/font.rs` (unit tests), `resolve.rs` tests, `crates/discard-app/src/desktop/theme.rs` tests, `web/src/bridge/theme.test.ts`

**Interfaces:**
- Produces: `pub fn mono_font_family(configs: &[PathBuf]) -> Option<String>` in `font.rs`.
  - Read the files in load order. A later file wins.
  - Within one file, the first non-empty `font-family` value counts. Ghostty adds later lines as fallbacks.
  - A value of `""` in a later file clears the family.
  - Trim the value and strip one pair of surrounding quotes.
  - Return `None` when the result is empty or contains `"`, `\`, `;`, `{`, `}`, or a control character.
- Produces: the CSS custom properties `--ansi-1` to `--ansi-6` (from the theme tokens) and `--mono-family` (set only when `monoFont` is present).
- Produces: `export const activeTheme: ShallowRef<ThemeDto | null>` in `bridge/theme.ts`. `applyTheme` sets it. Both windows already call `applyTheme`.
- Produces these Tailwind theme names in `tokens.css`:
  - `--font-mono: var(--mono-family, 'SF Mono', ui-monospace, Menlo, monospace)`, so the `font-mono` utility follows Ghostty;
  - `--color-ansi-1` to `--color-ansi-6`;
  - `--color-agent: var(--agent-color, var(--primary))`, which gives the `text-agent`, `border-agent`, and `bg-agent` utilities.

- [ ] **Step 1: Write failing Rust tests**

In `font.rs`:

```rust
#[test] fn reads_the_first_family_in_a_file()           // "font-family = \"SF Mono\"\nfont-family = Menlo\n" -> Some("SF Mono")
#[test] fn a_later_file_wins()                           // file1: Iosevka, file2: "SF Mono" -> Some("SF Mono")
#[test] fn an_empty_value_in_a_later_file_clears_it()    // file1: Iosevka, file2: font-family = "" -> None
#[test] fn ignores_comments()                            // "# font-family = Iosevka\nfont-family = SF Mono\n" -> Some("SF Mono")
#[test] fn rejects_css_breaking_values()                 // each of: Foo"; Foo\\; Foo;color:red; Foo{ ; Foo} ; "Foo\u{7}" -> None
#[test] fn missing_files_give_none()
```

In `resolve.rs`:
- `custom_source_still_reads_the_ghostty_font`. With source `Custom` and a config containing `font-family = SF Mono`, `mono_font` is `Some("SF Mono")`, and `watched` contains the config path.

In `map.rs`:
- `ansi_tokens_come_from_palette_1_to_6`. Use the Atom One Dark file text. Expect `ui.ansi[0] == #e06c75` and `ui.ansi[5] == #56b6c2`.

In `desktop/theme.rs`:
- `a_plugin_ansi_token_overrides_and_an_unset_one_keeps_ghostty`. A plugin sets only `ansi_2`. `layer` changes `ansi[1]` and keeps the other five from Ghostty.
- `theme_dto_carries_ansi_tokens_and_mono_font`. The DTO tokens contain `ansi_1` through `ansi_6`.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p discard-core ghostty && cargo test -p discard-app theme`
Expected: compile errors or failures.

- [ ] **Step 3: Implement the Rust side**

Implement the Interfaces above. Plugin themes that set `ansi_*` pass `discard-lua` validation through `COLOR_TOKENS`.

- [ ] **Step 4: Write failing web tests in `web/src/bridge/theme.test.ts`**

```ts
test('applyTheme sets the mono family and the ansi tokens', () => {
  // theme.tokens includes ansi_4: '#61afef'; theme.monoFont = 'SF Mono'
  // expect root.style.getPropertyValue('--ansi-4') === '#61afef'
  // expect root.style.getPropertyValue('--mono-family') === '"SF Mono"'
})
test('applyTheme removes the mono family when the theme has none', () => { /* monoFont: null -> '' */ })
test('applyTheme publishes the active theme', () => { /* activeTheme.value === theme */ })
```

- [ ] **Step 5: Implement `theme.ts` and `tokens.css`**

In `theme.ts`, set `--mono-family` to the family in double quotes. Rust has already rejected unsafe characters.

In `tokens.css`:
- Change `--radius: 10px` to `--radius: 5px`.
- Add the fallback tokens `--ansi-1: #cc6666; --ansi-2: #b5bd68; --ansi-3: #f0c674; --ansi-4: #81a2be; --ansi-5: #b294bb; --ansi-6: #8abeb7`. These are Ghostty's `DEFAULT_PALETTE` values.
- Add the `@theme inline` names listed in Interfaces.
- Add `@keyframes cursor-blink { 50% { opacity: 0 } }`.

- [ ] **Step 6: Run the gate**

Expected: all pass. The radius change may break snapshot-style class assertions. Update those assertions.

---

### Task 3: Agent identity colors, avatars, and pickers

**Files:**
- Modify: `web/src/components/agents/agent-icon.ts`
- Create: `web/src/components/agents/useAgentIdentity.ts`
- Modify: `web/src/components/agents/AgentAvatar.vue`
- Modify: `web/src/components/agents/AgentIconPicker.vue`
- Modify: `web/src/components/workspace/ProjectAvatar.vue`
- Modify: `web/src/components/ServerRail.vue`
- Modify: every `AgentAvatar` call site, which passes `agent-id`:
  - `MemberList.vue`
  - `MessageRow.vue` (`message.author.agentId`)
  - `workspace/WorkspacePage.vue`
  - `workspace/AgentsNavigation.vue`
  - `agents/AgentSummary.vue`
  - `agents/AgentProfileDialog.vue`
  - `agents/FriendsList.vue`
  - `agents/AgentProfileEditor.vue` (for the picker's `fallbackColor`)
- Modify: `crates/discard-core/src/store.rs`. `update_project_profile` must also accept `ansi-[1-6]` as the third part.
- Test: `web/src/components/agents/agent-icon.test.ts`, the `store.rs` tests, and the existing avatar and picker tests

**Interfaces** (all in `agent-icon.ts` unless noted):
- **Stored color.** A stored color string is `'#rrggbb' | 'ansi-1' … 'ansi-6'`. The `parseAgentIcon` regex becomes `/^lucide:([a-z0-9-]*):(#[0-9a-f]{6}|ansi-[1-6])$/i`. `formatAgentIcon` and `validIconColor` accept both forms. Lowercase the result.
- `export type PaletteSlot = 1 | 2 | 3 | 4 | 5 | 6`
- `export const paletteSlots: readonly { slot: PaletteSlot; label: string }[]` with the labels Red, Green, Yellow, Blue, Magenta, Cyan, in slot order 1–6.
- `export function hashedSlot(agentId: string): PaletteSlot` uses FNV-1a 32-bit over UTF-16 code units:
  - start with `h = 0x811c9dc5`;
  - for each code unit, apply `h ^= c; h = Math.imul(h, 0x01000193) >>> 0`;
  - return `(h % 6) + 1`.
- `export function resolveColor(color: string, tokens: Record<string, string>): string` returns a hex value:
  - for `ansi-n`, it returns `tokens['ansi_n']`, or the Task 2 fallback hex for slot n;
  - for a hex value, it returns the value unchanged.
- `export function contrastRatio(a: string, b: string): number` uses WCAG relative luminance. Move the luminance math out of `iconForeground` and reuse it.
- `export function readableOn(color: string, background: string, foreground: string, min: number): string`:
  - return `color` when it already meets `min`;
  - otherwise mix it toward `foreground` in 10% steps and return the first mix that meets `min`;
  - return `foreground` when no step meets it.
- `export function agentIdentity(avatar: string | null | undefined, agentId: string, tokens: Record<string, string>): { color: string; text: string; mark: string }`:
  - The base color is the parsed stored color. Without one (null, an image URL, or another unparsed value), it is `ansi-${hashedSlot(agentId)}`.
  - `text` is `readableOn(base, bg, fg, 4.5)`. `mark` is `readableOn(base, bg, fg, 3)`.
  - `bg` and `fg` come from `tokens.background` and `tokens.foreground`. The fallbacks are `#1e1e1e` and `#f0f0f0`.
- `useAgentIdentity(agentId: MaybeRefOrGetter<string | undefined>, avatar: MaybeRefOrGetter<string | null | undefined>): ComputedRef<{ color; text; mark }>`, in `useAgentIdentity.ts`. It reads `activeTheme.value?.tokens ?? {}`.
- **`AgentAvatar`** gets a new prop `agentId?: string`. The rendering for parsed avatars and hashed fallbacks:
  - `rounded-full`;
  - background `color-mix(in srgb, mark 15%, var(--background))`;
  - icon or initials in `mark`;
  - ring `box-shadow: inset 0 0 0 1px color-mix(in srgb, mark 40%, transparent)`.
  An image URL still renders `AvatarImage`. Keep `data-agent-color` as the stored color string.
- **`AgentIconPicker`** gets a new prop `fallbackColor?: string`.
  - It shows six swatches from `paletteSlots`. Each swatch background is `var(--ansi-n)`, its `aria-label` is `Use red` and so on, and selecting one stores `ansi-n`.
  - The custom color input shows `resolveColor(current, activeTheme tokens)` and stores the hex value it receives.
  - The default when nothing is stored is `fallbackColor ?? 'ansi-4'`.
  - `AgentProfileEditor` passes `` `ansi-${hashedSlot(agent.id)}` ``.
- **`ProjectAvatar` and the `ServerRail` project icons** become squares. Replace `rounded-[25%]` and `rounded-xl` with `rounded-lg`, which is `var(--radius)`, now 5 px. The color is `resolveColor(stored, tokens)`, and the foreground is `iconForeground(resolved)`.

- [ ] **Step 1: Write failing tests in `agent-icon.test.ts`**

```ts
test('parses and formats palette slots', () => {
  expect(parseAgentIcon('lucide:bot:ansi-5')).toEqual({ icon: 'bot', color: 'ansi-5' })
  expect(parseAgentIcon('lucide::ANSI-2')).toEqual({ icon: null, color: 'ansi-2' })
  expect(parseAgentIcon('lucide:bot:ansi-7')).toBeNull()
  expect(formatAgentIcon('bot', 'ansi-3')).toBe('lucide:bot:ansi-3')
})
test('hashedSlot is stable and in range', () => {
  expect(hashedSlot('agent-a')).toBe(5)
  expect(hashedSlot('agent-b')).toBe(2)
  expect(hashedSlot('coordinator')).toBe(6)
  expect(hashedSlot('0194f3a2-7c1e-7b8a-9d2e-5f6a7b8c9d0e')).toBe(4)
})
test('resolveColor uses the theme token, then the fallback', () => {
  expect(resolveColor('ansi-4', { ansi_4: '#61afef' })).toBe('#61afef')
  expect(resolveColor('ansi-4', {})).toBe('#81a2be')
  expect(resolveColor('#123456', {})).toBe('#123456')
})
test('readableOn keeps a readable color', () => {
  expect(readableOn('#61afef', '#21252b', '#abb2bf', 4.5)).toBe('#61afef') // 6.51:1
})
test('readableOn fixes a color equal to the background', () => {
  const out = readableOn('#21252b', '#21252b', '#abb2bf', 4.5)
  expect(contrastRatio(out, '#21252b')).toBeGreaterThanOrEqual(4.5)
})
test('readableOn darkens palette yellow on a light theme', () => {
  const out = readableOn('#f0c674', '#ffffff', '#000000', 4.5) // 1.61:1 raw
  expect(contrastRatio(out, '#ffffff')).toBeGreaterThanOrEqual(4.5)
})
test('agentIdentity hashes when nothing is stored or the avatar is an image', () => {
  const tokens = { ansi_5: '#c678dd', background: '#21252b', foreground: '#abb2bf' }
  expect(agentIdentity(null, 'agent-a', tokens).color).toBe('#c678dd')
  expect(agentIdentity('https://x/y.png', 'agent-a', tokens).color).toBe('#c678dd')
})
```

Add component tests:
- `AgentAvatar` with `agent-id="agent-a"` and no avatar renders `rounded-full`, and its icon color equals the `mark`.
- `AgentIconPicker` renders six swatches labeled "Use red" … "Use cyan", and clicking "Use magenta" emits an avatar ending in `:ansi-5`.

- [ ] **Step 2: Write the failing Rust test in `store.rs`**

`project_profile_accepts_palette_slots`: `lucide:folder:ansi-4` passes. `lucide:folder:ansi-0` and `lucide:folder:ansi-7` return `StoreError::Invalid`.

- [ ] **Step 3: Run both test suites and confirm they fail**

Run: `pnpm --dir web test agent-icon && cargo test -p discard-core project_profile`
Expected: FAIL.

- [ ] **Step 4: Implement the Interfaces, then update the call sites**

Update every call site in the Files list.

- [ ] **Step 5: Run the gate**

Update the existing picker and avatar tests that assert the old eight presets, `#789cf5`, solid fills, or `rounded-[25%]`.

---

### Task 4: Braille spinner and block cursor

**Files:**
- Create: `web/src/components/spinner.ts`
- Create: `web/src/components/BrailleSpinner.vue`
- Modify these files to show the spinner where an agent is working:
  - `web/src/App.vue`: the toolbar "Working" indicator. Replace the `LoaderCircle`.
  - `web/src/components/MemberList.vue`: replace the status dot when `status === 'working'`.
  - `web/src/components/workspace/AgentsNavigation.vue`: rows of working agents.
  - `web/src/components/ChannelList.vue` and `web/src/components/SidebarWorktree.vue`: the row of the channel or worktree whose agent is working, at the row's right end. Use the running state these components already receive.
  - The conversation tab bar. Find it with `grep -rn "Merge review" web/src/components`. Add an activity note at its right end while `snapshot.turnRunning`: `<BrailleSpinner/> {agent name} is working`, in mono, muted, with the spinner in the agent's color.
- Modify: `web/src/components/MessageRow.vue` (the `stream-cursor` span)
- Test: `web/src/components/spinner.test.ts`

**Interfaces:**
- Produces, in `spinner.ts`:
  - `export const SPINNER_FRAMES = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'`
  - `export const SPINNER_INTERVAL_MS = 90`
  - `export const STATIC_FRAME = '⠿'`
  - `export function useSpinnerFrame(): Readonly<Ref<string>>`
- **The shared timer.**
  - One module-level `setInterval` starts with the first subscriber and stops when the last subscriber unmounts (`onScopeDispose`).
  - When `matchMedia('(prefers-reduced-motion: reduce)').matches`, return `STATIC_FRAME` and start no timer. Listen for `change` on that media query.
- **`BrailleSpinner.vue`** takes the prop `label?: string`, default `'Working'`. It renders `<span role="img" :aria-label="label" class="font-mono text-agent">{{ frame }}</span>`. The color comes from `--agent-color` when an ancestor sets it. Otherwise it is primary.
- **Agent color source.** Set `--agent-color` with `useAgentIdentity(...).value.mark` on the row or element that owns the spinner.
- **Block cursor.** Change the `stream-cursor` span in `MessageRow.vue`:
  - remove the text `▍`;
  - classes: `stream-cursor inline-block h-[1.1em] w-[0.55em] align-[-0.2em] bg-foreground animate-[cursor-blink_1.05s_steps(1)_infinite]`.
  The global reduced-motion rule turns it solid.

- [ ] **Step 1: Write failing tests in `spinner.test.ts`**

```ts
test('advances one frame per interval', () => { /* fake timers; mount a host using useSpinnerFrame; advance 90ms; frame === SPINNER_FRAMES[1] */ })
test('subscribers share one timer', () => { /* spy setInterval; mount two hosts; called once; unmount both; clearInterval called once */ })
test('reduced motion shows the static frame and starts no timer', () => { /* stub matchMedia matches=true; frame === '⠿'; setInterval not called */ })
```

In `MemberList`, add a test: a member with `status: 'working'` renders `role="img"` with the name "Working" and no status dot.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `pnpm --dir web test spinner`
Expected: FAIL (module not found).

- [ ] **Step 3: Implement the Interfaces and replace the indicators**

Replace the indicators in the files listed above.

- [ ] **Step 4: Run the gate**

Update the assertions that look for `▍` or `LoaderCircle`.

---

### Task 5: Transcript (message header, tool calls, diffs, code blocks)

**Files:**
- Modify: `web/src/components/MessageRow.vue`
- Modify: `web/src/components/ToolCallBlock.vue`
- Modify: `web/src/components/DiffBlock.vue`
- Modify: `web/src/components/messages.ts`, which gets `parseDiff`
- Modify: `web/src/components/ui/code-block/CodeBlockHeader.vue` and `CodeBlockTitle.vue`, which give the shared mono header for diffs and code
- Modify: `web/src/components/BlockView.vue`. Pass `running` to `ToolCallBlock`: true for blocks of the live row while `turnRunning`.
- Test: `web/src/components/messages.test.ts`, plus a new or existing `ToolCallBlock` test

**Interfaces:**
- **`MessageRow`, agent rows.**
  - Set `style="--agent-color: <identity.mark>"` on the row root.
  - The name gets `color: identity.text`.
  - Do not add a provider tag after the name. An earlier version of this plan added `[agent · <provider>]`. The tag was removed.
  - The timestamp gets `font-mono`.
  - User rows stay as they are.
- **`ToolCallBlock`** props: `name`, `input`, `output`, `isError?: boolean`, `running?: boolean`. The block stays collapsed by default.
  - The trigger is one mono line: the chevron, `$` in `text-agent`, the action label lowercased (from today's `label`), then the `summary`.
  - The wrapper gets `border-l-2 border-agent rounded-none`, with background `color-mix(in srgb, var(--canvas) 55%, var(--background))`.
  - The expanded content keeps its structure. Its labels and output become dim mono.
- **The right-end mark of `ToolCallBlock`.** It has exactly one state, and each state has an `aria-label`:
  - `output === null && running` → `<BrailleSpinner label="Running"/>`
  - `output === null && !running` → the dim `Clock3` icon, with `aria-label="Awaiting result"`
  - `isError === false` → `✓` in `text-success`, with `aria-label="Succeeded"`
  - `isError === true` → `✗` in `text-destructive`, with `aria-label="Failed"`
  - `isError === undefined` and output present → a dim `↵`, with `aria-label="Returned"`

  Delete the "Do not infer success" comment and the old `status` computed.
- **`parseDiff`** in `messages.ts`:

  ```ts
  export function parseDiff(text: string): {
    path: string | null
    added: number
    removed: number
    lines: { text: string; kind: 'added' | 'removed' | 'hunk' | 'context'; number: number | null }[]
  }
  ```

  - The path comes from `+++ b/<path>`, with `b/` stripped. When that is `/dev/null`, it comes from `--- a/<path>`. Otherwise it is null.
  - Remove the meta lines (`diff --git`, `index`, `---`, `+++`) from `lines`.
  - Hunk headers `@@ -a[,b] +c[,d] @@` set the counters. Removed lines take the old counter, and added and context lines take the new counter. Hunk lines get `number: null`.
  - Before any hunk header, every number is null.
  - `added` and `removed` count only the `+` and `-` content lines.
- **`DiffBlock`, not inline.**
  - The header shows `path ?? 'diff'` in mono, then `+{added}` in `text-success` and `−{removed}` in `text-destructive`, then the copy button.
  - Each line is a grid: a gutter of about 3ch for the number (right-aligned, muted), the sign, then the text.
  - Added and removed lines get the background `color-mix(in srgb, var(--diff-added|--diff-removed) 11%, transparent)`, and their signs use the diff colors.
  - Hunk lines are `text-ansi-6` on `color-mix(in srgb, var(--ansi-6) 6%, transparent)`.
  - Keep `data-diff` on each line. Inline mode (inside tool output) keeps no header and no gutter, but it gets the line tints.
- **`CodeBlockHeader` and `CodeBlockTitle`:** mono, `bg-muted`, `border-b`, `py-1.5`. The title is lowercase, for example the language or "diff". Code blocks keep their language title and their copy button.

- [ ] **Step 1: Write failing tests**

In `messages.test.ts`:

```ts
test('parseDiff numbers lines and reads the path', () => {
  const d = parseDiff('--- a/src/worktree.rs\n+++ b/src/worktree.rs\n@@ -12,3 +12,5 @@\n fn cleanup() {\n-    remove();\n+    if ok {\n+        remove();\n+    }\n }')
  expect(d.path).toBe('src/worktree.rs')
  expect([d.added, d.removed]).toEqual([3, 1])
  expect(d.lines.map((l) => [l.kind, l.number])).toEqual([
    ['hunk', null], ['context', 12], ['removed', 13], ['added', 13], ['added', 14], ['added', 15], ['context', 16],
  ])
})
test('parseDiff handles a new file and a diff without hunks', () => {
  expect(parseDiff('--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+hi').path).toBe('new.txt')
  const bare = parseDiff('+a\n-b')
  expect(bare.lines.every((l) => l.number === null)).toBe(true)
  expect(bare.path).toBeNull()
})
```

For `ToolCallBlock`, write one test per right-end state. Each test finds the mark by its `aria-label`: "Running", "Awaiting result", "Succeeded", "Failed", and "Returned". Also assert that `$` renders. Add a `BlockView` test: a stored `tool_call` block without `is_error` and with output shows "Returned" and no "Succeeded".

For `MessageRow`, add these tests:
- an agent message from provider `claude` renders no agent tag;
- a user message renders no agent tag.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `pnpm --dir web test messages ToolCallBlock MessageRow`
Expected: FAIL.

- [ ] **Step 3: Implement the Interfaces**

- [ ] **Step 4: Run the gate**

---

### Task 6: Navigation, panels, and mono structure

**Files:**
- Modify: `web/src/styles/tokens.css`, which gets the utilities below.
- Modify the section labels and active rows in:
  - `web/src/components/ChannelList.vue`
  - `web/src/components/workspace/NavigationPanel.vue`
  - `web/src/components/workspace/DirectMessageSidebar.vue`
  - `web/src/components/workspace/AgentsNavigation.vue`
  - `web/src/components/MemberList.vue`
- Modify: `web/src/components/SidebarWorktree.vue` and `web/src/components/branch-navigation.ts` (the tree prefix)
- Modify: `web/src/components/workspace/ContextPanel.vue` and `web/src/components/workspace/ProjectChannelDetails.vue` (the key-value rows)
- Modify: `web/src/components/ChannelHeader.vue` and the project header in the navigation (mono names and paths)
- Modify: `web/src/components/Composer.vue` (the `›` glyph)
- Modify: the conversation tab bar (see Task 4). The active tab underline uses `--color-primary`.
- Test: `web/src/components/branch-navigation` tests, `ChannelList.*.test.ts`, and the Context panel tests

**Interfaces:**
- **Utilities in `tokens.css`** (Tailwind 4 `@utility`):
  - `section-label`:
    - `display:flex; align-items:center; gap:.375rem;`
    - `font-family: var(--font-mono); font-size: 10.5px; font-weight: 500;`
    - `letter-spacing: .08em; text-transform: uppercase; color: var(--color-muted-foreground);`
    - `&::after { content:''; flex:1; border-top:1px dashed var(--color-border) }`
  - `nav-active-edge`: `box-shadow: inset 2px 0 0 var(--color-primary)`
  - `kv-leader`: `flex:1; min-width:1rem; margin-inline:.375rem; border-bottom:1px dotted color-mix(in srgb, var(--color-muted-foreground) 60%, transparent); transform: translateY(-0.25em)`
- **Section labels.** Apply `section-label` to every section heading in the five navigation and member files: Channels, Branch channels, Detached worktrees, Integrations, Members, and the DM and agent groups. Collapsible headings keep a `size-3` chevron before the text. Counts stay after the text, so the rule fills the space after the count.
- **Active rows.** Add `nav-active-edge` to the selected row class in the same five files.
- **Branch tree.**
  - Add `export function treePrefix(isLast: boolean): '├─' | '└─'` to `branch-navigation.ts`.
  - `SidebarWorktree` gets the prop `isLast?: boolean`.
  - For non-detached rows, replace `CornerDownRight` with `<span aria-hidden="true" data-testid="tree-prefix" class="w-[2ch] shrink-0 font-mono text-muted-foreground/70">{{ treePrefix(isLast) }}</span>`.
  - `ChannelList` passes `is-last` for the last worktree child of each branch row.
  - Detached rows and repository groups get no prefix.
- **Key-value rows.**
  - Each `dt`/`dd` row in `ContextPanel` becomes `flex items-baseline font-mono text-xs`.
  - The `dt` is lowercase and muted, and it has no icon. A `<span aria-hidden="true" class="kv-leader"/>` follows it, and the `dd` sits right-aligned.
  - Apply the same pattern to the key-value rows in `ProjectChannelDetails`. Its member list (`li` rows) is not a key-value list. Leave it alone.
- **Mono.** Add `font-mono` (with `text-[0.92em]` when inside sans text) to:
  - channel, branch, and worktree names in the sidebar rows and the channel header;
  - the project path in the navigation header and the channel header;
  - count badges.

  Do not change navigation labels (Overview, Agents, Memory, Artifacts, Settings, CLI connections, Lua plugins, Approvals), tab labels, buttons, or prose.
- **Composer.** Put `<span aria-hidden="true" class="font-mono text-primary">›</span>` before the textarea inside the input group. The placeholder text stays as it is. A native placeholder cannot mix fonts.

- [ ] **Step 1: Write failing tests**

- In the `branch-navigation` tests, assert that `treePrefix(false)` is `'├─'` and `treePrefix(true)` is `'└─'`.
- In the `ChannelList` repositories test, a branch with two worktree children renders `tree-prefix` texts `['├─', '└─']`, and a detached worktree renders none.
- In the Context panel test, the branch row renders the `dt` text `branch`, and it contains an element with the class `kv-leader`.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `pnpm --dir web test branch-navigation ChannelList ContextPanel`
Expected: FAIL.

- [ ] **Step 3: Implement the Interfaces**

- [ ] **Step 4: Run the gate**

---

### Task 7: Status line

**Files:**
- Create: `web/src/components/StatusLine.vue`
- Create: `web/src/components/workspace/channel-branch.ts`. Move the branch logic from `ContextPanel.vue` (around lines 38–40) into it, and use it from both places.
- Modify: `web/src/App.vue`. Render `<StatusLine>` inside `<main data-ui-id="app-shell">`, after the shell container (`<div v-if="store.snapshot" class="flex min-h-0 flex-1 …">`). Render it only when `store.snapshot` exists.
- Test: `web/src/components/StatusLine.test.ts`

**Interfaces:**
- Produces: `export function channelBranch(snapshot: SnapshotDto, projectChannels: ProjectChannelDto[] | undefined): string | null`. It returns the current channel's metadata branch, or the current branch row's label, or null.
- **`StatusLine` props:**

  ```ts
  {
    project: string
    branch: string | null
    workingAgents: { id: string; name: string; avatar: string | null }[]
    pendingApprovals: number
  }
  ```

- **Markup.**
  - The root is `<footer data-ui-id="status-line" class="flex h-6 shrink-0 items-center gap-4 overflow-hidden whitespace-nowrap border-t bg-background pr-3 font-mono text-[11.5px] text-muted-foreground">`.
  - The left segment is `bg-primary text-primary-foreground font-bold px-2.5 h-full flex items-center` and shows the project name.
  - Then, in order:
    - `⎇ {branch}` when the branch exists;
    - one item for each working agent: a span with `--agent-color` from `useAgentIdentity`, then `<BrailleSpinner/>`, then the name in lowercase;
    - `▲ {n} approval(s)` in `text-warning` with `ml-auto` when `pendingApprovals > 0` ("1 approval", "2 approvals").
  - There is no provider item, because the snapshot has no connection data, and there are no clickable elements.
- **`workingAgents` in `App.vue`.** Use the same agent status data that `MemberList` and `AgentsNavigation` use, and take the agents whose status is `'working'`. When that data is not at app level, use the current channel's agent while `snapshot.turnRunning`. Pass `snapshot.pendingApprovals`.

- [ ] **Step 1: Write failing tests in `StatusLine.test.ts`**

```ts
test('shows project, branch, working agents, and approvals', () => { /* text contains project, '⎇ feat/x', agent name; '▲ 2 approvals' */ })
test('hides the branch when null and approvals at zero', () => { /* no '⎇', no '▲' */ })
test('uses the singular for one approval', () => { /* '▲ 1 approval' */ })
test('has no buttons or links', () => { /* findAll('button,a').length === 0 */ })
```

In the `channel-branch` tests:
- the metadata branch wins over the current row;
- with no data, the result is null.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `pnpm --dir web test StatusLine channel-branch`
Expected: FAIL.

- [ ] **Step 3: Implement the component and wire it into `App.vue`**

- [ ] **Step 4: Run the gate**

---

### Task 8: Final gate

- [ ] **Step 1: Run every check**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build`
Expected: all pass, with no new warnings.

- [ ] **Step 2: Report**

List:
- every file changed;
- every existing test assertion that changed, and why;
- anything in the spec that was not built.

Do not commit.

The reviewer then checks the running app (`cd crates/discard-app && cargo tauri dev`) with:
- Atom One Dark;
- Chalk;
- a plugin theme that sets no `ansi_*` tokens;
- reduced motion on.
