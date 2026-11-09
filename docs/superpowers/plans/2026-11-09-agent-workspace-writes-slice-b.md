# Agent Workspace Writes, Slice B — Implementation Plan

> **For agentic workers:** This plan is built with `codex-build`. The executor does not commit. The reviewer commits after reading the diff. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An agent can ask for write access where it has none. It can request a new worktree for its branch channel, or a one-response grant to write in the main checkout. The user can also save durable Always-allow rules for shell commands and network hosts.

**Architecture:** Slice A gave write sessions only to channels that already have a worktree. Slice B adds:
- two app-owned approval requests, built on the existing worktree-removal request pattern
- an engine `Command::Continue`, which starts the write-capable follow-up response after an approval
- a single-use main-checkout grant store, which the engine reads at response start
- a Discard-owned Always-allow rule table, which the engine checks before it shows a provider approval card

**Tech Stack:** Rust (tokio, rusqlite, serde), Claude Code 2.1.318, Codex 0.166.0, Vue 3 with shadcn-vue, Vite+.

**Spec:** `docs/superpowers/specs/2026-11-09-agent-workspace-writes-design.md`. It covers decisions 2, 10, 12, 18, 19, and 20a–20d. Slice A is committed as `b2ca6c4`. Read `docs/superpowers/reports/2026-11-09-agent-workspace-writes-slice-a-build-log.md` for what slice A found. Slice C (moves, new branches, "create a branch here") is out of scope.

## Global Constraints

- Write sessions fail closed. A missing fact, control, or binding gives a read-only response with a stated reason.
- The `--builder` `Coding` path keeps its behavior, including its provider-session Always allow.
- Schema changes go in a new migration to `user_version` 9 in `crates/discard-core/src/store.rs`, after the existing migrations.
- No new crate or npm dependencies. The shell-word parser is written in the project.
- App-owned approval requests expire after 10 minutes, are single-use, and are cancelled by an app restart. This is the existing `TTL` in `worktree_access.rs`.
- Only the user can approve worktree creation and main-checkout grants. These approvals are never available as an MCP tool, a rule, or a plugin decision.
- UI honesty (`AGENTS.md`): remove the slice A `Soon` Always-allow button and its `TODO` in write sessions. A control that is disabled because of current state needs no `TODO`.
- User-facing text uses short, direct sentences and active voice. Use the exact strings below.
- Proof command (run from the repo root):
  `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build`

## Review Focus

1. **Shell syntax that slips past a rule.** Examples: `cargo test; rm -rf ~`, `cargo test && curl x`, `cargo test | sh`, `cargo test > ~/.zshrc`, `cargo test $(curl x)`, `` cargo test `id` ``, `FOO=1 cargo test`, `sudo cargo test`, a newline between two commands, `cargo test &`, and an unterminated quote. None of these may match a `cargo test` rule. Task 2 has the tests.
2. **The scope of a main-checkout grant.** A grant covers exactly one response, in one channel, for one agent. A second response in that channel is read-only again. A grant never moves to another channel. A response from another agent in the same channel, for example through a mention, does not use it. Task 7 has the tests.
3. **Revoking a rule mid-response.** Revoking a rule affects the next approval request, including one in a response that is already running. Task 3 has the test.
4. **State changes between request and approval.** The branch is checked out somewhere else, deleted, or the target folder already exists. Each case fails with a clear message and leaves no partial binding or folder. Task 6 has the tests.
5. **Approving twice, after expiry, or for another channel.** Each case is refused, and no follow-up response starts. Tasks 6 and 7 have the tests.

---

### Task 1: Spike — Codex command marker and Git protection in the main checkout

**Files:**
- Modify: `crates/discard-providers/tests/probes/codex_workspace.py`, `crates/discard-providers/tests/probes/claude_workspace.py`, `crates/discard-providers/tests/probes/workspace_execution.py`
- Modify: `docs/provider-codex.md`, `docs/provider-claude.md` (in "Workspace sessions", add "Slice B spike results")
- Modify, only if a marker is found: `crates/discard-providers/src/codex/parse.rs`, `crates/discard-providers/fixtures/codex-workspace-turn.jsonl`

