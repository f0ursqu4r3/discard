# Native provider resume per agent — design

**Date:** 2026-11-18
**Status:** Approved. Implemented. Approved by the owner on 2026-11-18 (approach A, with summary headroom).
**Builds on:** [Agent workspace writes — design](2026-11-09-agent-workspace-writes-design.md), decision 6 ("A provider session for each agent, with native resume, is a later slice.")

## Intent

Today every Conversation and Workspace response starts a fresh provider session. Discard rebuilds the prompt from the saved transcript, personality, and memory, and summarizes the history when it does not fit. Long channels pay for the whole history on every response, and summaries lose detail.

Discard will keep one provider session for each agent in each channel and continue it. The provider already holds the earlier turns, so Discard sends only what is new. The owner's reason is cost, latency, and fewer lossy summaries on long histories. The user must not see any difference in what an agent knows.

## Current state

- `SessionMode::Conversation` and `Workspace` never pass `resume_handle`: `engine.rs:817` sets it only for `Coding`, and both adapters refuse it (`claude/provider.rs:257`, `codex/provider.rs:302`).
- `Coding` stores one handle for each session row (`sessions.resume_handle`, one row for each channel) and saves it after a successful turn (`engine.rs:1108`).
- Codex starts Conversation threads with `ephemeral: true` (`codex/provider.rs:166`). Claude Conversation sessions run in a fresh Discard-owned empty working directory.
- Both providers can resume: Claude with `--resume <id>`, Codex with `thread/resume`. Codex reports a rejected resume as `ProviderError::ResumeRejected`.
- The engine rebuilds context for every response: personality, applicable memory, topic, project and worktree context, repository instructions, and the transcript with a rolling summary (`conversation_context.rs`, `docs/runtime/conversation-context.md`).
- Several agents can answer in one channel, so one provider session per channel is not enough.

## Goals

- A response in a channel continues that agent's provider session when it is safe, and sends only new information.
- Every drift between the provider session and the current Discard state is detected. A drift drops the handle and starts a full replay, with the reason shown to the user.
- No isolation control changes. A resumed session passes the same adapter checks as a fresh one.
- The saved transcript, the replay path, and the summary machinery stay as they are, with one exception: the summary target for resumable agents (see "Summary headroom"). Replay is the fallback and the reference behavior.

## Non-goals and anti-goals

- No resume for summary sessions, sub-agent threads, or the `--builder` Coding path.
- No sharing of one provider session between agents or between channels.
- No use of provider-side memory, project instruction files, or session features beyond resume.
- No change to what the user sees in the transcript.
- **Anti-goal:** a resumed agent that answers from stale context (old memory, old topic, a missed message). Falling back to replay is always preferred to a doubtful resume.

## Constraints

- Isolation rules of decision 5 stay: no user MCP servers, hooks, automatic provider memory, or provider plugins. Every resumed process gets the same flags, settings, and per-session MCP config as a fresh one. Each response still issues a new MCP token, so the resumed session gets a new MCP config file.
- A write session keeps the write lock, the sandbox checks, and the changes card for every response.
- Provider CLIs keep resumed transcripts on disk, under the provider's own folders, outside Discard's control. Discard cannot delete them. The feature is opt-in because of this.

## Approaches

- **A. Resume per (channel, agent); send only new messages; replay when anything drifts.** Chosen by the owner. It gives the saving even when memory changes after most responses, because memory goes in as a delta. It has the most moving parts.
- **B. Resume, but any change to prompt context drops the handle.** Simple and always correct. Automatic memory saves change the context after most responses, so the handle would drop often and the gain would be small.
- **C. No resume; keep a stable prompt prefix so provider-side caching hits.** Smallest change. The gain depends on provider cache behavior that Discard does not control, and it does not match the spec's "native resume".

## Design

### Setting (builder's call, owner may change)

`AgentConfig.native_resume: bool`, false by default. It lives next to `workspace_write` and `sandbox_autorun` in agent settings, with a plain note: "Keeps this agent's provider session between responses. The provider stores the conversation on disk." It needs no policy-loosening confirmation, because it grants no new tool access.

### Session record

One new table, `agent_sessions`, keyed by (`channel_id`, `agent_id`):

| Column | Meaning |
|---|---|
| `handle` | the provider's session or thread ID |
| `fingerprint` | hash of the session fingerprint (below) |
| `delivered_through` | the ID of the last saved message that the provider session has seen |
| `delivered_memory` | JSON map of memory item ID to version already given to the session |
| `delivered_topic_version` | topic version already given |
| `delivered_capture_paused` | capture-pause flag already given |
| `bytes_in_session` | estimate of bytes the session holds (input sent plus output received) |
| `cwd` | the stable working directory of this session |
| `dropped_reason` | set when the handle is unusable (a marker such as `handle_rejected`, `provider_compacted`, `over_budget`, or the text of an unsafe previous turn). The next response replays and shows this reason. |

