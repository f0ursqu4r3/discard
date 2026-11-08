# Plugin dogfooding — design

**Date:** 2026-11-07
**Status:** Approved by the owner
**Builds on:** [Terminal visual direction](2026-11-07-terminal-visual-design.md), [Lua elements match the app style](2026-11-07-lua-element-style-design.md), [Lua chart elements](2026-11-07-lua-chart-elements-design.md) (approved, committed as `9e17ecf` on `kyle/amazing-ritchie-e670f7`)

## Intent

The owner wants an easy way to extend Discard's capabilities and UI. Lua plugins are that way. When Discard draws its own UI through the plugin API, the API gets proved solid and usable. That is dogfooding.

Dogfooding starts later. This document is the reference for that work. It lists every UI surface, rates each one against a rubric, names the API that each surface needs before it can convert, and sets the conversion order and the fallback rule.

This document converts nothing and adds no API.

### What this covers now

- The only near-term build is [Follow-up 1](#follow-ups): pause `member_list` and fix the header subtitle.
- Each conversion step gets its own design when dogfooding starts. The owner approves each one. That design fixes the final names of its API before they freeze.
- The element style work and the chart elements work are not part of this design. Their own designs own them. This design only depends on them.

## Decisions

All decisions are the owner's unless marked *(Claude's call)*. The owner accepted the inventory verdicts, the conversion order, and the no-private-hooks rule after a playback.