**Interfaces:**
- Produces two recorded facts:
  - `codex_in_sandbox_marker`: the exact request field and values, or `none`
  - `codex_protects_git_in_root`: `true` or `false`
- Produces `claude_protects_git_in_root: true | false` for a main-checkout root with `denyWrite` on `<root>/.git`.

- [ ] **Step 1: Find a Codex marker.** Extend `codex_workspace.py` to compare `item/commandExecution/requestApproval` params for two cases:
  1. A command that Codex asks to run inside the sandbox under `untrusted`, such as `touch inside.txt`.
  2. A command for which the synthetic model requests escalation, such as a write outside the root or network access.

  Record every params field that differs. A marker is reliable only if it differs in every run of both cases and is absent from neither.
- [ ] **Step 2: Test main-checkout protection.** Add `--main-checkout` runs to both probes. The root is a normal repository root with `.git` inside it. Try `touch .git/marker`, `git add -A`, `git commit -m x`, and an edit tool write to `.git/config`. Record which ones succeed.
- [ ] **Step 3: Run both probes.** Commands: `python3 crates/discard-providers/tests/probes/codex_workspace.py --main-checkout` and `python3 crates/discard-providers/tests/probes/claude_workspace.py --main-checkout`. Expected: each exits 0 and prints a JSON report.
- [ ] **Step 4: Record the results** in both provider docs.
  - If `codex_in_sandbox_marker` is not `none`: change `codex/parse.rs` so that a Workspace command approval with the in-sandbox marker gets `ApprovalKind::ProviderTool`. Every other command approval keeps `SandboxEscape`. Update the fixture and the parser test `workspace_recording_uses_file_paths_and_escape_fallback`, and add `in_sandbox_marker_gives_provider_tool_kind`.
  - If it is `none`: change nothing in the parser.

---

### Task 2: Always-allow rules — parsing, matching, and storage

**Files:**
- Create: `crates/discard-core/src/always_allow.rs` (register it in `lib.rs`)
- Modify: `crates/discard-core/src/store.rs` (migration 9, rule API, cleanup on agent and project deletion)

**Interfaces:**
- Produces:
  - `pub fn command_words(command: &str) -> Option<Vec<String>>` returns `None` for any syntax it does not accept.
  - `pub fn default_cut(words: &[String]) -> usize` returns the number of words up to and including the first word after the program that does not start with `-`. It returns 1 when there is no such word.
  - `pub fn normalize_host(host: &str) -> Option<String>` lowercases ASCII and accepts only `[a-z0-9.-]`, with no leading or trailing dot. It returns `None` for `*`, an empty string, a port, or a path.
  - `#[serde(tag = "kind", rename_all = "snake_case")] pub enum RuleTarget { Command { words: Vec<String> }, Host { host: String } }`
  - `pub enum RuleRequest<'a> { Command(&'a [String]), Host(&'a str) }`
  - `pub fn matches(target: &RuleTarget, request: &RuleRequest) -> bool`. A command matches when the request words start with the rule words exactly. A host matches when it is equal after normalization.
  - `pub struct AlwaysAllowRule { pub id: String, pub agent: AgentId, pub project: ProjectId, pub target: RuleTarget, pub created_at: i64 }`
  - `Store::insert_always_allow_rule(&self, agent: &AgentId, project: &ProjectId, target: &RuleTarget) -> Result<AlwaysAllowRule>`. Inserting the same target again returns the existing row.
  - `Store::always_allow_rules(&self, project: &ProjectId) -> Result<Vec<AlwaysAllowRule>>`
  - `Store::delete_always_allow_rule(&self, project: &ProjectId, id: &str) -> Result<bool>`
  - `Store::matching_always_allow_rule(&self, agent: &AgentId, project: &ProjectId, request: &RuleRequest) -> Result<Option<AlwaysAllowRule>>`

- [ ] **Step 1: Write failing tests in `always_allow.rs`.**

