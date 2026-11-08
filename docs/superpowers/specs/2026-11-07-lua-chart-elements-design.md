# Lua chart elements — design

**Date:** 2026-11-07
**Status:** Approved by the owner
**Builds on:** [Terminal visual direction](2026-11-07-terminal-visual-design.md), [Lua elements match the app style](2026-11-07-lua-element-style-design.md)

## Intent

Discard uses Lua plugins so that it is easy to extend. A user asks an agent to write a plugin, for example "show live token usage" or "graph the git change history as a heat map". The agent can run inside Discard or outside it. The agent writes the plugin from the generated API reference. The reference is generated from `crates/discard-lua/src/api_spec.rs`.

Today a plugin cannot do this. It cannot draw data. It has no place to draw that does not replace built-in UI. It cannot redraw when its own data changes. It cannot read token counts or git history.

This design adds the following:
- data-display elements
- a panel slot
- a plugin data store that triggers redraws
- timers
- two read-only data sources

It also writes out the doc strings that agents read. An agent gets a plugin right only when the docs are exact. Discard dogfoods the result: the bundled `default-ui` plugin gets the token-usage panel.

## Goals

1. Six chart elements that Discard draws in the app's style: `stat`, `sparkline`, `meter`, `bars`, `heatgrid`, `table`.
2. A place for charts that adds to the UI and replaces nothing: `discard.panel`.
3. A way for plugin data to reach render and cause a redraw: `discard.data` and `state.data`.
4. Timers, a view-change event, and local-date helpers, so plugins can refresh and bucket data.
5. Token counts for each finished turn, with one meaning across providers: the `usage` event and `discard.usage.history`.
6. Git history of the active project: `discard.git.log`.
7. Host data calls that never block the plugin thread.
8. Agents inside and outside Discard can read the same reference. A complete git example in the guide and a bundled token-usage panel show the whole pattern.

## Non-goals

