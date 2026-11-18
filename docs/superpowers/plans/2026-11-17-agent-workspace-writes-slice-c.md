# Agent Workspace Writes, Slice C — Implementation Plan

> **For agentic workers:** A Sonnet subagent builds this plan task by task. The executor does not commit. The reviewer reads the full diff and runs the proof command. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The project's Primary / Moderator agent can ask to move a conversation to an existing channel or to a new branch. The user approves on a card. Discard then posts a handoff and the user's message in the target and starts a response there.

**Architecture:** Slice B gave Discard app-owned write requests: an MCP tool creates a pending `WriteRequest`, a protected card shows it, and `Channels::decide_write_request` runs the approved action. Slice C adds a third request kind, `ConversationMove`, on the same store, commands, and card. The request carries a resolved target and a handoff. Approval reuses the user path for branch channels and worktrees (`GitWorkspace::create_channel_with_worktree`), posts two messages in the target, sends the user's message to the target engine as a normal `Send`, and posts a link block in both channels. No engine change and no schema change are needed.

**Tech Stack:** Rust (tokio, rusqlite, serde), Vue 3 with shadcn-vue, Vitest (`vp test`).

**Spec:** `docs/superpowers/specs/2026-11-09-agent-workspace-writes-design.md`, decisions 20–23 and "Delivery". Slice B is the pattern: `docs/superpowers/plans/2026-11-09-agent-workspace-writes-slice-b.md`, Tasks 6 and 7.

## Global Constraints

- Only the user approves a move. The approval is never an MCP tool, a rule, or a plugin decision.
- Only the project's main agent (`Store::project_main_agent`) can request a move. Discard checks this when it lists the tool, when the tool is called, and again at approval.
- A move request expires after 10 minutes, is single-use, and is cancelled by an app restart. This is the existing `TTL` in `write_requests.rs`.
- The source transcript is never copied. The target gets the handoff and one user message only.
- The user's view never changes channel because of a move.
- An existing target channel keeps its topic. A new channel gets the handoff as its topic.
- No schema migration. No new crate or npm dependencies.
- Web tests go in a `__tests__` directory beside the code they test (`AGENTS.md`).
- User-facing text uses short, direct sentences and active voice. Use the exact strings below.
- Do not commit. Do not add Claude attribution anywhere.
- Proof command (run from the repo root):
  `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build`

## Out of scope

- **"Create a branch here" (decision 17).** It needs a different gate (any write-capable agent in a detached worktree channel, not the moderator) and a rebind of the worktree channel to the new branch. `docs/project-channel-status.md` keeps it as planned.
- **An existing branch that has no channel as a target.** The agent names an existing channel or a new branch. A `new_branch` name that already exists is refused with the existing message "A branch named `x` already exists."
- **Choosing the repository for a new branch.** A new branch is created in the repository of the source channel. For `#general` and text channels, this is the project root repository.
- Native audit rows in `approvals`. Slice B write requests do not have them. Slice C follows slice B: a transcript note records the outcome.

## Review Focus

1. **A session that is not the moderator calls the tool by name.** The tool is not listed for it, but it can still send `tools/call`. The call is refused with the gate message, and no request is created. The same applies to a thread session and to a summary session. Task 3 has the tests.
2. **State changes between request and approval.** The main agent changes, the target channel is archived, the branch name is taken, the base commit is gone, or the source message is deleted. Each case fails with a clear message. No branch, worktree, channel, or target message remains. Task 4 has the tests.
3. **A busy target.** A turn runs in the target, or an approval waits there. Approval is refused before the request is consumed, so the request stays pending and the user can try again. Task 4 has the test.
4. **Approving twice, after expiry, or from another channel.** Each case is refused. The target gets no second handoff and no second response. Tasks 1 and 4 have the tests.
5. **Handoff text is model output.** It appears on a protected card and becomes a channel topic. Oversize text is refused at request time (the topic limit is 4096 bytes). The card renders it as plain text, never as HTML. Tasks 1 and 5 have the tests.

---

### Task 1: Core — move request data and the link block

**Files:**
- Modify: `crates/discard-core/src/write_requests.rs`
- Modify: `crates/discard-core/src/model.rs` (`Block::ChannelLink`, `LinkDirection`)
- Modify: `crates/discard-core/src/context_menu.rs:42` (`message_text` returns `None` for the new block)