```rust
#[test]
fn simple_commands_parse_into_words() {
    assert_eq!(command_words("cargo test -p discard-core workspace").unwrap(), ["cargo", "test", "-p", "discard-core", "workspace"]);
    assert_eq!(command_words("git log --format='%h %s'").unwrap(), ["git", "log", "--format=%h %s"]);
    assert_eq!(command_words(r#"echo "a b" c\ d"#).unwrap(), ["echo", "a b", "c d"]);
}

#[test]
fn shell_syntax_never_parses() {
    for command in ["cargo test; rm -rf ~", "cargo test && curl x", "cargo test || true", "cargo test | sh",
        "cargo test > ~/.zshrc", "cargo test < input", "cargo test $(curl x)", "cargo test `id`", "cargo test ${HOME}",
        "cargo test $HOME", "FOO=1 cargo test", "sudo cargo test", "cargo test\nrm x", "cargo test &", "cargo 'test",
        "(cargo test)", "cargo test 2>&1", ""] {
        assert_eq!(command_words(command), None, "{command:?}");
    }
}

#[test]
fn default_cut_is_program_and_first_non_flag_word() {
    let words = |s: &str| command_words(s).unwrap();
    assert_eq!(default_cut(&words("cargo test -p discard-core workspace")), 2);
    assert_eq!(default_cut(&words("ls -la")), 1);
    assert_eq!(default_cut(&words("npm run build")), 2);
    assert_eq!(default_cut(&words("pnpm --dir web test")), 3);
}

#[test]
fn command_rules_match_prefixes_only() {
    let rule = RuleTarget::Command { words: vec!["cargo".into(), "test".into()] };
    let ok = command_words("cargo test -p x").unwrap();
    let other = command_words("cargo testx").unwrap();
    let short = command_words("cargo").unwrap();
    assert!(matches(&rule, &RuleRequest::Command(&ok)));
    assert!(!matches(&rule, &RuleRequest::Command(&other)));
    assert!(!matches(&rule, &RuleRequest::Command(&short)));
}

#[test]
fn host_rules_match_exact_hosts_only() {
    assert_eq!(normalize_host("Registry.NPMjs.org"), Some("registry.npmjs.org".into()));
    for bad in ["*.npmjs.org", "", "registry.npmjs.org:443", "registry.npmjs.org/x", ".npmjs.org"] { assert_eq!(normalize_host(bad), None, "{bad}"); }
    let rule = RuleTarget::Host { host: "registry.npmjs.org".into() };
    assert!(matches(&rule, &RuleRequest::Host("REGISTRY.npmjs.org")));
    assert!(!matches(&rule, &RuleRequest::Host("evil.registry.npmjs.org")));
}
```

  `default_cut` does not know which flags take values. In `pnpm --dir web test`, the first word that is not a flag is `web`, so the default is `pnpm --dir web`. The test pins this result. The user can widen the cut to include `test` on the card.

  Store tests:
  - `rules_round_trip_and_deduplicate`
  - `rules_are_scoped_to_agent_and_project`: a rule for agent A in project P does not match agent B, or agent A in project Q.
  - `deleting_a_rule_stops_matching`
  - `deleting_an_agent_or_project_removes_its_rules`
  - `migration_9_adds_the_rule_table_to_a_version_8_database`
- [ ] **Step 2: Run them to see them fail.** Commands: `cargo test -p discard-core always_allow` and `cargo test -p discard-core store::tests::rules`. Expected: compile errors.
- [ ] **Step 3: Implement the module and the store API.** `command_words` is a small state machine. It accepts unquoted words, single quotes, double quotes, and backslash escapes outside single quotes. It rejects these, anywhere outside single quotes:
  - unquoted `;`, `&`, `|`, `<`, `>`, `(`, `)`, `` ` ``, and `$`
  - a newline
  - `$` or `` ` `` inside double quotes
  - an unterminated quote
  - a first word that contains `=`
  - a first word that is `sudo`, `doas`, `env`, `exec`, `eval`, `command`, `builtin`, `nohup`, or `xargs`
  - an empty result

  Migration 9 creates this table:

```sql
CREATE TABLE always_allow_rules (
  id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents(id), project_id TEXT NOT NULL,
  target_json TEXT NOT NULL, created_at INTEGER NOT NULL, UNIQUE(agent_id, project_id, target_json));
```

  Agent hard-deletion and project removal delete the rules that belong to them.