- Drawing primitives (rects, paths, canvas). Charts are fixed elements. A new chart type is a new element that Discard adds.
- Multi-series charts, pie and donut charts, axis charts, and line charts with axes.
- Click actions on data points.
- Animation of any kind.
- Context-window and cost data. Discard does not keep either. Codex sends a context-window size (`modelContextWindow`), and Discard drops it. No example or doc string may suggest that Discard has this data.
- Git commands other than the log. No git writes.
- A CLI to test or submit a plugin from outside Discard. That is a separate design (see [Follow-ups](#follow-ups)).
- User reordering of panel cards.
- Persistence of `discard.data`. Plugins that must keep data use `memory` or `fs`.

## Anti-goals

The work fails, even if it runs, when any of these is true:

- A chart color is hard-coded, or a plugin can pass a hex color to a chart.
- A chart is readable only through color, or only through hover.
- A chart is unreadable in a light theme (for example Chalk) or in a dark theme (for example Atom One Dark).
- Something moves when the system asks for reduced motion.
- Discard silently clamps or invents data. Invalid element data must be an error.
- A plugin string renders as HTML.
- A doc string or example implies data that Discard does not have.
- Token counts mean different things for different providers.
- A slow host data call makes a plugin time out, fail, or revert.
- An agent that follows the generated docs exactly writes a plugin that fails `plugin_test`.

## Constraints

- `API_VERSION` stays `"1"`. Manifests must match it exactly. Every addition is backward compatible: an existing plugin loads and renders the same as before.
- **Build order.** Build this design after the [element style design](2026-11-07-lua-element-style-design.md) has landed. This design needs things that do not exist yet:
  - the `agent` container and color;
  - `agent.id` in `state`;
  - `ansi_*` tokens in `web/src/plugin/props.ts`.
- Plugin elements render through the same components, utilities, and tokens as the Vue UI.
- Every color comes from a theme token, `ansi_1`–`ansi_6`, the `agent` identity color, or a `color-mix()` of these.
- Rust validates every element tree (`crates/discard-lua/src/element.rs`). The web renderer (`web/src/plugin/PluginElement.vue`, `web/src/plugin/props.ts`) still filters props and never trusts types.
- Protected surfaces stay Rust-rendered. A plugin cannot draw on them.
- Follow `AGENTS.md`. This design adds no unimplemented controls.
- The `api_spec.rs` tests that check the docs against the runtime stay, and they cover every new function, namespace, tag, and prop.

## Decisions

All decisions below are the owner's unless marked *(Claude's call)*.

### 1. Fixed chart elements

Charts are a fixed set of element tags. Discard owns scales, color, contrast, layout, number format, and accessibility. A plugin supplies data and a few options.

### 2. Placement: `discard.panel`

- `discard.panel(id, title, render)` adds a card to the right-hand context panel (`ContextPanel.vue`), below the Members card. It needs the `ui` capability.
- Many plugins can show panels at the same time. A panel replaces nothing.
- `id` uses lowercase letters, digits, and `-`, and is unique within the plugin. `title` has 1 to 40 characters. *(Claude's call.)*
- The card title uses the `section-label` style. The card can collapse. Discard remembers the collapsed state for each card, keyed by plugin name and panel id.
- Cards appear in plugin load order, then in registration order. A plugin may register at most 4 panels.
- A render error hides that card and shows the protected plugin problem line. Other cards still render.
- Panels are hidden in direct messages, like the `member_list` plugin slot. (The built-in Members card still shows there.)
- `ui_inspect` lists panels as well as slots. *(Claude's call.)*

### 3. Data inside elements: list props

- Chart tags accept list-valued props: arrays of numbers, arrays of strings, arrays of arrays, and arrays of records. Each tag accepts lists only on the props named in [Elements](#elements).
- Layout tags and the other existing tags still accept only scalar props. A table prop on them is still an error, so `rejects_functions_and_tables_as_property_values` stays.
- An array has keys `1..n` only, with no holes and no string keys. A record has string keys only.
- Lists serialize to JSON arrays and objects in `ElementDto` props.

### 4. Plugin data and redraw: `discard.data`

- `discard.data.set(key, value)` and `discard.data.get(key)` need no capability.
- A value is nil, a boolean, a finite number, a string, or a table of these, nested at most 8 levels deep.
- A plugin stores at most 64 KB in total, measured as JSON. A `set` that goes over the cap is an error, and the old value stays.
- `get` returns a copy. Changing the copy does not change the store.
- The data lives in memory only. A plugin reload or an app restart clears it.
- Calling `discard.data.set` inside a render function is an error.
- Render receives the plugin's own data as `state.data`:
  - It is always a table, and it is empty when nothing is stored.
  - A plugin never sees another plugin's data.
  - `state.data` reaches the slots (`channel_header`, `member_list`) as well as panels. *(Claude's call.)*
- A `set` marks the plugin's slots and panels for redraw. The host redraws a plugin at most 4 times per second. It coalesces sets, and the last value wins.
- The render cache (`SlotCache` in `crates/discard-app/src/desktop/slots.rs`) keys on the slot state plus a version number for each plugin's data. Today it keys on the slot state only.

### 5. Timers: `discard.every`

- `discard.every(seconds, run)` calls `run` every `seconds` seconds. The minimum is 10. A plugin may have at most 4 timers.
- A timer runs on the plugin thread under the normal per-call budget. If the plugin queue is full, Discard skips that tick. It does not queue it.
- An error in a timer is logged, like an error in an event handler (`runtime.rs`, `event`). It does not count toward a revert. Only a limit violation stops a plugin. *(Claude's call. This matches the code.)*
- Timers stop when the plugin unloads. `plugin_test` does not run timers.

### 6. View changes: the `view_changed` event

- `view_changed` fires once after the plugin loads, with the current view. Then it fires each time the active project, channel, or agent changes. *(The first fire is Claude's call. It gives plugins one place for their first fetch.)*
- The payload is `{ project, project_id, channel, channel_id, agent_id }`. The owner chose `project`, `channel`, and `agent_id`. *(Claude added the two ids, because names are not unique keys.)*
- The active project is the active channel's project, the same one that slot `state` uses (`state.active.project_id`). It is not the project the user browses in the sidebar. In a direct message, `project` and `project_id` are nil. *(Claude's call.)*
- No capability is needed.

### 7. Token usage data

Facts:

- **Claude.** It reports usage on each message and in the final `result`. Discard keeps only `result.usage` (`crates/discard-providers/src/claude/parse.rs`). Claude's `input_tokens` excludes the cache. It reports `cache_read_input_tokens` and `cache_creation_input_tokens` separately. In the fixture `claude-approval-turn.jsonl`, the values are 2, 7569, and 3440.
- **Codex.** It sends `thread/tokenUsage/updated` during a turn, with `turnId`, a running `total`, and a per-request `last`. Discard keeps the totals and reports them at `turn/completed` (`codex/parse.rs`). Codex's `inputTokens` includes the cache, and `cachedInputTokens` is part of it. The Codex `Mapper` starts at zero for each app-server process. After a restart, its totals cannot be trusted.
- `EventKind::Usage` keeps only `input_tokens` and `output_tokens`. Codex token updates create no stored event. Claude's cache counts survive only in `events.raw_json`.
- Usage events are stored in SQLite (`events.kind_json`). Event ids are UUID v7, so each stored event has a time.
- Today `from_event` in `crates/discard-core/src/observer.rs` does not send usage to plugins. The test `text_usage_and_errors_are_not_sent_to_plugins` asserts this for usage. That assertion changes. Text and errors stay hidden.

Decisions:

- **One meaning for every provider.**
  - `input_tokens` is all input, cache included.
  - `cached_input_tokens` is the input read from the provider's cache. It is part of `input_tokens`, not added to it.
  - `output_tokens` is all output.

  | Provider | `input_tokens` | `cached_input_tokens` |
  | --- | --- | --- |
  | Claude | `input_tokens + cache_read_input_tokens + cache_creation_input_tokens` | `cache_read_input_tokens` |
  | Codex | `inputTokens` | `cachedInputTokens` |

- **Per-turn counts at the source.**
  - `EventKind::Usage` gets `cached_input_tokens: Option<u64>`, with a serde default, so old stored events still load.
  - The Claude parser fills the new field from `result.usage`.
  - The Codex parser sums the `last` counts of every `thread/tokenUsage/updated` notification for the turn's `turnId`, then sends that sum at `turn/completed`. A new event therefore always holds per-turn counts.
- **Old stored events.** One function in `discard-core` converts each stored usage event to a per-turn record. *(Claude's call.)* It handles three cases:
  - An event that has `cached_input_tokens` is already in the new form.
  - An old Claude event takes its cache counts from `events.raw_json`. If the raw JSON has none, the record keeps the stored count and has no `cached_input_tokens`.
  - An old Codex event holds a running total. The function takes the difference from the previous usage event in the same session. A total that drops marks a restart, and that one record is left out. The function never guesses a value.
- **The `usage` event.** Discard sends it once per finished turn: `{ time, channel_id, channel, agent_id, agent, provider, input_tokens, output_tokens, cached_input_tokens? }`. `time` is Unix seconds. No capability is needed.
- **`discard.usage.history(options, done)`.**
  - It returns past turns from the store, in the same record shape, through a callback (see [decision 9](#9-host-data-calls-take-a-callback)).
  - `days` defaults to 7, with a maximum of 90.
  - Records come oldest first, at most 10,000. When there are more, Discard keeps the newest 10,000. *(The limits are Claude's call.)*
  - It needs the new read-only `usage` capability.
- **Agent attribution in history.** A stored session links to a channel, not to an agent. History gives each turn the channel's `channels.agent_id`, unless the builder finds a direct link from session to agent. The live event uses the agent that the engine ran.
- **Manifest.** `[capabilities] usage = true`. The review screen says: "Read token counts for finished turns, with channel and agent names."

### 8. Git data

- A new read-only `git` capability: `[capabilities] git = true`. The review screen says: "Read the commit history of the active project: commit times, author names, subjects, and line counts."
- `discard.git.log(options, done)`:
  - It reads the root repository of the active project (decision 6) at HEAD. It reads no other repository. In a direct message there is no active project, so `done` gets the error `no active project`.
  - `days` defaults to 90, with a maximum of 365. `limit` defaults to 1000, with a maximum of 5000. *(Claude chose the `days` maximum.)*
  - `path` is optional and relative to the repository root. It may not start with `/` or `-`, and it may not contain `..`. Discard passes it after `--`.
  - Each commit is `{ hash, time, author, subject, additions, deletions, files }`. Commits come oldest first. When more than `limit` commits match, Discard keeps the newest. *(Claude chose oldest first, to match `history` and the order charts plot.)*
  - `time` is the author time in Unix seconds. `author` is the author's name, never the email.
  - `additions` and `deletions` count lines. Binary files count 0. `files` is the number of files changed.
  - Merge commits are excluded (`--no-merges`). *(Claude's call. Merges carry no numstat by default.)*
- The host runs the `git` CLI (`git log --numstat`), as it already does elsewhere. The call has its own 5 s timeout. A timeout, or a project with no repository, gives `done` an error.

### 9. Host data calls take a callback

- `discard.usage.history` and `discard.git.log` take a callback, `done(result, err)`. The host runs the query on a worker thread, outside the plugin thread. When the query finishes, the host queues `done` on the plugin thread, the same way it queues an event.
- The plugin thread never waits for a query. Renders, events, and timers keep running. A slow query cannot cause a timeout or a revert. The per-call budget (`runtime.rs`, `budgeted`) covers only `done` itself. That budget is wall-clock time.
- On success, `done(result, nil)` runs. On failure, `done(nil, message)` runs. Bad options, such as a `days` value outside the range or an invalid `path`, raise an error at once instead.
- A plugin may have at most 2 data calls in flight. A third call raises an error at once. *(Claude's call. It bounds the worker load.)*
- A callback that comes back after the plugin unloads is dropped.
- In `plugin_test`, these calls always call `done(nil, "not available in plugin_test")`.

### 10. Local dates: `discard.date` and `discard.days`

Fact: in Luau, `os.date` uses local time, but `os.time{...}` reads its table as UTC (`luau/VM/src/loslib.cpp`: "we prefer UTC for consistency"). Date math with these two functions is a trap. An agent that buckets commits by local day gets days wrong near midnight and when daylight saving time changes.

- `discard.date(time?)` returns the local date of a Unix time as `YYYY-MM-DD`. Without `time`, it returns today.
- `discard.days(count)` returns the last `count` local dates, oldest first, ending today. `count` is 1 to 371.
- No capability is needed. Both functions are pure, so they also work in `plugin_test`.

### 11. Motion and interaction

- Charts never animate. A data change redraws at once, so reduced motion needs nothing extra. Tooltips appear without a transition.
- `sparkline` and `heatgrid` show a tooltip on hover and on keyboard focus.
  - Each one is a single tab stop.
  - Arrow keys move from point to point and show the same tooltip. Left and right work in both; up and down also work in a grid.
- `stat`, `meter`, `bars`, and `table` print all their values. They are not interactive and are not tab stops. *(Claude's call. Bars print every value, so a tooltip on each bar adds nothing.)*
- Data points have no click actions.
- In a tooltip, the value comes first and is strong. The label follows in muted text. Example: `4,200 tokens · turn 12 of 60`.

### 12. Accessibility

- Every chart requires `label`. A chart without one fails validation.
- Rust generates a text alternative from the data. It is one summary sentence plus a table of all values. The web renders the table visually hidden. The optional `summary` prop replaces the sentence. The table stays. *(Generating in Rust is Claude's call. Then `plugin_test` and `ui_inspect` see the same text as the screen.)*
- The chart's accessible name is `label`. Its description is the summary sentence.
- `table` is a native `<table>`, with `label` as its caption. It needs no generated text.
- Identity and state never depend on color alone:
  - The `stat` delta has `▲` or `▼` and a sign.
  - The `meter` state has a word.
  - The `heatgrid` has a legend and a hidden table.
- Generated sentences follow these patterns:
  - `stat`: `Last turn: 4,200 tokens. ▲ 12% vs previous turn.`
  - `sparkline`: `Tokens per turn: 60 values, from 3,100 to 4,200. Low 900, high 9,800.`
  - `meter`: `Daily budget: 142,000 of 200,000 tokens (71%). High.`
  - `bars`: `Tokens by agent: builder 42,000; reviewer 9,100.`
  - `heatgrid` calendar: `Lines changed per day: 2026-08-10 to 2026-11-08. Total 4,210 lines. Busiest day 2026-10-02 with 610.`
  - `heatgrid` matrix: `Edits by hour: 2 rows, 24 columns. Total 310. Busiest cell Tue 14h with 22.`

### 13. Color

- Each chart draws one series and takes one `color` token. The default is `primary`.
- `color` accepts the theme color tokens (`COLOR_TOKENS`, which include `ansi_1`–`ansi_6`) and `agent`. `agent` is the identity color of the enclosing `agent` container, as in the element style design. Any other value fails validation.
- If a mark color is too close to the card surface, Discard raises it until it reaches 3:1. It uses the same contrast function as agent colors (terminal visual design, section 2).
- `success`, `warning`, and `danger` mean state. Discard uses them for `meter` thresholds and for the `stat` delta direction. Plugins may pass them as `color`, but the docs say to use them for state only.
- Text never takes the series color. Values, labels, and legends use `foreground` and `muted_foreground`. A colored mark beside the text carries the identity.
- **`heatgrid` ramp:**
  - Step 0 (zero or empty) uses `muted`.
  - Steps 1–4 use `color-mix(in oklab, <color> N%, <card surface>)`. N rises to 100% at step 4.
  - Four linear bins split the range from 1 to the max value.
  - The legend reads `less ■■■■ more`.
- **Ramp check:** *(Owner's decision. It replaces the earlier "first step reaches 3:1 against `muted`".)*
  - Lightness rises monotonically from step 0 to step 4.
  - Adjacent steps, including step 0 to step 1, differ by at least 0.06 OKLCH L.
  - Step 4 reaches 3:1 against the card surface after the contrast lift.
  - A unit test checks the ramp with Atom One Dark, with a Chalk test fixture, and with a plugin theme that does not set `ansi_*` tokens. Chalk is not a bundled theme, so the test carries Chalk's palette as a fixture.
- **Forced colors:** Under `forced-colors: active`, marks use `CanvasText`, and heat steps use opacity 0.25, 0.5, 0.75, and 1. The hidden table carries the values. *(Claude's call.)*

### 14. Number format

Rust formats every number that a chart shows, in the tree and in the text alternative. The web shows the strings that Rust sends. *(Claude's call. It gives one formatter and one test surface.)* The format is fixed: a comma groups thousands and a period marks decimals.

- `format = "number"` (default): grouped, with at most 2 decimals, for example `1,284`.
- `format = "compact"`: `12.9K` or `4.2M`.
- `format = "percent"`: the value is a ratio, so `0.46` shows as `46%`, with at most 1 decimal.
- Large standalone values (`stat`) use proportional figures. Columns of numbers (`table`, and the values in `bars`) use `tabular-nums`. *(Dataviz rule.)*
- `unit` appears in tooltips and in the text alternative, never on marks.

### 15. Limits

| Limit | Value |
| --- | --- |
| Data values in one chart | 1000 |
| Data values in one tree | 5000, counted apart from the 2000-node limit |
| A string inside data | 80 bytes |
| `label` | 1–80 characters |
| `summary` | 280 characters |
| `unit` | 16 characters |
| Numbers | finite only |
| `discard.data` per plugin | 64 KB as JSON, nesting depth 8 |
| Redraws per plugin | 4 per second |
| Panels per plugin | 4 |
| Timers per plugin | 4, at least 10 s apart |
| Data calls in flight per plugin | 2 |
| `usage.history` | 90 days, 10,000 records |
| `git.log` | 365 days, 5000 commits, 5 s |

### 16. Validation and errors

- Rust validates each chart tag against its schema. Invalid data is an error that names the tag, the prop, and the problem, for example `heatgrid.cells: row 3 has 5 values, expected 7`. Add an `ElementError` variant for this.
- Discard never clamps or drops element data to make it fit. The only change is display formatting.
- An empty array is valid: the chart shows its label and a muted `No data yet`. The same happens for a `stat` without `value` and for a `sparkline` with fewer than 2 values.
- Unknown props on chart tags are ignored, as on other tags.

### 17. `plugin_test` and `ui_inspect`

- `plugin_test` renders panels as well as slots, in this order: *(Claude's call.)*
  1. It loads the plugin.
  2. It fires `view_changed` once with sample values.
  3. It runs every queued callback. In the sandbox, `git` and `usage` calls call `done(nil, "not available in plugin_test")`. The examples therefore store their empty state or their error state.
  4. It renders every slot and panel. This is the "no data" render.
  5. If the agent passes sample `data` as JSON, the sample replaces the store, and `plugin_test` renders again.
- `discard.data`, `discard.date`, and `discard.days` work in the sandbox. Timers do not run.
- The result includes the generated text alternative of each chart, so an agent can check what a chart says without seeing it.
- `ui_inspect` shows panels, their trees, and their text alternatives.

### 18. Docs and the dogfood panel

- `api_spec.rs` stays the one source. Add every function, namespace, event payload, tag, and prop below.
- Commit the generated reference as two files:
  - `docs/plugin-api.md`: the markdown reference plus the guide;
  - `docs/plugin-api.d.luau`: the Luau types.

  A test regenerates both files and fails when the committed files differ. An agent outside Discard reads these files.
- `docs/plugins.md` links to them.
- **Dogfood.** The bundled `default-ui` plugin (`crates/discard-lua/bundled/default-ui`) gets the token-usage panel below, and its manifest adds `usage = true`. This panel is the usage example.
- The guide includes two examples:
  - the git example, as text;
  - the bundled panel's code, read with `include_str!` from the bundled file so it cannot drift. *(Claude's call. Agents inside Discard cannot read the bundled source otherwise.)*
- Tests:
  - `bundled.rs` loads `default-ui` and renders the panel.
  - A guide test loads the git example and renders its panel with empty data and with sample data, like `the_guides_example_plugin_is_a_valid_plugin`.

## Elements

Every chart tag has these shared props:

| Prop | Type | Rule |
| --- | --- | --- |
| `label` | string | Required. 1–80 characters. The title and the accessible name. |
| `summary` | string? | At most 280 characters. Replaces the generated summary sentence. |
| `color` | color token? | Default `primary`. A theme token, `ansi_1`–`ansi_6`, or `agent`. |
| `format` | `"number"` \| `"compact"` \| `"percent"`? | Default `"number"`. |
| `unit` | string? | At most 16 characters. Used only in tooltips and the text alternative. |

Layout inside a panel: a chart fills the width of the card. Each tag has a fixed height, so a redraw causes no layout jump.

### `stat`

```lua
{ "stat", label = "Last turn", value = 4200, format = "compact", unit = "tokens",
  delta = 0.12, delta_label = "vs previous turn", good = "down",
  trend = { 3100, 4200, 3900, 5100 } }
```

| Prop | Type | Rule |
| --- | --- | --- |
| `value` | number? | The headline value. If it is missing, the chart shows `No data yet`. |
| `delta` | number? | The change as a ratio: `0.12` is +12%. It shows with `▲` or `▼` and a sign. |
| `delta_label` | string? | What the delta compares to, like `vs yesterday`. At most 40 characters. |
| `good` | `"up"` \| `"down"`? | Which direction is good. Default `"up"`. The delta uses `success` when it moves the good way, `danger` when it moves the bad way, and `muted_foreground` at 0. |
| `trend` | `{ number }`? | At most 60 values, oldest first. Drawn as a small sparkline in `muted_foreground`, with the last point in `color`. |

The value uses semibold sans with proportional figures.

### `sparkline`

```lua
{ "sparkline", label = "Tokens per turn", values = { 3100, 4200, 3900 },
  format = "compact", unit = "tokens", height = "md" }
```

| Prop | Type | Rule |
| --- | --- | --- |
| `values` | `{ number }` | 0–500 values, oldest first. With fewer than 2, the chart shows `No data yet`. |
| `height` | `"sm"` \| `"md"`? | 16 px or 32 px. Default `"md"`. |
| `min` | number? | The bottom of the scale. Default: the smaller of 0 and the lowest value. |
| `max` | number? | The top of the scale. Default: the highest value. `max` must be greater than `min`. |

The line is 2 px wide, with round joins. The last point has an 8 px end dot with a 2 px surface ring. There are no axes. The tooltip shows the value and `point N of M`.

### `meter`

```lua
{ "meter", label = "Daily budget", value = 142000, max = 200000,
  warn = 150000, danger = 180000, format = "compact", unit = "tokens" }
```

| Prop | Type | Rule |
| --- | --- | --- |
| `value` | number | Required. At least 0. A value above `max` fills the bar, and the text shows the real value. |
| `max` | number | Required. Greater than 0. |
| `warn` | number? | At this value and above, the fill uses `warning` and the text adds `high`. |
| `danger` | number? | At this value and above, the fill uses `danger` and the text adds `critical`. When both are set, `warn` must be less than `danger`. |

The text reads `142K / 200K · high`. The track is the fill color mixed about 20% over the card surface. The bar is 8 px high, with fully rounded ends.

### `bars`

```lua
{ "bars", label = "Tokens by agent", format = "compact", unit = "tokens",
  items = { { name = "builder", value = 42000 }, { name = "reviewer", value = 9100 } },
  sort = "desc" }
```

| Prop | Type | Rule |
| --- | --- | --- |
| `items` | `{ { name: string, value: number } }` | 0–20 items. Each value is at least 0. |
| `sort` | `"desc"` \| `"none"`? | Default `"desc"`: largest first. `"none"` keeps the order the plugin gives. |

Each item is a row:
- The name is on the left, and the value is on the right.
- Below them is a horizontal bar, at most 8 px thick. The bar end at the baseline is square. The end at the data has a 4 px radius.
- All bars share one baseline and one scale, from 0 to the max value.

### `heatgrid`

`heatgrid` has two shapes. Each element uses one shape. Mixing props from both shapes is an error.

```lua
-- Calendar: one value per local day, from `start`. Discard lays out the weeks.
{ "heatgrid", label = "Lines changed per day", unit = "lines",
  start = "2026-08-10", values = { 0, 3, 1, 0, 7 } }

-- Matrix: rows by columns.
{ "heatgrid", label = "Edits by hour", rows = { "Mon", "Tue" }, columns = { "0h", "1h" },
  cells = { { 0, 2 }, { 1, 0 } } }
```

| Prop | Type | Rule |
| --- | --- | --- |
| `start` | string | Calendar. A local date, `YYYY-MM-DD`. |
| `values` | `{ number }` | Calendar. 0–371 values, one per day from `start`. Each value is at least 0. |
| `rows` | `{ string }` | Matrix. 1–12 row labels. |
| `columns` | `{ string }` | Matrix. 1–60 column labels. |
| `cells` | `{ { number } }` | Matrix. One array per row, with one value per column. Each value is at least 0. |

- Cells are square and at least 8 px wide, with a 2 px surface gap and a 2 px corner radius.
- **Calendar:**
  - Columns are weeks, and rows are weekdays.
  - The week starts on the first day of the locale. If the locale gives no first day, the week starts on Monday.
  - Row labels for Mon, Wed, and Fri appear in mono `xs`.
  - If the weeks do not fit the card at 8 px per cell, the grid shows the most recent weeks that fit. It prints the date range it shows under the grid. *(Claude's call.)*
  - The text alternative and the hidden table always cover all values.
- **Matrix:** Discard shows fewer column labels so that they do not overlap, for example every 6th label of 24.
- **Tooltip:** the value, then the date with its weekday (`610 lines · 2026-10-02 Fri`), or the row and column (`22 · Tue 14h`).

### `table`

```lua
{ "table", label = "Recent turns",
  columns = { "Turn", "Input", "Output" },
  rows = { { "fix tests", 3100, 420 }, { "review", 900, 210 } } }
```

| Prop | Type | Rule |
| --- | --- | --- |
| `columns` | `{ string }` | 1–8 column names. |
| `rows` | `{ { string \| number \| boolean } }` | 0–50 rows. Each row has exactly one cell per column. |

Number cells use mono `tabular-nums`, align right, and follow `format`. Booleans show as `yes` and `no`. `color` does not apply to `table`: Discard accepts it and ignores it.

## API doc strings

These are the exact strings for `api_spec.rs`. If a fact turns out to be wrong, the builder may fix it, but must keep the same precision. Existing entries that are not listed here stay unchanged, except `on` and `SLOT_STATE_DOC`.

### Functions on `discard`

| Name | Signature | Capability | Doc |
| --- | --- | --- | --- |
| `panel` | `(id: string, title: string, render: (state: any) -> Element) -> ()` | `ui` | Add a card to the context panel, below the Members card. Panels add to the window and replace nothing. `id` uses lowercase letters, digits, and `-`, and is unique within the plugin. `title` has 1 to 40 characters and is the card's heading. `render` works like a slot's render: it gets the current state and returns an element tree. At most four panels per plugin. Cards appear in plugin load order, then in registration order. Panels are hidden in direct messages. If `render` fails, Discard hides the card and reports the error. |
| `every` | `(seconds: number, run: () -> ()) -> ()` | none | Call `run` every `seconds` seconds. The minimum is 10. At most four timers per plugin. A timer runs on the plugin's thread under the normal time budget. An error in it is logged, like an error in an event handler. `plugin_test` does not run timers. |
| `date` | `(time: number?) -> string` | none | The local date of a Unix time in seconds, like `"2026-11-07"`. Without `time`, today. Use this instead of `os.time{...}`, which reads its table as UTC. |
| `days` | `(count: number) -> { string }` | none | The last `count` local dates, oldest first, ending today, like `{ "2026-11-05", "2026-11-06", "2026-11-07" }`. `count` is 1 to 371. Use it to build a `heatgrid` calendar: `start` is the first date. |
| `on` | unchanged | none | Subscribe to an event: `message_sent`, `tool_call`, `tool_result`, `approval_requested`, `agent_status`, `turn_finished`, `usage`, `view_changed`. `usage` fires once per finished turn with `{ time, channel_id, channel, agent_id, agent, provider, input_tokens, output_tokens, cached_input_tokens? }`: `time` is Unix seconds, `input_tokens` counts all input including cached input, and `cached_input_tokens` is the part read from the provider's cache. `view_changed` fires once after the plugin loads and then whenever the active project, channel, or agent changes, with `{ project, project_id, channel, channel_id, agent_id }`; `project` and `project_id` are nil in a direct message. Events are observe-only for now: a returned `{ block = true }` has no effect. |

The builder adds the payloads of the six existing events to the `on` doc in the same style.

### Namespaces

**`data`** needs no capability. *(`ApiNamespace.capability` is a `&'static str` today. The builder either makes it optional or documents `data` another way.)*

> Plugin data that render functions read as `state.data`. Values are nil, booleans, finite numbers, strings, or tables of these, nested at most 8 levels. A plugin stores at most 64 KB. Data lives in memory: a reload or restart clears it. A change redraws the plugin's slots and panels, at most four times per second. Calling `set` inside a render function is an error. Read data in event handlers, commands, actions, timers, and callbacks, store a summary with `set`, and draw it from `state.data`.

| Function | Signature | Doc |
| --- | --- | --- |
| `set` | `(key: string, value: any) -> ()` | Store `value` under `key`. `nil` removes the key. A value over the size cap is an error, and the old value stays. |
| `get` | `(key: string) -> any` | A copy of the value stored under `key`, or nil. Change the copy and `set` it again to update it. |

**`usage`** needs the `usage` capability.

> Token counts for finished turns. Discard records usage once per finished turn. `input_tokens` counts all input, including cached input; `cached_input_tokens` is the part read from the provider's cache and is absent when unknown. Discard has no context-window or cost data. A record is `{ time, channel_id, channel, agent_id, agent, provider, input_tokens, output_tokens, cached_input_tokens? }`, the same as the `usage` event. The query runs in the background and calls `done` on the plugin's thread when it finishes.

| Function | Signature | Doc |
| --- | --- | --- |
| `history` | `(options: { days: number? }?, done: (records: { UsageRecord }?, err: string?) -> ()) -> ()` | Finished turns from the last `days` days, oldest first. `days` is 1 to 90, default 7. At most 10,000 records; when there are more, the newest are kept. `done(records, nil)` on success, `done(nil, message)` on failure. Bad options are an error at once. At most two `usage` and `git` calls per plugin can run at a time; a third is an error at once. In `plugin_test`, `done` gets `nil` and an error. |

**`git`** needs the `git` capability.

> Read-only commit history of the active project's repository. The active project is the active channel's project; a direct message has none. The repository is the project's root folder at HEAD. No other git command is available. The query runs in the background and calls `done` on the plugin's thread when it finishes. A query that takes more than 5 seconds fails.

| Function | Signature | Doc |
| --- | --- | --- |
| `log` | `(options: { days: number?, limit: number?, path: string? }?, done: (commits: { Commit }?, err: string?) -> ()) -> ()` | Commits from the last `days` days, oldest first, without merge commits. `days` is 1 to 365, default 90. `limit` is 1 to 5000, default 1000; when more commits match, the newest are kept. `path` limits the log to a file or folder, relative to the repository root, without `..`, and not starting with `/` or `-`. A commit is `{ hash, time, author, subject, additions, deletions, files }`: `time` is the author time in Unix seconds, `author` is a name (never an email), `additions` and `deletions` count lines (binary files count 0), and `files` is the number of files changed. `done(commits, nil)` on success, `done(nil, message)` on failure, for example `no active project`. Bad options are an error at once. At most two `usage` and `git` calls per plugin can run at a time; a third is an error at once. In `plugin_test`, `done` gets `nil` and an error. |

### Luau types

```luau
export type UsageRecord = {
	time: number, channel_id: string, channel: string, agent_id: string, agent: string,
	provider: string, input_tokens: number, output_tokens: number, cached_input_tokens: number?,
}
export type Commit = {
	hash: string, time: number, author: string, subject: string,
	additions: number, deletions: number, files: number,
}
```

### `SLOT_STATE_DOC`

Add `data` here. `agent.id` comes from the element style design.

> `state` for slots and panels: `{ project: string, channel: string, pending_approvals: number, turn_running: boolean, agent: { id: string, name: string, provider: string, status: "idle" | "working" | "awaiting_approval" | "offline" }, data: { [string]: any } }`. `data` is this plugin's own store from `discard.data`; it is an empty table when nothing is stored.

### Chart section of the reference

A new constant. The markdown prints it before the chart tags. The Luau types print it as a comment.

> Chart elements draw data: `stat`, `sparkline`, `meter`, `bars`, `heatgrid`, and `table`. Discard draws them in the app's style, formats their numbers, and writes a text alternative from the data, so screen readers and `plugin_test` can read every chart. A chart draws one series in one color. Data props take Lua arrays, like `values = { 3, 5, 8 }`: keys 1 to n only, no holes, no string keys. Numbers must be finite. A string inside data has at most 80 bytes. A chart holds at most 1000 data values, and a tree at most 5000. Invalid data is an error that names the tag and the prop; Discard never drops or clamps data. An empty array is valid, and the chart shows `No data yet`. Every chart needs `label`. Charts do not animate. Put charts in a panel with `discard.panel`.
>
> Shared props for every chart: `label` (string, required, 1 to 80 characters: the title and accessible name), `summary` (string, at most 280 characters: replaces the generated summary sentence), `color` (color token, default `primary`), `format` (`"number"`, `"compact"`, or `"percent"`, default `"number"`; `"percent"` shows the ratio `0.46` as `46%`), `unit` (string, at most 16 characters, like `"tokens"`: used in tooltips and the text alternative).
>
> Chart colors: the theme tokens, `ansi_1` to `ansi_6`, and `agent` (the identity color of the enclosing `agent` container). Discard raises a color that is too faint against the card. `success`, `warning`, and `danger` mean state: use them for state only. Hex colors are not accepted.

### Tag props

These strings are for `tag_props`. Every chart tag also lists the shared props above.

**`stat`**

- `value` (number): The headline value. Without it the chart shows `No data yet`.
- `delta` (number): Change as a ratio: `0.12` shows as `▲ 12%`, `-0.05` as `▼ 5%`.
- `delta_label` (string): What the delta compares to, like `"vs yesterday"`. At most 40 characters.
- `good` (`"up"` | `"down"`): Which direction is good. Default `"up"`. The delta is green when it moves the good way and red when it moves the bad way.
- `trend` (`{ number }`): Up to 60 recent values, oldest first, drawn as a small line under the value.

**`sparkline`**

- `values` (`{ number }`): The points, oldest first. 0 to 500 values. Fewer than 2 shows `No data yet`.
- `height` (`"sm"` | `"md"`): 16 or 32 pixels. Default `"md"`.
- `min` (number): Bottom of the scale. Default: the smaller of 0 and the lowest value.
- `max` (number): Top of the scale. Default: the highest value. Must be greater than `min`.

**`meter`**

- `value` (number): Required. The current amount, at least 0. A value above `max` fills the bar.
- `max` (number): Required. The full amount, greater than 0.
- `warn` (number): From this value the bar is yellow and the text says `high`.
- `danger` (number): From this value the bar is red and the text says `critical`. Must be greater than `warn`.

**`bars`**

- `items` (`{ { name: string, value: number } }`): 0 to 20 items. Values are at least 0. Each item shows its name, its value, and a bar.
- `sort` (`"desc"` | `"none"`): `"desc"` (default) puts the largest first. `"none"` keeps your order.

**`heatgrid`**

- `start` (string): Calendar shape. The local date of the first value, `YYYY-MM-DD`. Get it from `discard.days`.
- `values` (`{ number }`): Calendar shape. One value per day from `start`, 0 to 371 values, each at least 0. Discard lays out the weeks.
- `rows` (`{ string }`): Matrix shape. 1 to 12 row labels.
- `columns` (`{ string }`): Matrix shape. 1 to 60 column labels.
- `cells` (`{ { number } }`): Matrix shape. One array per row with one value per column, each at least 0. Use either `start` and `values`, or `rows`, `columns`, and `cells`.

**`table`**

- `columns` (`{ string }`): 1 to 8 column names.
- `rows` (`{ { string | number | boolean } }`): 0 to 50 rows, each with one cell per column. Numbers align right and follow `format`.

### Guide additions

Add to the workflow and capability text in the guide: request `usage` to read past token counts, and `git` to read commit history. `net`, `exec`, and `approvals_decide` stay unavailable. Add these rules:

- Charts go in panels. Use `discard.panel`, not a slot, unless you mean to replace a built-in part of the window.
- Read data in event handlers, commands, actions, timers, and callbacks. Store a summary with `discard.data.set`. Draw it from `state.data`. A render function never calls `discard.fs`, `discard.memory`, `discard.usage`, `discard.git`, or `discard.data.set`.
- Store summaries, not raw records. The store holds 64 KB.
- Do the first fetch in a `view_changed` handler. It fires once after the plugin loads.
- `discard.usage.history` and `discard.git.log` answer through a callback, `done(result, err)`. Always handle `err`: it is set in `plugin_test`, in a direct message (for `git`), and when a query fails.
- Use `discard.days` and `discard.date` for local dates. `os.time{...}` reads its table as UTC.
- Test with no data first. Then pass sample `data` to `plugin_test` to check the full chart. Read the text alternatives in the result.

### Bundled token-usage panel (`default-ui`)

The `default-ui` manifest adds `usage = true` under `[capabilities]`. The guide includes this code through `include_str!`. *(The pending buffer is Claude's call. Without it, a turn that ends while history loads is lost or counted twice.)*

```lua
-- Discard records token counts once per finished turn.
local KEEP = 60
local pending = nil -- turns that arrive while history loads

local function empty()
  return { per_turn = {}, by_agent = {} }
end

local function add(usage, turn)
  local total = turn.input_tokens + turn.output_tokens
  table.insert(usage.per_turn, total)
  if #usage.per_turn > KEEP then
    table.remove(usage.per_turn, 1)
  end
  usage.by_agent[turn.agent] = (usage.by_agent[turn.agent] or 0) + total
end

-- view_changed fires once after load. Load the last 7 days once.
discard.on("view_changed", function()
  if discard.data.get("usage") or pending then
    return
  end
  pending = {}
  discard.usage.history({ days = 7 }, function(turns)
    local usage, last = empty(), 0
    for _, turn in ipairs(turns or {}) do
      add(usage, turn)
      last = turn.time
    end
    for _, turn in ipairs(pending) do
      if turn.time > last then
        add(usage, turn)
      end
    end
    pending = nil
    discard.data.set("usage", usage)
  end)
end)

discard.on("usage", function(turn)
  if pending then
    table.insert(pending, turn)
    return
  end
  local usage = discard.data.get("usage") or empty()
  add(usage, turn)
  discard.data.set("usage", usage)
end)

discard.panel("token-usage", "Token usage", function(state)
  local usage = state.data.usage or empty()
  local turns = usage.per_turn
  local last, previous = turns[#turns], turns[#turns - 1]
  local agents = {}
  for name, total in pairs(usage.by_agent) do
    table.insert(agents, { name = name, value = total })
  end
  return {
    "column", gap = 12,
    {
      "stat", label = "Last turn", value = last, format = "compact", unit = "tokens",
      delta = (last and previous and previous > 0) and (last - previous) / previous or nil,
      delta_label = "vs previous turn", good = "down",
    },
    { "sparkline", label = "Tokens per turn", values = turns, format = "compact", unit = "tokens" },
    { "bars", label = "Tokens by agent", items = agents, format = "compact", unit = "tokens" },
  }
end)
```

### Guide example: git change heat graph

`plugin.toml`:

```toml
name = "change-heat"
version = "0.1.0"
api = "1"
description = "Lines changed per day in the active project, last 13 weeks."

[capabilities]
ui = true
git = true
```

`init.lua`:

```lua
local DAYS = 91

local function refresh()
  discard.git.log({ days = DAYS, limit = 5000 }, function(commits, err)
    if err then
      discard.data.set("heat", { error = err })
      return
    end
    local dates = discard.days(DAYS)
    local index, values = {}, {}
    for i, date in ipairs(dates) do
      index[date] = i
      values[i] = 0
    end
    for _, commit in ipairs(commits) do
      local i = index[discard.date(commit.time)]
      if i then
        values[i] = values[i] + commit.additions + commit.deletions
      end
    end
    discard.data.set("heat", { start = dates[1], values = values })
  end)
end

local project
discard.on("view_changed", function(view)
  if view.project_id ~= project then
    project = view.project_id
    refresh()
  end
end)
discard.every(300, refresh)

discard.panel("change-heat", "Changes", function(state)
  local heat = state.data.heat
  if heat == nil then
    return { "text", "Loading…", size = "sm", color = "muted_foreground" }
  end
  if heat.error then
    return { "text", heat.error, size = "sm", color = "muted_foreground" }
  end
  return { "heatgrid", label = "Lines changed per day", unit = "lines", start = heat.start, values = heat.values }
end)
```

## Left to the builder

- How to store list props in `Prop` and `ElementDto`, and how the web side types them.
- How to make `ApiNamespace.capability` optional, or another way to document `data`.
- How the host schedules timers, runs data-call workers, queues callbacks, and coalesces redraws.
- How the Rust number formatter and the text alternative reach the web: as extra fields in the element data, or another way.
- The exact `color-mix()` percentages for the heat ramp, within the ramp check in decision 13.
- The tooltip component and the keyboard model inside `sparkline` and `heatgrid`, within decision 11.
- Whether a stored session can be linked to its agent directly, for usage history.
- Component structure and test layout.

## Proof

- **Rust tests:**
  - **Chart validation.** Each chart tag parses with valid data. It fails with a named error for each of these cases:
    - a missing `label`;
    - an unknown color;
    - a non-finite number;
    - an array with a hole;
    - a string key in an array;
    - too many values;
    - a string over 80 bytes;
    - a ragged `heatgrid.cells`;
    - a mix of calendar and matrix props;
    - `warn` not less than `danger`.
  - Layout tags still reject table props.
  - The data-value limits (1000 per chart, 5000 per tree) hold, and they are counted apart from the node limit.
  - The number formatter and the generated text alternatives match the patterns in decisions 12 and 14.
  - **`discard.data`:**
    - `get` returns a copy.
    - A `set` over the size cap keeps the old value.
    - `set` inside render fails.
    - Render sees `state.data` as an empty table at first.
    - One plugin cannot read another plugin's data.
  - **Redraw.** The render cache redraws after `discard.data.set`, at most 4 times per second.
  - **Timers.** `discard.every` rejects an interval under 10 s and a fifth timer. A timer error is logged and does not count toward a revert.
  - **Panels.** `discard.panel` rejects a bad id, a long title, and a fifth panel.
  - **`view_changed`.** It fires once after load and on each change. In a direct message, `project` is nil.
  - **Data calls:**
    - A slow `git.log` does not delay a render or a timer.
    - `done` runs on the plugin thread.
    - A third call in flight is an error.
    - Bad options raise an error at once.
    - A callback that comes back after the plugin unloads is dropped.
  - **Usage counts:**
    - With the Claude fixture, `input_tokens` = 2 + 7569 + 3440, and `cached_input_tokens` = 7569.
    - With the Codex fixture, the counts are the sum of `last` for the turn.
    - An old Claude event takes its cache counts from `raw_json`.
    - An old Codex total converts by difference. A drop in the total leaves that record out.
    - `usage` events reach plugins. Text and errors still do not.
  - **History and log.** `usage.history` and `git.log` keep to their limits and their order. `git.log` rejects a `path` with `..`, a leading `/`, or a leading `-`. It returns author names without emails. It reports `no active project` in a direct message.
  - **Local dates.** `discard.date` and `discard.days` give local dates across a daylight saving change.
  - **Docs:**
    - The spec lists exactly what the runtime provides, for every new function, namespace, tag, and prop.
    - The committed `docs/plugin-api.md` and `docs/plugin-api.d.luau` match the generated text.
  - **Examples:**
    - `default-ui` loads and renders the token-usage panel with no data and with sample data.
    - The guide's git example does the same.
- **Web tests** (`web/src/plugin/plugin.test.ts`):
  - Each chart renders its label, its formatted values, and its text alternative.
  - `sparkline` and `heatgrid` are tab stops, and arrow keys move the tooltip. The other charts are not tab stops.
  - The `stat` delta shows `▲` or `▼`, and uses `success` or `danger` according to `good`.
  - The `meter` text shows `high` and `critical` at the thresholds.
  - The `heatgrid` ramp passes the ramp check with Atom One Dark, the Chalk fixture, and a plugin theme without `ansi_*` tokens.
  - Plugin strings render as text, not HTML.
  - Nothing animates. No chart has a transition.
- **All checks:**

  ```sh
  cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build
  ```

- **Visual check** in `cargo tauri dev`, with the bundled panel and the git example installed. Check with Atom One Dark, Chalk, reduced motion on, and forced colors on.

## Follow-ups

- **Outside-agent tooling.** A CLI such as `discard plugin test <dir>`, and a way to submit a plugin for review from outside the app. This needs a separate design.
- **Context-window meter.** Codex sends `modelContextWindow`, and Discard drops it. A later design could keep it, and find a Claude equivalent, so that a plugin can show context use.
- **Dogfooding audit.** A separate design decides which other UI surfaces become plugins.
