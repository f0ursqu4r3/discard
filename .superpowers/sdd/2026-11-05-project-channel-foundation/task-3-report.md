# Task 3 report: project channel navigation and details

## Implementation

- Added TypeScript DTOs and bridge wrappers for `get_project_channels`, `get_project_members`, `get_channel_details`, `update_channel_topic`, `set_project_main_agent`, `select_canonical_project_channel`, and `select_canonical_branch`. The wrappers use the reviewed camelCase DTO contract and `general`, `branch`, and `archived_legacy` kinds.
- Added a project channel rail with persistent `#general`, canonical branch rows, and archived history entries. Project selection opens the returned channel ID. The rail uses the selected project snapshot, so it remains usable while a direct chat is active. Details use the active channel's exact project and channel IDs.
- Added channel details with topic editing, compare-and-swap version saves, topic history, channel identity, branch, members, and the project main agent. Configured agents are available as main-agent choices. Main-agent and canonical commands reload details after success and failure because a launcher error can follow a persisted change.
- Linked archived histories expose canonical selection only when the history has a supported branch or general identity. Other linked history stays readable and explains why it cannot become canonical. The history rail does not expose Restore.
- Project channel messages use a neutral `Agent` label and avatar. The current main agent cannot reliably identify the author of a historical reply because messages do not store durable per-message agent identity. Direct-chat attribution is unchanged. Durable attribution belongs to the routing slice.
- Membership is displayed as channel membership. The UI does not imply that membership enables mentions or group responses. Channel details state that responses use the channel's assigned agent.
- Project `New Channel` and the project navigation `New conversation` controls are visible, disabled, and labelled `Soon`. Each has a TODO beside its shared or local definition and no executable handler. Existing direct-chat flows remain available.
- Topic refreshes inspect the current draft when the response arrives. They preserve typing that starts or continues during the request. A changed topic version marks the draft stale; unrelated details refreshes with the same topic version keep it editable. A user can use the latest topic or cancel the draft.

## Verification

- Focused `ProjectChannelDetails` tests — 10 passed. This includes deferred response tests for typing during a same-version refresh and continued typing while a changed-version response is pending.
- Full frontend suite — 37 files and 214 tests passed.
- `pnpm check` — passed. This includes formatting, lint, and TypeScript checks.
- `pnpm build` — passed. The bundler reported existing `INEFFECTIVE_DYNAMIC_IMPORT` warnings for icon imports in `src/components/agents/agent-icon.ts`; they did not fail the build.
- `git diff --check` — passed.

## Formatting baseline

The unchanged baseline failed `pnpm check` on nine existing files. The approved narrow formatting cleanup covers `components/ChannelList.vue`, `components/Composer.vue`, `components/MessageRow.vue`, `components/ui/button/Button.vue`, `components/ui/message-scroller/MessageScrollerContent.vue`, `components/ui/resizable/ResizableHandle.vue`, `components/workspace/AgentsNavigation.vue`, `components/workspace/TranscriptContent.vue`, and `components/workspace/WorkspacePage.vue`. No whole-tree formatting was run.