- [ ] **Step 4: Run the tests.** Command: `cargo test -p discard-core`. Expected: PASS.

---

### Task 3: Engine — save and apply rules

**Files:**
- Modify: `crates/discard-core/src/ui_state.rs` (`Command::Decide` gains `rule_cut`)
- Modify: `crates/discard-core/src/timeline.rs` (`ApprovalPrompt` gains rule data)
- Modify: `crates/discard-core/src/approval.rs` (`Policy` may decide `NetworkDomain`)
- Modify: `crates/discard-core/src/engine.rs` (match rules, convert Always allow into a rule plus Approve)
- Modify: `crates/discard-app/src/desktop/core.rs` (`decide_in`), `crates/discard-app/src/desktop/commands.rs` (`decide_approval`), `crates/discard-app/src/desktop/dto.rs` (`PromptDto`)
- Test: `crates/discard-providers/tests/engine.rs`, `crates/discard-core/src/approval.rs`

**Interfaces:**
- Consumes: everything Task 2 produces.
- Produces:
  - `Command::Decide { approval_id: ApprovalId, decision: Decision, rule_cut: Option<usize> }`. Every existing sender passes `None`.
  - `ApprovalPrompt { .., kind, rule_words: Option<Vec<String>>, rule_host: Option<String> }`, filled by the timeline. A `Bash` or `command` tool with a parseable `input.command` gets `rule_words`. `NetworkDomain` with a valid `input.host` gets `rule_host`.
  - `PromptDto { .., kind, always_allow: String, rule_words: Option<Vec<String>>, default_cut: Option<usize>, rule_host: Option<String>, always_allow_note: Option<String> }`
  - The Tauri command `decide_approval(channel_id, approval_id, decision, rule_cut: Option<usize>)`.

- [ ] **Step 1: Write failing tests.**
  - `approval.rs`: `policy_may_decide_network_domain_but_not_escapes`. `Decider::Policy("always_allow:r1")` may approve `NetworkDomain`. It may not approve `SandboxEscape`, `MainCheckoutWrite`, or `WorktreeCreate`. `Decider::Plugin` still decides only `ProviderTool`.
  - `engine.rs`:
    - `matching_command_rule_auto_approves_in_workspace_sessions`: the audit decider is `Policy("always_allow:<id>")`.
    - `matching_host_rule_auto_approves_network_requests`
    - `compound_command_never_matches_a_rule`: a rule `cargo test` and the request `cargo test; rm x` leave the card pending.
    - `always_allow_saves_the_chosen_cut_and_approves_once`: `rule_cut = Some(2)` on `cargo test -p x` saves `["cargo","test"]`. The provider receives `Decision::Approve`, not `AlwaysAllow`.
    - `always_allow_without_a_cut_uses_the_default`
    - `always_allow_for_unparseable_command_is_refused`: the error is "This command uses shell syntax that Always allow cannot match. Approve it once." Nothing is forwarded.
    - `revoked_rule_stops_matching_in_a_running_response`: two approval requests in one response, with the rule deleted between them. The first is auto-approved, and the second stays pending.
    - `rules_never_apply_to_escapes_or_other_agents`
    - `coding_sessions_keep_provider_session_always_allow`