**Interfaces:**
- Produces:
  - `WriteRequestKind::ConversationMove` (serializes as `"conversation_move"`).
  - `#[serde(rename_all = "camelCase")] pub struct Handoff { pub goal: String, pub constraints: Vec<String>, pub decisions: Vec<String>, pub open_questions: Vec<String> }`
  - `Handoff::from_args(args: &serde_json::Map<String, serde_json::Value>) -> Result<Handoff, String>` reads `goal`, `constraints`, `decisions`, and `open_questions`. It trims each string.
  - `Handoff::markdown(&self) -> String`
  - `#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")] pub enum MoveTarget { Channel { channel_id: String, name: String }, NewBranch { repository: String, repo_path: String, branch: String, base: String, base_commit: String } }`. `repository` is the repository ID of the project channel metadata (empty for the project root). `repo_path` is its folder.
  - `#[serde(rename_all = "camelCase")] pub struct MoveDetails { pub target: MoveTarget, pub handoff: Handoff, pub source_message_id: String }`
  - `WriteRequest` gains `#[serde(rename = "move", skip_serializing_if = "Option::is_none")] pub details: Option<MoveDetails>`. The two existing kinds set `None`.
  - `WriteRequests::request_move(&self, channel: &ChannelId, agent: &AgentId, details: MoveDetails) -> Result<WriteRequest, String>`. `branch` is the new branch name or empty. `path` is empty.
  - `#[serde(rename_all = "snake_case")] pub enum LinkDirection { MovedTo, MovedFrom }`
  - `Block::ChannelLink { direction: LinkDirection, channel_id: String, channel_name: String }` (serializes with `"type":"channel_link"`).

- [ ] **Step 1: Write failing tests in `write_requests.rs`.**
  - `move_request_carries_target_and_handoff`: the JSON of the request has `kind == "conversation_move"`, `move.target.kind == "channel"`, `move.handoff.openQuestions`, and `move.sourceMessageId`.
  - `a_second_move_request_is_refused_while_one_is_pending`: the error is "A move request is already pending in the app. The user must decide it first." After `begin(.., false)`, a new request is accepted.
  - `a_denied_move_has_the_denial_text`: `result == Some("Conversation move denied.")`.
  - `handoff_needs_a_goal_and_bounded_text`: these are refused, each with the given error:
    - a missing or blank `goal`: "`goal` is required."
    - a `goal` over 600 characters: "Keep `goal` under 600 characters."
    - a list that is not an array of strings: "`constraints` must be a list of strings." (same form for `decisions` and `open_questions`)
    - a list with more than 12 items: "Give at most 12 items in `constraints`."
    - an item over 400 characters, or blank: "Each item in `constraints` needs 1 to 400 characters."
    - markdown over 4096 bytes: "The handoff is too long. Keep it under 4096 bytes."
  - `handoff_markdown_omits_empty_sections`:

```rust
let handoff = Handoff { goal: "Ship the importer".into(), constraints: vec![], decisions: vec!["Use CSV".into()], open_questions: vec!["Which delimiter?".into()] };
assert_eq!(handoff.markdown(), "**Goal:** Ship the importer\n\n**Decisions**\n- Use CSV\n\n**Open questions**\n- Which delimiter?");
```

  Test in `model.rs`: `channel_link_block_round_trips`: `{"type":"channel_link","direction":"moved_to","channel_id":"c","channel_name":"feature"}` parses and serializes back to the same JSON.
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-core write_requests model::`. Expected: compile errors.
- [ ] **Step 3: Implement.** Section order in `markdown` is Goal, Constraints, Decisions, Open questions. `request_move` shares the expiry, the limit of 100 requests, and the insert code with `request`. It differs in one way: a pending or running move in the same channel is an error, not a returned duplicate. `begin` gets the denial text for the new kind.
- [ ] **Step 4: Run the tests.** Command: `cargo test -p discard-core`. Expected: PASS.

---

### Task 2: Core — the moderator gate and target resolution

**Files:**
- Modify: `crates/discard-core/src/workspace.rs`
- Modify: `crates/discard-core/src/branch_actions.rs`

**Interfaces:**
- Consumes: `MoveTarget` (Task 1), `branch_actions::validate_branch_name`, `branch_actions::default_branch`, `worktree::list_branches`.
- Produces:
  - `pub struct MoveTargetArgs<'a> { pub channel: Option<&'a str>, pub new_branch: Option<&'a str>, pub base: Option<&'a str> }`
  - `GitWorkspace::move_source(&self, channel: &ChannelId, agent: &AgentId, project: Option<&ProjectId>) -> Result<(), String>`. It is the gate. It does not look at messages, so the tool list and the prompt do not depend on a saved user message.
  - `GitWorkspace::move_request_target(&self, channel: &ChannelId, agent: &AgentId, project: Option<&ProjectId>, args: &MoveTargetArgs) -> Result<(MoveTarget, MessageId), String>`. It calls `move_source` first. The message ID is `Store::last_user_message_id` of the channel.
  - `branch_actions::branch_commit(repo: &Path, branch: &str) -> Result<String>` returns the full commit ID of `refs/heads/<branch>`.
  - `branch_actions::create_branch_at(repo: &Path, name: &str, commit: &str) -> Result<()>` runs `git branch -- <name> <commit>`. It does not switch any working tree. The caller validates the name first.

- [ ] **Step 1: Write failing tests.** Use a temporary repository with `main` and one commit, as `creation_rig` in `crates/discard-mcp/src/tools.rs` does.
  - `workspace.rs`:
    - `only_the_project_main_agent_passes_the_move_gate`: a second agent gets "Only this project's Primary / Moderator agent can move a conversation. Suggest the move in your reply."
    - `move_gate_refuses_threads_archived_channels_and_other_projects`. The errors are:
      - a thread channel: "A conversation cannot move from a thread."
      - an archived channel or agent: "This agent or channel is archived."
      - no project, or another project: "Conversation moves require this conversation's project."
    - `a_move_request_needs_a_user_message`: `move_source` passes, and `move_request_target` gives "This channel has no user message to move."
    - `channel_target_resolves_by_name`: `#Feature`, `feature`, and ` FEATURE ` all resolve to the channel named `feature`.
    - `channel_target_refuses_the_source_unknown_and_archived_names`. The errors are:
      - the source itself: "The conversation is already in #general."
      - an unknown or archived name: "No active channel is named #nope. Active channels: #docs, #feature." (names sorted, the source left out, at most 30)
      - two active channels with the same name: "More than one channel is named #x. Ask the user which one."
    - `target_arguments_must_name_exactly_one_target`: both or neither give "Give exactly one of `channel` or `new_branch`." `base` with `channel` gives "`base` applies only to `new_branch`."
    - `new_branch_target_uses_the_default_branch_and_records_its_commit`: `base == "main"`, and `base_commit` equals `git rev-parse refs/heads/main`.
    - `new_branch_target_accepts_a_named_base_and_refuses_a_missing_one`: "Base branch `nope` does not exist."
    - `new_branch_target_refuses_an_existing_or_invalid_name`: the `validate_branch_name` messages pass through.
    - `new_branch_target_needs_a_default_branch`: with a detached main checkout and no `base`, the error is "The main checkout has a detached HEAD. Name a `base` branch."
  - `branch_actions.rs`:
    - `create_branch_at_starts_a_branch_without_switching`: the new branch points at the commit, and `HEAD` of the main checkout is unchanged.
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-core workspace branch_actions`. Expected: compile errors.
- [ ] **Step 3: Implement.** Resolve saved facts under the store lock, then run Git outside it, as `write_request_target` does.
  - Target channels are the active channels of the project with kind `General`, `Text`, `Branch`, or `Legacy`. Threads are never targets.
  - The repository of a new branch is the source channel's `ProjectChannelMetadata::repository`. When it is empty, it is the project folder. When that folder is not a Git repository, the error is "This channel's repository is not a Git repository."
  - The move gate does not require `workspace_write`. A moderator that cannot edit files can still move a conversation.
- [ ] **Step 4: Run the tests.** Command: `cargo test -p discard-core`. Expected: PASS.

---

### Task 3: MCP tool, prompt line, and provider allowlists

**Files:**
- Modify: `crates/discard-mcp/src/tools.rs`
- Modify: `crates/discard-mcp/src/service.rs` (`prepare_scoped`)
- Modify: `crates/discard-providers/src/conversation.rs` (`MEMORY_TOOLS`)
- Modify: `crates/discard-providers/src/claude/provider.rs:736` and `crates/discard-providers/tests/codex_process.rs:350` (the expected tool lists)
- Test: `crates/discard-mcp/src/tools.rs`, `crates/discard-mcp/tests/worktree_access.rs`

**Interfaces:**
- Consumes: `GitWorkspace::move_source`, `GitWorkspace::move_request_target`, `MoveTargetArgs` (Task 2); `Handoff::from_args`, `MoveDetails`, `WriteRequests::request_move` (Task 1).
- Produces: the MCP tool `conversation_request_move`. `MEMORY_TOOLS` gains the name after `main_checkout_request_write`.

- [ ] **Step 1: Write failing tests.**
  - `tools.rs` (build on `creation_rig`; add a second agent, make the first one the project main agent with `Store::set_project_main_agent`, and save one user message in the channel):
    - `move_tool_is_listed_only_for_the_project_main_agent`: listed for the main agent. Not listed for the second agent, with `can_write == false`, with `project == None`, or after the main agent changes.
    - `move_tool_refuses_a_caller_that_is_not_the_moderator`: `tools/call` by the second agent returns `isError` with the gate message. `requests.list(channel)` is empty.
    - `move_tool_creates_one_pending_request_with_the_handoff`: the reply has `approvalRequired: true`, `request.kind == "conversation_move"`, and the message below. A second call returns the "already pending" error.
    - `move_tool_refuses_bad_arguments`: these are tool errors, and no request is created: `{"approve":true}` as an extra key, both `channel` and `new_branch`, a missing `goal`, and a `target` that is a string.
  - `worktree_access.rs`:
    - `moderator_prompt_names_the_move_tool`: the system prompt of the main agent in a project channel contains the prompt line below. The prompt of another agent, and of the main agent in a direct chat, does not contain `conversation_request_move`.
- [ ] **Step 2: Run them to see them fail.** Commands: `cargo test -p discard-mcp` and `cargo test -p discard-providers conversation`. Expected: FAIL.
- [ ] **Step 3: Implement the tool.**
  - List it when `identity.can_write`, `self.writes` is set, and `move_source` returns `Ok`.
  - In `call`, add the name to the write-request branch of `known`, and route it to a new `run_move_request`.
  - `run_move_request` accepts only the keys `target`, `goal`, `constraints`, `decisions`, and `open_questions`. `target` accepts only `channel`, `new_branch`, and `base`. Any other key gives "Only `target`, `goal`, `constraints`, `decisions`, and `open_questions` are accepted."
  - Description: "Ask the user to move this conversation to another channel or to a new branch. Only this project's Primary / Moderator agent has this tool. Give the target and a handoff: the goal, and any constraints, decisions, and open questions. Does nothing until the user approves in the app. After approval, the app posts the handoff and the user's latest message in the target and starts a response there. This channel's transcript is not copied, so the handoff must stand alone."
  - Input schema:

```json
{"type":"object","properties":{
  "target":{"type":"object","description":"Give exactly one of channel or new_branch.","properties":{
    "channel":{"type":"string","description":"The name of an existing channel in this project, with or without #."},
    "new_branch":{"type":"string","description":"The name of a branch to create. The app also creates its worktree and channel."},
    "base":{"type":"string","description":"With new_branch only: the local branch to start from. Leave out to use the default branch."}},
    "additionalProperties":false},
  "goal":{"type":"string","description":"What the work in the target must achieve. At most 600 characters."},
  "constraints":{"type":"array","items":{"type":"string"},"maxItems":12},
  "decisions":{"type":"array","items":{"type":"string"},"maxItems":12},
  "open_questions":{"type":"array","items":{"type":"string"},"maxItems":12}},
 "required":["target","goal"],"additionalProperties":false}