1. **Conversion waits.** No surface converts now. Dogfooding starts at a later date, from this reference.
2. **The API grows to meet the surface.** A converted surface must give the same information and the same actions as its Vue version. When the API cannot do that yet, the conversion waits for the API. A conversion never ships with less function. Icons that repeat nearby text are decoration, not function. *(The icon rule is Claude's call.)*
3. **No private hooks.** Each API gap closes through the public plugin API. A bundled plugin gets no hook, state field, or element that a third-party plugin cannot use. *(Claude's call, accepted by the owner.)*
4. **`default-ui` stops drawing `member_list`.** The bundled plugin's Members card has no context menu and no "View profile" button, so it gives less than `MemberList.vue`. `default-ui` drops its `member_list` slot until the API has host actions and context targets (see [Members card](#members-card)). The Vue card then draws again. This document does not make the change; see [Follow-ups](#follow-ups).
5. **`channel_header` stays converted.** It is close to the Vue header. It already adds a "Needs approval" badge.
6. **The full Vue component stays as the fallback.** Every converted surface keeps its complete Vue component. The component is the fallback when the plugin fails, and it is the reference that the plugin must match. See [Fallback rule](#fallback-rule).
7. **Protected surfaces never convert.** These are the approval card, the review banner, and the plugin problem line, plus the other surfaces in [Never convert](#never-convert).

## Goals

1. A complete list of the surfaces in `web/src/App.vue` and `web/src/components/workspace/`, with the components they mount.
2. A rating for each surface against one rubric.
3. For each surface that passes, the exact API it needs: elements, props, state fields, slots, and actions. Each of these becomes frozen public API under `API_VERSION` "1".
4. A conversion order.
5. A fallback rule for converted surfaces.

## Non-goals

- Converting any surface now.
- Adding any API now. The [API growth list](#api-growth-list) is a plan, not a build.
- Changing the protected surfaces.
- Changing `API_VERSION`.
- The token-usage panel. The chart elements design owns it. This document only places it first in the order.
- Planned surfaces that are not built yet: the Merge review tab and the Artifacts tab. They are decided when they are built.

## Anti-goals

The work fails, even if every check passes, when any of these is true:

- A converted surface gives less information or fewer actions than its Vue version.
- An API addition exists only for a bundled plugin.
- An API addition is frozen before a surface uses it.
- An addition from this design lets a plugin read agent replies, message history, tool input or output, diffs, or error text. (Plugins already get the text of messages the user sends; see [Constraints](#constraints). This design does not widen that.)
- A plugin can hide, cover, or imitate a protected surface.
- A plugin failure leaves a surface blank.

## Constraints

These are facts about the current code.

- **Slot state.** Every slot gets one state table from `slot_state` in `crates/discard-core/src/ui_state.rs`: `project`, `channel`, `pending_approvals`, `turn_running`, and `agent { name, provider, status }`. The element style design adds `agent.id`. The chart elements design adds `data`.
- **Two projects.** Slot `state.project` is the active conversation's project (`state.active.project_name` in `crates/discard-app/src/desktop/core.rs`). It is empty in a direct message. The Vue status line, title bar, branch context card, and overview page use a different project: the one the user browses in the sidebar (`snapshot.project`, `projectPath`, `projectDisplayPath`). `select_sidebar_project` changes the browsed project without changing the active conversation. `channelBranch` (`web/src/components/workspace/channel-branch.ts`) also falls back to the browsed project's branch tree.
- **Redraw.** `SlotCache` in `crates/discard-app/src/desktop/slots.rs` keys on the whole state JSON. A slot redraws only when that state changes. A new state field therefore redraws every slot when it changes.
- **Slots today.** `render_slots` draws only `channel_header` and `member_list`. The header slot draws only in a project conversation. The direct-message header stays Vue. The header slot replaces only the title area. The context-panel toggle and the history search stay Vue outside the slot.
- **Fallback chain.** `PluginManager::render_slot` (`crates/discard-lua/src/manager.rs`) tries the last plugin that registered the slot, then each earlier one. When all fail, it returns `None` and the Vue component draws. User plugins load after `default-ui`, so a user plugin falls back to `default-ui` first.
- **Failures.** A render error goes to the protected plugin problem line. `PluginService::maintain` rolls back an installed plugin after `REVERT_THRESHOLD` failures in a row, or uninstalls it when it has no earlier version. `default-ui` is loaded straight into the manager (`crates/discard-app/src/main.rs`) and is not in the plugin repo. Rollback does not apply to it, so when it fails, the Vue component draws.
- **Tree size.** An element tree has at most 2000 nodes.
- **API version.** `API_VERSION` is "1", and manifests must match it exactly. Every element, prop, state field, slot, and action that a plugin can use is public API and cannot change later. New API must be additive.
- **Plugin actions.** An element's `on_click` calls back into the same plugin (`pluginAction`). `discard.menu_item` adds a labeled item to the host context menu for a target type, such as `agent`; choosing it calls back into the plugin (`append_plugin_items` in `crates/discard-core/src/action.rs`). A plugin cannot open a profile, select a channel, or open a host context menu from its own elements.
- **Privacy.** The `message_sent` event gives every subscribed plugin the text of each message the user sends, with no capability (`message_sent` in `crates/discard-core/src/observer.rs`). Plugins do not get agent replies, tool input or output, usage, or errors (`from_event`, tests `a_tool_call_carries_the_tool_name_and_id_but_not_the_input` and `text_usage_and_errors_are_not_sent_to_plugins`). Plugins have no `net` or `exec`.
- **Protected surfaces.** These carry `data-protected="true"` and render outside plugin control (`web/src/components/Notices.vue`, `ApprovalCard.vue`, `protected.test.ts`).
- Follow `AGENTS.md`.

## Rubric

Rate each surface with five questions.

| # | Question | Pass when |
| --- | --- | --- |
| 1 | Does it only display data? | It only displays data, or its only controls are host navigation actions from a fixed list. |
| 2 | Is its state small and safe to expose? | The state is a few scalars or short lists, and it has no message text, tool output, diffs, errors, or secrets. |
| 3 | Does it need text input, drag, virtualization, streaming, or context menus? | It needs none of these. A host-owned context menu on a target is allowed once the API has it. |
| 4 | What does a plugin failure cost? | Low: the Vue fallback draws, and the user loses no work or safety control. |
| 5 | Which API becomes frozen? | The new API is general. A third-party plugin could use it for another purpose. |

Failure cost levels:

- **Low.** The region looks different for a moment. The Vue fallback draws it.
- **Medium.** Navigation or a workflow step is degraded.
- **High.** The user can lose work, a recovery path, or a safety control.

A surface **passes** when it passes all five questions, possibly after the API grows. It **fails** when any answer is a hard no: text input, streaming, sensitive data, or high failure cost.

## Inventory and ratings

Abbreviations in the table: **Q1** displays data only, **Q2** state small and safe, **Q3** needs no input, drag, virtualization, streaming, or plugin menus, **Q4** failure cost.

### Main window

| Surface | Components | Q1 | Q2 | Q3 | Q4 | Verdict |
| --- | --- | --- | --- | --- | --- | --- |
| Title bar | `App.vue` header | Mostly | Yes | Yes | Low | Keep Vue. It holds the UI inspector toggle, a developer control, and little else. |
| Server rail | `ServerRail.vue`, `ProjectContextMenu.vue` | No | Yes | No: context menus | Medium | Fail. |
| Navigation panel | `NavigationPanel.vue` | — | — | — | — | Container only. Not a surface. |
| Channel list and branch tree | `ChannelList.vue`, `SidebarWorktree.vue`, `NavigationContextMenu.vue` | No | No: all channels, branches, repositories | No: collapsible groups, dropdown menus, context menus, selection | Medium | Fail. |
| Direct-message sidebar and agents navigation | `DirectMessageSidebar.vue`, `AgentsNavigation.vue` | No: opens chats | No: all agents and conversations | Yes | Medium | Deferred. See [Deferred](#deferred). |
| User bar | `UserBar.vue` | No | Yes | Yes | Low | Keep Vue. One settings button. Nothing to learn. |
| Channel header | `ChannelHeader.vue`, slot `channel_header` | Yes | Yes | Yes | Low | **Converted.** Keep. |
| Conversation tabs and activity line | `App.vue` | No: tab control | Yes | Yes | Low | Keep Vue. The tabs are a control. The activity line is too small to matter. |
| Transcript | `MessageList.vue`, `TranscriptContent.vue`, `MessageRow.vue`, `BlockView.vue`, `ToolCallBlock.vue`, `WorktreeApprovals.vue` | No | No: full message history, agent replies, tool output | No: streaming, scroll restore, jump to message, context menus, quote | High | Fail. |
| Changes tab | `DiffBlock.vue` list in `App.vue` | Yes | No: diffs are large and can hold secrets | Yes | Low | Fail on Q2. |
| Run tab | `ToolCallBlock.vue` list in `App.vue` | Yes | No: tool input and output | No: live tool state | Low | Fail on Q2. |
| Merge review tab | `App.vue` | — | — | — | — | Planned (`Soon`). Out of scope. |
| Recovery actions | `App.vue`: copy unsaved reply, retry, choose model, restore conversation, read-only line | No | Yes | Yes | High | Keep Vue. They appear when something failed, which is the worst time for a plugin failure. |
| Thinking line | `App.vue`, above the composer | Yes | Yes | Yes | Low | Passes, but not planned. Too small to test the API. |
| Composer | `Composer.vue` | No | No: drafts | No: text input, attachment drag and drop, mentions | High | Fail. |
| Status line | `StatusLine.vue` | Yes | Yes | Yes | Low | **Passes** after the API grows. |
| Loading state | `App.vue` "Opening workspace…" | Yes | Yes | Yes | Low | Keep Vue. It draws before plugins load. |
| UI inspector | `UiInspector.vue` | No | Yes | Yes | Medium | Keep Vue. It must show plugin trees truthfully, so a plugin must not draw it. |
| Toasts | `Toaster` in `App.vue` | Yes | Yes | Yes | Medium | Keep Vue. Host messages must reach the user even when plugins fail. |

### Context panel

| Surface | Components | Q1 | Q2 | Q3 | Q4 | Verdict |
| --- | --- | --- | --- | --- | --- | --- |
| Panel shell and tabs | `ContextPanel.vue` | No: tab control | Yes | Yes | Medium | Keep Vue. |
| Channel details card | `ProjectChannelDetails.vue` | No | Yes | No: topic text input, "make canonical" | Medium | Fail. |
| Branch context card | `ContextPanel.vue` `branch-context` | Yes | Yes | Yes | Low | **Passes** after the API grows. |
| Members card | `MemberList.vue`, slot `member_list` | No: "View profile" | Yes | Needs a host context menu | Low | **Passes** after the API grows. The slot pauses now (decision 4). |
| Review queue card | `ContextPanel.vue` `review-queue` | No | Yes | Yes | High | **Never convert.** A plugin could hide the plugin-review button. *(Claude's call.)* |
| Plugin panels | `discard.panel` (chart elements design) | Yes | Yes | Yes | Low | New and additive. The token-usage panel is first. |
| Memory tab | `MemoryBrowser.vue` | No | No: memory content needs the `memory` capability | No: editing | Medium | Fail. |
| Artifacts tab | `ContextPanel.vue` | — | — | — | — | Planned (`Soon`). Out of scope. |
| Direct-message profile | `AgentSummary.vue` | No: profile button | Yes | Yes | Low | Deferred. See [Deferred](#deferred). |
| History search | `AgentHistorySearch.vue` | No | No: message text | No: text input | Medium | Fail. |

### Workspace pages (`WorkspacePage.vue`)

| Surface | Components | Q1 | Q2 | Q3 | Q4 | Verdict |
| --- | --- | --- | --- | --- | --- | --- |
| Project overview | `WorkspacePage.vue` `overview` | Mostly: one "current work" button | Yes | Yes | Low | **Passes** after the API grows. |
| Project agents page | `WorkspacePage.vue` `agents` | No: profile links | No: all project agents | Yes | Low | Deferred. See [Deferred](#deferred). |
| Project settings | `WorkspacePage.vue` `project-settings`, `ProjectIdentitySettings.vue`, remove-project dialog | No | Yes | No: forms | High: destructive actions | Fail. |
| Project memory page | `MemoryBrowser.vue` | No | No | No | Medium | Fail. |
| Artifacts page | `WorkspacePage.vue` empty state | — | — | — | — | Planned. Out of scope. |
| Direct-message home | `FriendsList.vue` | No | No: all agents | Yes | Medium | Deferred. See [Deferred](#deferred). |

### Other windows and dialogs

| Surface | Components | Verdict |
| --- | --- | --- |
| Settings window | `web/src/settings/SettingsApp.vue` and `pages/*` | Fail: forms. The Plugins page controls plugin review and trust, so it must never be plugin-drawn. |
| Agent dialogs | `AddAgentDialog.vue`, `AgentProfileDialog.vue` | Fail: forms. |
| Create channel dialog | `CreateChannelDialog.vue` | Fail: form. Only tests mount it today. |

### Never convert

- Approval card (`ApprovalCard.vue`).
- Worktree approvals (`WorktreeApprovals.vue`). They decide on agent requests.
- Confirm banner and archive banner (`Notices.vue`).
- Plugin review banner (`Notices.vue`).
- Plugin problem line (`Notices.vue`).
- Review queue card (`ContextPanel.vue`). *(Claude's call.)*
- Settings Plugins page.

## Surfaces that pass

Each section lists what the conversion needs. Names in `code` are proposals. The conversion's own design fixes the final names, because they become frozen.

The element style design must land first. It gives `agent` containers, `variant = "label"` and `variant = "mono"` on `text`, `spinner`, and `agent.id`. The sections below assume it.

**Which project.** Each converted surface uses the same project as its Vue version (decision 2). The status line, the branch context card's repository row, and the overview use the browsed sidebar project, not `state.project`. So state gains a `browsed` table for the browsed project: `{ project, project_display_path, branch }`. `state.project` keeps its meaning. *(Claude's call, from decision 2.)* The branch context card mixes the two: its channel rows come from the active conversation, and its repository row comes from the browsed project. The card's conversion design checks whether that is intended before it freezes the fields.

### Channel header

Already converted. No new API.

Parity gap: `ChannelHeader.vue` shows the project name as a subtitle. `default-ui` does not. `state.project` already exists, so `default-ui` can draw it. Fix this when `default-ui` is next changed (see [Follow-ups](#follow-ups)). *(Claude's call.)*

### Status line

Vue: `StatusLine.vue`. It shows the browsed project's name on a primary block, the branch (`⎇ name`), one spinner and name for each agent with a running turn in any channel, and `▲ N approvals` at the end in warning color. It draws in direct messages too.

| Need | Kind | Frozen API |
| --- | --- | --- |
| `status_line` | slot | The slot name. It replaces the whole footer. |
| `browsed.project` | state | `string`. The browsed sidebar project's name. Set in direct messages too. |
| `browsed.branch` | state | `string?`. The value of `channelBranch`. Nil when there is none. |
| `working_agents` | state | `{ { id: string, name: string } }`. Agents with a running turn in any channel. The `agent` prop on `row` gives each one its identity color, so state needs no avatar. |

The elements exist: `row` with `bg = "primary"`, `primary_foreground`, `spacer`, `spinner`, and `agent` containers.

Layout: the footer is 24 px high. The host clips overflow. *(Claude's call.)*

Why first among conversions: it adds one slot and a few state fields, and no elements.

### Branch context card

Vue: the `branch-context` card in `ContextPanel.vue`. Its header row has a branch icon, the branch name (or the channel name) in `text-sm`, and an `Active` or `Archived` badge. Its body has key-value rows for `channel`, `branch`, `repository`, and `status`. The repository row shows the browsed project's name, and the absolute project path on hover.

| Need | Kind | Frozen API |
| --- | --- | --- |
| `branch_context` | slot | The slot name. It replaces the card. A panel does not fit, because a panel adds and replaces nothing. |
| `browsed.branch`, `browsed.project` | state | Shared with the status line. |
| `browsed.project_display_path` | state | `string`. The path with the home folder shown as `~`, as the overview page shows it. The Vue hover shows the absolute path. The display path counts as the same information, because the home folder is the user's own, and it keeps the user name out of plugin state. *(Claude's call.)* |
| `archived` | state | `boolean`. |
| `card` | element | A card container with a header row and a body. Each holds any children, so the branch card puts its name and badge in the header, and the Members card puts a `variant = "label"` heading there. |
| `kv` | element | One key-value row: mono key, dotted leader, value (the `kv-leader` utility). |
| `badge` with text | prop | Today `badge` shows only `count`. Add a text label, like `Archived`. |
| `tooltip` | prop | A plain-text hover label on `text`. It carries the path. |

### Members card

Vue: `MemberList.vue`. It shows a `Members` section label, an avatar, the name in identity color, `status · provider`, and a spinner or presence dot. The row has the host context menu for the agent (`ContextMenuHost`). The footer has a "View profile" button. During a turn it shows the responder, which can differ from the channel agent.

| Need | Kind | Frozen API |
| --- | --- | --- |
| `member_list` | slot | Exists. `default-ui` registers it again. |
| Host actions | prop | A prop such as `action = "open_profile"` with an `agent` id, on `button` and `row`. The host runs it. It is a fixed list of navigation actions. A plugin cannot start engine actions: send, stop, approve, archive, delete, or change settings. The first action is `open_profile`. Each action name and its arguments are frozen. |
| Context targets | prop | A prop such as `context = "agent"` with an `agent` id, on `row` and `column`. A right-click opens the host's own context menu for that target: the host items, plus the items that plugins added with `discard.menu_item` for that target type. The drawing plugin cannot change or remove host items. The first target type is `agent`. |
| `card` | element | Shared with the branch context card. A footer area holds the button. |
| `responder` | state | `{ id, name, provider }?`. The agent that answers the running turn, when it differs from the channel agent. A new field, because changing what `agent` means would break existing plugins. *(Claude's call.)* |

When this lands, the `member_list` code from the element style design is the starting point. Today that code is uncommitted. Commit the element style work before the pause lands, so that git history keeps it. *(Claude's call.)*

### Project overview

Vue: the `overview` page in `WorkspacePage.vue`. It shows the project avatar, name, and display path, four counts (branches, conversations, agents, current messages), and a "current work" card with the channel name, `Running` or `Idle`, and a button that opens the conversation.

| Need | Kind | Frozen API |
| --- | --- | --- |
| `overview` | slot | A page slot. It replaces the page body. Page slots are a new idea, so the conversion's design decides how they work. |
| `stat` | element | From the chart elements design. |
| Counts | data source | The four counts. Prefer one general read-only source, which other plugins can also use, over four one-off state fields. The conversion's design decides. |
| `open_conversation` | host action | Added to the host action list. |
| Project avatar | element | The `ProjectAvatar.vue` look, or the `avatar` element with a project mode. |

This is the largest step. It waits until the chart elements and host actions have been in use.

## Deferred

These surfaces could pass, but they need large lists in state, selection state, and several navigation actions. Reconsider them after the Members card proves host actions.

- Direct-message sidebar and agents navigation.
- Direct-message home (`FriendsList.vue`).
- Direct-message profile (`AgentSummary.vue`). It needs the same API as the Members card. Panels are hidden in direct messages.
- Project agents page.

## Conversion order

Each step starts only when its entry rules are met.

1. **Token-usage panel.** Owned by the chart elements design. It is new, so it is not a conversion. It proves panels, `discard.data`, charts, and usage data.
2. **Status line.**
3. **Branch context card.**
4. **Members card.** `default-ui` draws `member_list` again.
5. **Project overview.**

The order goes from the least new frozen API to the most. Each step reuses what the step before it added.

Entry rules for each step:

- Each new element, prop, state field, slot, and action has exact docs in `crates/discard-lua/src/api_spec.rs` and is covered by the `api_spec.rs` tests.
- Each addition is additive under `API_VERSION` "1".
- Each addition is usable by a third-party plugin. The guide or a test shows a use that is not the bundled surface. *(Claude's call.)*
- Rust validates each new element and prop in `crates/discard-lua/src/element.rs`. The web renderer filters props in `web/src/plugin/props.ts`.
- The plugin version gives the same information and actions as the Vue component (decision 2).

## Fallback rule

1. Every converted surface keeps its complete Vue component (decision 6).
2. The fallback chain does not change: the last plugin that registered the slot, then earlier plugins, then `default-ui`, then the Vue component.
3. When a plugin fails, the protected plugin problem line reports it, and the next plugin in the chain or the Vue component draws. A surface is never blank.
4. Every visual or behavior change to a converted surface lands in the Vue component and in `default-ui` in the same change.
5. Each converted slot has a test that renders `default-ui` and the Vue component with the same state and checks that both show the same information and actions. The builder chooses the method. *(Claude's call.)*

The cost is two versions of each converted surface. The owner accepts that cost, because the Vue version is the reference for the conversion.

## API growth list

Every addition that the passing surfaces need, in order of first use.

| Addition | Kind | First needed by | Also used by |
| --- | --- | --- | --- |
| `status_line` | slot | Status line | — |
| `browsed.project` | state | Status line | Branch context card, project overview |
| `browsed.branch` | state | Status line | Branch context card |
| `working_agents` | state | Status line | — |
| `branch_context` | slot | Branch context card | — |
| `archived` | state | Branch context card | — |
| `browsed.project_display_path` | state | Branch context card | Project overview |
| `card` | element | Branch context card | Members card |
| `kv` | element | Branch context card | Any plugin card |
| `badge` text label | prop | Branch context card | Any plugin |
| `tooltip` | prop | Branch context card | Any plugin |
| Host actions (`open_profile`) | prop | Members card | Project overview, deferred surfaces |
| Context targets (`agent`) | prop | Members card | Deferred surfaces |
| `responder` | state | Members card | — |
| `overview` page slot | slot | Project overview | — |
| Count data source | function or state | Project overview | Any plugin |
| `open_conversation` | host action | Project overview | — |
| Project avatar | element | Project overview | — |

Dependencies from other designs:

- Element style design: `agent` containers, `agent` color token, `text` variants, `spinner`, `agent.id`.
- Chart elements design: `discard.panel`, `discard.data`, `state.data`, `stat`, the `usage` capability.

## Work in progress elsewhere

- **Element style design.** Approved. The work is uncommitted in the main checkout, on `codex/project-memory`. Its section 4 restyles `member_list` in `default-ui`. Decision 4 removes that slot, so that part of the work is not used until the Members card converts. Its elements, the `channel_header` change, and their tests stay useful. The `bundled.rs` tests that render `member_list` change with decision 4.
- **Chart elements design.** Approved, committed as `9e17ecf` on `kyle/amazing-ritchie-e670f7`. It owns the token-usage panel in `default-ui`, and it lists this audit as a follow-up ("Dogfooding audit").

## Left to the builder

- The method of the parity test in the fallback rule.

## Follow-ups

1. **Pause `member_list` in `default-ui`.** Land it after the element style work is committed. Remove the slot so that `MemberList.vue` draws. Add the project subtitle to `channel_header`. Update the `bundled.rs` tests. This is a small change.
2. **One design per conversion step**, when dogfooding starts. Each one fixes the final names of its API.