- [ ] **Step 2: Run them to see them fail.** Commands: `cargo test -p discard-core approval` and `cargo test -p discard-providers --test engine`. Expected: FAIL.
- [ ] **Step 3: Implement the changes.**
  - In `Approval::decide`:
    - `Plugin` deciders work only for `ProviderTool`.
    - `Policy` deciders work for `ProviderTool` and `NetworkDomain`.
    - `User` decides every kind.
    - `AlwaysAllow` is refused for every kind except `ProviderTool`.
  - In write sessions only, the pump checks `Store::matching_always_allow_rule(agent, project, request)` before it sends the event to the UI:
    - for `ProviderTool` requests with `rule_words`
    - for `NetworkDomain` requests with `rule_host`

    A match is approved like the slice A automatic edits, with the decider `Policy(format!("always_allow:{id}"))`.
  - A user `AlwaysAllow` in a write session works like this:
    1. Build the target from the prompt. For words, use `rule_cut.unwrap_or(default_cut)` and clamp it to `1..=len`. For a host, use the host.
    2. Save the rule.
    3. Send `Update::Notice(format!("Saved an Always-allow rule for `{}` in this project.", text))`, where `text` is the joined words or the host.
    4. Send `DecisionMsg { decision: Approve, decider: User }`.

    If the prompt has no rule data, send the refusal error from Step 1 and forward nothing. Remove the slice A message "Always allow is not available in write sessions yet. Approve this request once."
  - In `core.rs`, `always_allow` is:
    - `"available"` in write sessions for `ProviderTool` prompts with `rule_words` and for `NetworkDomain` prompts with `rule_host`
    - `"available"` in `Coding` sessions, as before
    - `"unavailable"` otherwise

    `always_allow_note` is set in a write session in these cases:
    - kind `SandboxEscape` and tool `command`: "Always allow is unavailable for Codex write sessions."
    - a `ProviderTool` prompt without `rule_words`: "This command uses shell syntax that Always allow cannot match."
- [ ] **Step 4: Run the tests.** Commands: `cargo test -p discard-core`, `cargo test -p discard-providers --test engine`, and `cargo test -p discard-app`. Expected: PASS.

---

### Task 4: Web — choose the prefix, and manage rules

**Files:**
- Modify: `web/src/components/ApprovalCard.vue`, `web/src/components/ApprovalCard.test.ts`, `web/src/bridge/api.ts`, `web/src/bridge/types.ts`
- Create: `web/src/components/workspace/ProjectAlwaysAllowRules.vue`, `web/src/components/workspace/ProjectAlwaysAllowRules.test.ts`
- Modify: `web/src/components/workspace/WorkspacePage.vue` (place the new card under `ProjectIdentitySettings`)
- Modify: `crates/discard-app/src/desktop/commands.rs`, `crates/discard-app/src/desktop/mod.rs` (two commands)

**Interfaces:**
- Consumes: `PromptDto` (Task 3) and the Task 2 store API.
- Produces:
  - the Tauri command `get_always_allow_rules(project_id) -> Vec<RuleDto>`, where `RuleDto { id, agent_id, agent_name, kind: "command" | "host", text, created_at }`
  - the Tauri command `revoke_always_allow_rule(project_id, rule_id) -> bool`
  - `decideApproval(channelId, approvalId, decision, ruleCut?: number)` in `api.ts`

- [ ] **Step 1: Write failing tests.**
  - `ApprovalCard.test.ts`:
    - `always allow shows the words with the default cut selected`: words `cargo test -p x` with `defaultCut` 2 gives the preview "Always allow `cargo test` commands".
    - `clicking a word moves the cut`: clicking `-p` gives "`cargo test -p` commands".
    - `always allow sends the chosen cut`
    - `host prompts preview the exact host`: "Always allow network access to `registry.npmjs.org`".
    - `unavailable shows the note and no Always allow button`
    - `no Soon badge remains`
  - `ProjectAlwaysAllowRules.test.ts`:
    - `lists rules with agent and target`
    - `revoke removes the row`
    - `empty state explains where rules come from`: "No Always-allow rules yet. Choose Always allow on an approval card to add one."
- [ ] **Step 2: Run them to see them fail.** Command: `pnpm --dir web test -- ApprovalCard ProjectAlwaysAllowRules`. Expected: FAIL.
- [ ] **Step 3: Implement the card and the settings list.**
  - Render the words as a row of toggle buttons. The selected prefix uses the primary token, and the rest are muted. Each button has `aria-pressed` for the words inside the cut.
  - The Always allow button sends the cut.
  - The rules card uses `Card`, with the title "Always-allow rules". Each row shows the agent name, then `Command` or `Network`, then the text in `<code>`, then the date, then a Revoke button.
  - Revoke has no confirmation, because the user can save the rule again from the next card.
  - Remove the slice A disabled `Soon` button and its `TODO`.
- [ ] **Step 4: Run the tests.** Command: `pnpm --dir web test` and `pnpm --dir web check`. Expected: PASS.

---

### Task 5: `Command::Continue` — follow-up responses after approvals