```

  - Result: `{"approvalRequired":true,"request":<WriteRequest>,"message":"The move is pending in the app. Nothing has moved. You cannot approve this request. After approval, the conversation continues in the target channel. End your reply here."}`
- [ ] **Step 4: Add the prompt line.** In `prepare_scoped`, when `can_write`, `self.writes` is set, and `move_source` returns `Ok`, add:

  "\nYou are this project's Primary / Moderator agent. When the user asks to move this conversation to another channel or to a new branch, call conversation_request_move with the target and a handoff. Do not save the handoff to memory instead. Other agents cannot move a conversation.\n"
- [ ] **Step 5: Run the tests.** Commands: `cargo test -p discard-mcp` and `cargo test -p discard-providers`. Expected: PASS.

---

### Task 4: App — approve and deny a move

**Files:**
- Modify: `crates/discard-app/src/desktop/core.rs` (`send_in_background`)
- Modify: `crates/discard-app/src/desktop/channels.rs` (`decide_write_request`, new private `approve_move`)
- Test: both files

**Interfaces:**
- Consumes: everything Tasks 1 and 2 produce; `GitWorkspace::create_channel_with_worktree`; `branch_actions::delete_branch`; `Store::insert_authored_message`; `Store::save_channel_topic`; `mentions::recipient`.
- Produces:
  - `AppCore::send_in_background(&self, channel: &str, text: String, recipient: Option<MessageAuthor>, attachments: Vec<Attachment>) -> Result<(), String>`. It has the checks and effects of `send_to_agent_with_attachments`, but the channel only needs to be loaded in the pool. No view needs to show it.
  - `Channels::decide_write_request` keeps its signature. For a `ConversationMove` request it runs the steps below.

- [ ] **Step 1: Write failing tests.**
  - `core.rs`:
    - `send_in_background_reaches_a_loaded_channel_that_no_view_shows`: the channel's command receiver gets `Command::Send`, and the view's selected channel is unchanged.
    - `send_in_background_refuses_a_running_or_read_only_channel`: the error is "This channel cannot send a message right now."
  - `channels.rs` (use the `Rig` near line 2214, whose launcher keeps the command receivers; the source is `#general`, and the caller is the project main agent):
    - `move_to_an_existing_channel_posts_the_handoff_and_starts_the_target`. After approval:
      - The stored target messages are, in order: a `System` message with `Block::ChannelLink { direction: MovedFrom, channel_name: "general", .. }`, then an `Agent` message whose author is the moderator and whose text is `"Handoff from #general\n\n" + handoff.markdown()`.
      - The target's command receiver has one `Command::Send` with the text of the source's user message.
      - The source's last stored message is a `System` message with `Block::ChannelLink { direction: MovedTo, .. }` that names the target.
      - The source's command receiver is empty. The view still selects the source.
      - The target's topic is unchanged. No other source message is in the target.
      - The request state is `completed`, and its result is "Moved to #feature."
    - `move_with_attachments_sends_them_to_the_target`: a source message with one attachment and no text gives `Command::SendAttachments` with the same attachment ID.
    - `move_to_a_new_branch_creates_the_branch_worktree_channel_and_topic`. After approval:
      - `refs/heads/<branch>` exists at the recorded `base_commit`.
      - A `Branch` channel for it exists with an owned worktree record. The record has the branch name and the base commit.
      - The channel's topic is `handoff.markdown()` at version 1, with an agent author.
      - The new channel got the same two messages and one `Send`.
    - `a_denied_move_posts_a_note_and_changes_nothing`: the source gets the `System` note "Conversation move denied." No branch, channel, or target message is created. No command is sent.
    - `a_move_waits_for_the_source_and_the_target`: with a turn running in the source, the error is "Wait for the current response to finish." With a turn running in the target, the error is "#feature is busy: A turn runs in this channel. Stop it first." In both cases the request is still `pending`, and a later approval succeeds.
    - `a_move_is_approved_once`: a second approval gives "This request is no longer pending." The target has one handoff.
    - `a_move_fails_cleanly_when_state_changed`. Each case ends with the request `failed`, a source note that starts with "Conversation move failed: ", and no target message:
      - The main agent changed: the gate message.
      - The target channel was archived: "The target channel is no longer available."
      - The source message was deleted: "The message to move is no longer available. Ask again."
      - The new branch name was taken: "A branch named `x` already exists."
    - `a_failed_worktree_leaves_no_branch_and_no_channel`: make the worktree folder a file before approval. After approval, `refs/heads/<branch>` does not exist, and the project has no new channel.
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-app move_ send_in_background`. Expected: FAIL.
- [ ] **Step 3: Implement `send_in_background`.** Share one private function with `send_to_agent_with_attachments`. The only difference is the lookup: the loaded channel, not the shown channel.
- [ ] **Step 4: Implement the move in `decide_write_request`.** Keep the `operation` lock for the whole call.
  1. **Before `requests.begin`, for an approval:** keep the existing source checks (`active_channel_id(view) == channel`, `begin_write_approval`). Then read the request with `requests.list`. For a `Channel` target that is loaded, when `core.channel_busy_reason(target)` is `Some(reason)`, return `Err(format!("#{name} is busy: {reason}"))`.
  2. `requests.begin`. A denial posts the Markdown note "Conversation move denied." in the source and ends.
  3. Check again: `workspace.move_source(source, agent, project)`. Load the source message by `source_message_id`. Its text is the joined `Block::Markdown` texts. Its attachments are its `Block::Attachment` blocks.
  4. Resolve the target channel.
     - `Channel`: load it. It must be in the project and not archived. Start it when the pool does not hold it (`self.start`).
     - `NewBranch`: run `validate_branch_name(repo_path, branch, None)`, then `create_branch_at(repo_path, branch, base_commit)`. Build the channel as `Channels::create_worktree` does for a branch with no channel: the name is the branch name (or `<repository label>/<branch>` for a nested repository) with a `-2`, `-3` suffix when taken, and the agent is the project main agent. Call `workspace.create_channel_with_worktree`. When it fails, call `delete_branch(repo_path, branch, true)` and return the error. Then start the channel and save the topic: `save_channel_topic(&id, 0, &handoff.markdown(), TopicAuthorKind::Agent, Some(&agent))`.
  5. Resolve the recipient with `mentions::recipient(&store, &target, &text)`, as `send_with_options` does.
  6. Insert two messages in the target, and apply each with `Update::ConversationMarker`:
     - `Role::System`, `[Block::ChannelLink { direction: MovedFrom, channel_id: <source>, channel_name: <source name> }]`
     - `Role::Agent`, `[Block::Markdown { text: format!("Handoff from #{source}\n\n{}", handoff.markdown()) }]`, saved with `insert_authored_message` under the moderator's `MessageAuthor`. Record the author in the channel's `UiState` with `set_author`.
  7. `core.send_in_background(target, text, recipient, attachments)`.
  8. Insert in the source, and apply: `Role::System`, `[Block::ChannelLink { direction: MovedTo, channel_id: <target>, channel_name: <target name> }]`.
  9. `requests.finish(id, Ok(format!("Moved to #{name}.")))`. Refresh the branch list. Start no response in the source.

  When a step from 3 to 7 fails, call `requests.finish(id, Err(format!("Conversation move failed: {error}")))`, and post that text as a Markdown `System` note in the source. A move never calls `continue_in`.
- [ ] **Step 5: Run the tests.** Command: `cargo test -p discard-app`. Expected: PASS.

---

### Task 5: Web — the move card and the link card

**Files:**
- Modify: `web/src/bridge/api.ts` (`WriteRequest`), `web/src/bridge/types.ts` (`BlockDto`)
- Modify: `web/src/components/workspace/WriteAccessApprovals.vue`
- Create: `web/src/components/workspace/MoveRequestDetails.vue`
- Create: `web/src/components/ChannelLinkCard.vue`
- Modify: `web/src/components/BlockView.vue`
- Test: `web/src/components/workspace/__tests__/WriteAccessApprovals.test.ts`, `web/src/components/workspace/__tests__/MoveRequestDetails.test.ts`, `web/src/components/__tests__/ChannelLinkCard.test.ts`

**Interfaces:**
- Consumes: the JSON of Task 1.
- Produces:

```ts
export interface MoveHandoff { goal: string; constraints: string[]; decisions: string[]; openQuestions: string[] }
export type MoveTarget =
  | { kind: 'channel'; channelId: string; name: string }
  | { kind: 'new_branch'; repository: string; repoPath: string; branch: string; base: string; baseCommit: string }