The fingerprint is stored as canonical JSON text, so the engine can name the first differing field. Migration to the next `user_version`. Deleting a channel, archiving it, or deleting the agent deletes its rows. The `sessions` table and the Coding path stay unchanged.

### Session fingerprint

A handle is usable only when the saved fingerprint equals the current one. The fingerprint holds:

- provider, exact model, reasoning effort, session mode
- the session root (Workspace) or the stable session directory (Conversation)
- a hash of the fixed system prompt: personality, fixed instructions, repository instructions, and the prompt lines that name tools
- the list of Discard MCP tools offered, and the delegation flag
- the sandbox and autorun state, and the adapter's required CLI version
- the summary boundary (`through`), so a new summary forces a replay
- a history revision that changes when a message at or before `delivered_through` is deleted, edited, or removed from the channel

Any difference drops the handle and starts a fresh session with full replay.

### What a resumed response sends

1. **New messages.** Every saved non-system message after `delivered_through`, in order, framed and compacted exactly as replay frames them (author labels, posted-from-thread labels, compacted tool calls). The agent's own earlier replies are already in the provider session and are skipped.
2. **Context update.** A short block, only when something changed since the last turn: memory items that are new or have a new version, memory items that were forgotten, a new topic, and a change of the capture-pause flag. It is labelled as context data, as the topic is today.
3. **Per-turn context that is already small and volatile** (worktree snapshot, project context) is sent every turn as today.
4. **The current user message.**

The fixed system prompt is sent on every resume, not only at start. Claude does not store `--append-system-prompt`. Codex freezes developer instructions at thread start, and the fixed-prompt hash in the fingerprint guarantees that they equal the current ones. After the turn, Discard saves `delivered_through`, the memory and topic markers, and the new `bytes_in_session`.

### When Discard drops the handle and replays

- the fingerprint differs
- the previous turn on this session failed, was cancelled, or was stopped (the provider session may hold a partial turn)
- the provider rejects the handle (`ResumeRejected`, or the Claude CLI reports an unknown session) or the provider session file is gone. Claude reports this on the first send. Codex reports it when the thread starts.
- the provider compacted the session (see call 6)
- a resumed Codex thread reports a model other than the selected one
- `bytes_in_session` plus the new input exceeds the app's context budget for the model (`conversation_context::input_budget`), or the provider returns a context-limit error (the engine reads the error text with a heuristic: "prompt is too long", or "context" with "limit", "window", "too long" or "exceed", and not "deadline")
- the user turns the setting off, or changes the agent's provider or model
- the app restarts and the provider cannot resume the handle in the new process

A replay starts a fresh provider session, saves its new handle, and shows a notice that names the reason ("Fresh replay: the model changed."). A rejected resume replays once in the same response. It never loops.

### Summary headroom (owner decision, 2026-11-18)

A replay that summarizes history normally fills the budget. The next resumed turn would then exceed `input_budget` and replay again, on every turn. To prevent this, a replay for an agent with `native_resume` on summarizes down to a lower target, about 50% of the input budget. Agents with the setting off keep today's target. The summary code takes the target as a parameter. The fixed 50% is the builder's starting value and may be tuned. A summary boundary change still drops the handle (it is in the fingerprint), so the cost is one replay for each summary.

### Provider changes

- **Both adapters:** accept `resume_handle` in `Conversation` and `Workspace` when the engine passes one. They run the same fail-closed checks as for a fresh session. The engine alone decides when a handle is valid. The adapters keep rejecting a handle for summary sessions.
- **Stable session directory (Conversation):** the adapters create a fresh empty directory today. A resumed session needs the same working directory as the original (Claude finds a session by its directory). A resumable Conversation session uses a stable, Discard-owned empty directory for each (channel, agent). Workspace sessions already have a stable root.
- **Codex:** a resumable thread starts with `ephemeral: false`. A non-resumable thread stays ephemeral.
- **Claude:** a resumable session must persist (no `--no-session-persistence` for it).
- **Spike first** (below): per-session controls must hold on resume.

### UI

The channel's context panel shows "Resumed session" or "Fresh replay: <reason>" for each agent, like the existing Autorun badge. The agent settings page shows the setting and its disk note. Both are implemented, not planned, so they need no `Soon` label.

## Calls made after the builder check (author's calls; the owner may change them)