**Files:**
- Modify: `crates/discard-core/src/ui_state.rs` (`Command::Continue`)
- Modify: `crates/discard-core/src/engine.rs` (`run`, `turn`, `snapshot`, `turn_snapshot`)
- Modify: `crates/discard-app/src/desktop/core.rs` (`continue_in`), `crates/discard-app/src/desktop/channels.rs` (`continue_after_approval`)
- Test: `crates/discard-providers/tests/engine.rs`

**Interfaces:**
- Produces:
  - `Command::Continue { agent: AgentId, note: String }`
  - `AppCore::continue_in(&self, channel: &str, agent: AgentId, note: String) -> Result<(), String>`
  - `Channels::continue_after_approval(&self, channel: &str, agent: AgentId, note: String) -> Result<(), String>`. It returns `Err("Wait for the current response to finish.")` while the channel is running.

- [ ] **Step 1: Write failing engine tests.**
  - `continue_answers_as_the_given_agent_without_a_new_user_message`: the stored messages gain an agent reply but no user message. The recorded provider input ends with the text from Step 3.
  - `continue_ignores_mentions_in_history`
  - `continue_uses_current_eligibility`: after a binding is added, the session mode is `Workspace`.
  - `continue_with_an_archived_agent_fails_with_the_normal_error`
- [ ] **Step 2: Run them to see them fail.** Command: `cargo test -p discard-providers --test engine continue_`. Expected: FAIL.
- [ ] **Step 3: Implement the command.**
  - `snapshot` takes `recipient: Option<AgentId>`, and a `Some` value skips mention resolution.
  - `turn_snapshot` takes `store_user_message: bool`.
  - For `Continue`, the current input is: "The user approved your request in the app: {note} Continue the user's latest request in this channel. Do not repeat work that is already done."
  - Existing `Send` behavior does not change.
- [ ] **Step 4: Run the tests.** Command: `cargo test -p discard-providers --test engine`. Expected: PASS.

---

### Task 6: Worktree creation requests

**Files:**
- Create: `crates/discard-core/src/write_requests.rs` (register it in `lib.rs`)
- Modify: `crates/discard-core/src/workspace.rs` (`GitWorkspace::create_for_branch`)
- Modify: `crates/discard-mcp/src/tools.rs`, `crates/discard-mcp/src/service.rs` (tool and prompt line)
- Modify: `crates/discard-app/src/main.rs` (shared `Arc<WriteRequests>`, `Arc<GitWorkspace>` in app state), `crates/discard-app/src/desktop/commands.rs`, `crates/discard-app/src/desktop/mod.rs`
- Create: `web/src/components/workspace/WriteAccessApprovals.vue`, `web/src/components/workspace/WriteAccessApprovals.test.ts`
- Modify: `web/src/components/workspace/TranscriptContent.vue` (render the new component next to `WorktreeApprovals`), `web/src/bridge/api.ts`

**Interfaces:**
- Consumes: `Channels::continue_after_approval` (Task 5), `WorktreeManager::prepare_branch`, `workspace_session::eligibility`.
- Produces:
  - `#[serde(rename_all = "snake_case")] pub enum WriteRequestKind { WorktreeCreate, MainCheckoutWrite }`
  - `pub struct WriteRequest { pub id: String, pub kind: WriteRequestKind, pub channel_id: String, pub agent_id: String, pub branch: String, pub path: String, pub state: String, pub result: Option<String> }`. `path` is the target worktree path, or the main checkout.
  - `pub struct WriteRequests` with:
    - `request(&self, kind, channel: &ChannelId, agent: &AgentId, branch: String, path: String) -> Result<WriteRequest, String>`
    - `list(&self, channel: &ChannelId) -> Vec<WriteRequest>`
    - `begin(&self, channel: &ChannelId, id: &str, approve: bool) -> Result<WriteRequest, String>`, which marks the request running or denied, once
    - `finish(&self, id: &str, result: Result<String, String>) -> WriteRequest`

    It keeps the same TTL, the same limit of 100 requests, and one pending request per channel and kind, as the removal requests.
  - `GitWorkspace::create_for_branch(&self, repo: &Path, channel: &ChannelId, branch: &str) -> Result<WorktreeRecord, String>`. It saves the binding with `owned: true`.
  - The MCP tool `worktree_request_creation`, with no arguments.
  - The Tauri commands `get_write_requests(channel_id)` and `decide_write_request(channel_id, request_id, approve)`.