export interface MoveDetails { target: MoveTarget; handoff: MoveHandoff; sourceMessageId: string }
// WriteRequest: kind gains 'conversation_move'; new optional field `move?: MoveDetails`
// BlockDto gains: { type: 'channel_link'; direction: 'moved_to' | 'moved_from'; channel_id: string; channel_name: string }
```

  - `MoveRequestDetails.vue` has one prop, `details: MoveDetails`.
  - `ChannelLinkCard.vue` has one prop, `block: Extract<BlockDto, { type: 'channel_link' }>`.

- [ ] **Step 1: Write failing tests.**
  - `MoveRequestDetails.test.ts`:
    - `shows an existing channel target`: the text has "To" and "#feature".
    - `shows a new branch with its base and short commit`: "New branch `importer`" and "`main` at `1a2b3c4`" (the first 7 characters of `baseCommit`).
    - `shows each handoff section that has content`: "Goal", "Decisions", and "Open questions" are present. "Constraints" is absent when the list is empty.
    - `handoff text renders as plain text`: a goal of `<img src=x onerror=alert(1)> **bold**` appears as that literal text, and the DOM has no `img` and no `strong` element.
  - `WriteAccessApprovals.test.ts`:
    - `a move request shows the move card`: the title is "Move this conversation?", the buttons are "Deny" and "Move", and the card has no "Path" row.
    - `a move to an existing channel explains what approval does`: "Approval posts this handoff and your latest message in #feature and starts a response there. This transcript is not copied. You stay in this channel."
    - `a move to a new branch explains what approval creates`: "Approval creates the branch, a worktree, and a channel. It posts this handoff and your latest message there and starts a response. This transcript is not copied. You stay in this channel."
    - `move approval is disabled while the channel is running`
    - `move and deny call the command once`: `decideWriteRequest('c', 'r', true)` and `decideWriteRequest('c', 'r', false)`.
  - `ChannelLinkCard.test.ts`:
    - `shows the direction and the channel`: "Moved to #feature" and "Moved from #general".
    - `open channel switches the channel and refreshes`: the button "Open channel" calls `switchChannel('<id>')`, then `useStore().refresh()`.
    - `a failed switch shows the error`: "Could not open channel: <error>".
- [ ] **Step 2: Run them to see them fail.** Command: `pnpm --dir web test -- WriteAccessApprovals MoveRequestDetails ChannelLinkCard`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - The move card keeps the section element, `data-ui-id="write-access-approval"`, `data-protected="true"`, the protected tokens, the expiry text, and the polling of `WriteAccessApprovals.vue`. Only the title, the body (`MoveRequestDetails`), the explanation, and the approve label change with the kind.
  - `MoveRequestDetails` uses a `dl`. Lists use `ul`. All handoff text uses text interpolation. Do not use `v-html` or `SafeMarkdown`.
  - `ChannelLinkCard` is a plain bordered row with `data-ui-id="channel-link"`. It uses the plain card tokens, not the protected tokens: it reports and asks for nothing. `BlockView.vue` renders it for `block.type === 'channel_link'`.
- [ ] **Step 4: Run the tests.** Commands: `pnpm --dir web test` and `pnpm --dir web check`. Expected: PASS.

---

### Task 6: Documentation and proof

**Files:**
- Modify: `docs/worktrees.md` (new section "Conversation moves (slice C)" after "Agent workspace writes (slices A and B)"; keep that heading, because other pages link to its anchor)
- Modify: `docs/runtime/conversation-context.md` (new section "Conversation moves" after "Approved write requests")
- Modify: `docs/project-channel-status.md`

- [ ] **Step 1: Update the docs.**
  - `worktrees.md`: describe the tool and its moderator gate, the two targets, the card, what approval does in order, what a new branch records (branch name and base commit in the worktree record), the failure rule (no branch or channel remains after a failed worktree), and denial.
  - `conversation-context.md`: state that the target receives the handoff as an authored agent message and the user's message as a normal user message, so both enter replay. State that link blocks are `System` messages and never enter replay. State that a new channel also gets the handoff as its topic.
  - `project-channel-status.md`:
    - Set "Updated" to 2026-11-17.
    - In "Remaining approved design", change the third bullet to "Main-agent tools for progress links and integration." Conversation moves and new-branch channels by the main agent are no longer remaining work.
    - In "Agent workspace writes", replace the slice C paragraph. State that conversation moves and new-branch targets are implemented. State that "create a branch here" for detached worktrees (decision 17) and native provider resume remain planned.
- [ ] **Step 2: Run the proof command.** Expected: every command exits 0.