1. **Memory authority.** Memory enters a fresh session through the system prompt. A delta in a user-turn block has weaker authority. So a new, changed, or forgotten `instruction` or `preference` item drops the handle and replays. Only other item kinds (facts, decisions, commitments) go in the context update.
2. **Which memory counts as new.** The memory delta uses the same per-turn selection as replay (mandatory items, plus word overlap with the request and the two latest messages). An item is delivered when it is selected now and `delivered_memory` does not hold its ID at its current version. An item that was delivered and no longer matches stays in the session and is not retracted. A forgotten item is retracted in the update (and, if it was an instruction or preference, see call 1). The update keeps scope precedence order.
3. **Volatile prompt lines.** `prepare_scoped` is split into a fixed part and a volatile part.
   - Fixed, so in the fingerprint: personality, fixed instructions, repository instructions, tool-naming lines, the delegation line.
   - Sent every turn, like the worktree snapshot: "Recent worktree approval outcomes", "This branch channel has no worktree", main-checkout availability, and the Primary / Moderator move line.
4. **Steers.** A steer does not drop the handle. The stopped parent turn and its continuation form one logical turn, and the continuation resumes the same session. A turn that is cancelled by the user, or that fails, drops the handle. A reply that fails to save (`ResponseSaveFailed`) drops the handle, because the provider saw a reply that the transcript lacks.
5. **Own replies.** `delivered_through` advances over the contiguous prefix of replay chunks that the session has seen: messages that existed at the start of the turn, the current user message, and the saved reply. Later messages are not skipped by author, so a handoff, a thread message posted from a sub-agent session, and replies from before the handle existed are all delivered. Known limit: a message posted during the turn stops the prefix. The agent's own reply then comes after the stop and is repeated once in the next delta.
6. **Provider compaction.** The spike (item 5) asks whether each CLI compacts a long session on its own and whether a flag turns that off. If a flag exists, Discard sets it. If compaction is detectable, it counts as a drift and the next response replays. If neither, the doc and the settings note say that the provider may compact a long resumed session, and the byte rule is the only guard. Result: Claude has `DISABLE_AUTO_COMPACT=1`, and Discard sets it for resumable sessions. Codex has no off switch, so Discard sets a very high `model_auto_compact_token_limit`. Both adapters detect a compaction event (Claude `compact_boundary`, Codex a `contextCompaction` item). A detected compaction keeps the new handle and marks the row, so the next response replays with a notice.

## Spike (start of the build)

Record each answer in the capability tables in `docs/provider-claude.md` and `docs/provider-codex.md`.

1. Claude 2.1.318 with the Conversation isolation flags: does `--resume` find the session from a stable empty directory? Do `--tools`, `--permission-mode`, `--mcp-config`, `--append-system-prompt`, and the sandbox settings given on resume take effect, or does the resumed session keep the old ones? Does `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1` still apply?
2. Codex 0.166.0: does `thread/resume` with a changed per-session MCP config and the Conversation controls (`codex_controls`, tool features off) keep the new values? Does a resumed thread keep the model and developer instructions from its start?
3. What does each CLI return for an unknown or expired session ID? Pick the marker the engine reads as "handle rejected".
4. Where does each provider store the session, and does a different `HOME` or `CODEX_HOME` change that?
5. Does each CLI compact a long session on its own? Is there a flag to turn it off, or a signal in the stream when it happens?

If the spike shows that a control cannot be re-applied on resume, the adapter refuses resume for that provider and the engine replays. The feature then works only for the provider that passes.

## Testing

- Unit tests: fingerprint equality and each field that changes it, the delta selection, the memory and topic delta, the drop rules, the migration, cleanup on channel and agent deletion, the summary target (lower for resumable agents, unchanged otherwise), and a channel that overflowed once resumes on its next turn.
- Engine tests with the fake provider (`started_with` already records the handle): a second response resumes and sends only new messages; each drop rule starts a fresh replay with the right notice; a rejected resume replays once; two agents in one channel keep separate sessions.
- Process-fixture tests for both adapters: the handle is passed, the flags are the same as a fresh start, a rejected handle maps to the engine marker.
- Opt-in live probes for both providers (a three-turn conversation with a memory change between turns).
- Gate: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm --dir web check`, `pnpm --dir web test`, `pnpm --dir web build`.

## Delivery (builder's proposal)

1. **Slice 1:** the spike, the setting, the table and migration, the engine path with the fake provider, and the Claude adapter.
2. **Slice 2:** the Codex adapter, live probes, and the UI indicator.

## Left to the builder

The exact fingerprint hash and its encoding. The wording of the notices and the settings note. The history-revision mechanism, as long as it catches a delete, edit, or removal at or before `delivered_through`. The byte estimate for `bytes_in_session`, as long as it never underestimates by more than the replay estimator does.