- [ ] **Step 1: Write failing tests.**
  - `write_requests.rs`:
    - `requests_expire_after_ttl`
    - `a_request_is_decided_once`
    - `decide_for_another_channel_is_refused`
    - `one_pending_request_per_channel_and_kind`
  - `workspace.rs`:
    - `create_for_branch_saves_an_owned_binding`
    - `create_for_branch_fails_when_the_branch_is_checked_out_elsewhere`: the error is "This branch is checked out in another folder. Ask again after it is free." No folder and no binding remain.
    - `create_for_branch_fails_for_a_deleted_branch`
    - `create_for_branch_fails_when_the_target_folder_exists`
  - `tools.rs`:
    - `creation_tool_is_listed_only_for_unbound_branch_channels_with_opted_in_agents`
    - `creation_tool_refuses_a_branch_checked_out_in_the_main_checkout`: the tool error is "This branch is checked out in the main checkout. Use main_checkout_request_write instead."
  - `WriteAccessApprovals.test.ts`:
    - `shows the branch and target folder`
    - `approve is disabled while the channel is running`: the text is "Wait for the current response to finish before approving."
    - `approve and deny call the command once`
- [ ] **Step 2: Run them to see them fail.** Commands: `cargo test -p discard-core write_requests workspace`, `cargo test -p discard-mcp`, and `pnpm --dir web test -- WriteAccessApprovals`. Expected: FAIL.
- [ ] **Step 3: Implement the tool.** `worktree_request_creation` checks these, at request time:
  - The channel is a project `Branch` channel.
  - The channel has no binding.
  - The agent has `workspace_write` and policy `Ask`.
  - The branch exists at `refs/heads/<branch>`.
  - The branch is not checked out in any worktree.

  Its description is: "Ask the user to create a worktree for this branch channel. Does nothing until the user approves in the app. After approval, the app starts a new response with file edits enabled." The tool result is `{"approvalRequired":true,"request":<WriteRequest>,"message":"Worktree creation is pending in the app. No folder exists yet. You cannot approve this request."}`.

  Add one prompt line for read-only branch channels without a worktree whose agent is opted in: "This branch channel has no worktree. Call worktree_request_creation when the user asks you to change files."
- [ ] **Step 4: Implement the approval command.** `decide_write_request` with `approve = true` does this, in order:
  1. Call `begin`.
  2. Call `GitWorkspace::create_for_branch` on a blocking task.
  3. Call `finish`.
  4. Save a system note: "Created a worktree for `<branch>` at `<path>`." or "Worktree creation failed: <error>".
  5. Refresh the branches.
  6. On success, call `continue_after_approval(channel, request.agent_id, "Discard created a worktree for <branch> at <path>.")`.

  Deny saves "Worktree creation denied." and starts no response. The card follows `WorktreeApprovals.vue` (polling, protected tokens, and `data-ui-id="write-access-approval"`), with the title "Create a worktree for this branch?" and the buttons "Create worktree" and "Deny".
- [ ] **Step 5: Run the tests.** Commands: `cargo test --workspace` and `pnpm --dir web test`. Expected: PASS.

---

### Task 7: Main-checkout write grant

**Files:**
- Create: `crates/discard-core/src/workspace_grants.rs` (register it in `lib.rs`)
- Modify: `crates/discard-core/src/workspace_session.rs` (`verify_binding` with a grant, prompt line)
- Modify: `crates/discard-core/src/workspace_commit.rs`, `crates/discard-core/src/worktree_access.rs` (allow commits in the main checkout only under a grant)
- Modify: `crates/discard-core/src/engine.rs` (consume the grant at response start)
- Modify: `crates/discard-mcp/src/identity.rs` (`Identity.main_checkout_grant: bool`), `crates/discard-mcp/src/tools.rs`, `crates/discard-mcp/src/service.rs`
- Modify: `crates/discard-providers/src/codex/provider.rs` (fail closed when the Task 1 result is `false`), `crates/discard-providers/src/claude/provider.rs` (fail closed when the Task 1 Claude result is `false`)
- Modify: `crates/discard-app/src/main.rs`, `crates/discard-app/src/desktop/commands.rs` (approval writes the grant), `web/src/components/workspace/WriteAccessApprovals.vue`

**Interfaces:**
- Consumes: `WriteRequests` (Task 6) and `Command::Continue` (Task 5).
- Produces:
  - `pub struct MainCheckoutGrants` with:
    - `grant(&self, channel: &ChannelId, agent: &AgentId)`, which replaces any earlier grant for the channel
    - `take(&self, channel: &ChannelId, agent: &AgentId) -> bool`, which consumes the grant. It returns false for another agent or after 10 minutes.
  - `pub fn verify_binding(binding: WorkspaceBinding, main_checkout_granted: bool) -> Result<WorkspaceGrant, ReadOnlyReason>`
  - `WorkspaceGrant.main_checkout: bool`
  - `pub fn commit(root: &Path, request: &CommitRequest, main_checkout_granted: bool) -> Result<CommitResult, String>`
  - The MCP tool `main_checkout_request_write`, with no arguments.

- [ ] **Step 1: Write failing tests.**
  - `workspace_grants.rs`:
    - `a_grant_is_used_once`
    - `a_grant_belongs_to_one_agent`
    - `a_grant_expires_after_ttl`
    - `a_grant_never_crosses_channels`
  - `workspace_session.rs`:
    - `main_checkout_needs_a_grant`: without the flag the result is `Err(MainCheckout)`. With it, the result is `Ok` and has `main_checkout == true`.
  - `workspace_commit.rs`:
    - `main_checkout_commit_needs_a_grant`
    - `main_checkout_commit_still_runs_no_hooks`
  - `engine.rs`:
    - `granted_response_writes_in_the_main_checkout_once`: response 1 is `Workspace` with `cwd ==` the main checkout. Response 2 is `Conversation` with reason `MainCheckout`.
    - `a_mentioned_agent_does_not_use_another_agents_grant`
  - `tools.rs`:
    - `main_checkout_tool_is_listed_only_where_the_binding_is_the_main_checkout`
  - Provider tests:
    - `codex_main_checkout_fails_closed_without_git_protection`, gated on the Task 1 result
    - `claude_main_checkout_denies_writes_to_dot_git`
- [ ] **Step 2: Run them to see them fail.** Commands: `cargo test -p discard-core`, `cargo test -p discard-mcp`, and `cargo test -p discard-providers`. Expected: FAIL.
- [ ] **Step 3: Implement the grant.**
  - The engine `snapshot` calls `grants.take(channel, recipient)` only when the binding is the main checkout. It passes the result to `verify_binding`, and the identity flag carries it to `WorktreeAccess::commit`.
  - Add a prompt line for main-checkout grants: "You are writing in the main checkout for this response only. The user may also be editing these files."
  - The tool description is: "Ask the user to let you write in the main checkout for one response. Does nothing until the user approves in the app."
  - The card title is "Write in the main checkout for one response?" and the buttons are "Allow once" and "Deny".
  - Approval writes the grant and calls `continue_after_approval(channel, agent, "The user allowed writes in the main checkout for one response.")`.
  - When Task 1 found that a provider does not protect `.git` inside the root, the adapter returns `SandboxUnavailable("<provider> cannot protect Git metadata in the main checkout")` for a root whose `.git` is a folder.
- [ ] **Step 4: Run the tests.** Command: the proof command. Expected: PASS.

---

### Task 8: Documentation

**Files:**
- Modify: `docs/worktrees.md`, `docs/project-channel-status.md`, `docs/runtime/conversation-context.md`, `docs/provider-claude.md`, `docs/provider-codex.md`

- [ ] **Step 1: Update the docs.** Describe worktree creation requests, main-checkout grants, Always-allow rules (the prefix cut, exact hosts, the Codex limit if one remains, and revoking in project settings), and `Command::Continue`. Mark slice B as implemented and slice C as planned. Make no claim about moves or new branches.
- [ ] **Step 2: Run the proof command.** Expected: every command exits 0.
