# Sandbox autorun — design

**Date:** 2026-11-10
**Status:** Draft. The owner skipped the design interview. Every decision below is the author's call unless marked otherwise.
**Builds on:** [Agent workspace writes](2026-11-09-agent-workspace-writes-design.md)

## Intent

In a write session, every shell command that writes asks the user. An agent that edits, builds, tests, and fixes stops at each step. The user approves `cargo test` many times in one response. The agent cannot iterate until its checks pass without the user at the keyboard.

The OS sandbox already enforces the important limits: writes stay in the session root, Git metadata is protected, and network access is off. Commands that the sandbox contains do not need a person to approve them. Sandbox autorun lets an opted-in agent run these commands without a card. The user still decides everything that leaves the sandbox.

## Goals

- An opted-in agent runs sandboxed shell commands with no approval card.
- Requests to leave the sandbox keep their current cards: sandbox escape, network domain, main-checkout write, worktree creation, and edits to protected or outside paths.
- The user can see which agents have autorun, and which commands ran without approval.
- Each provider fails closed. If Discard cannot prove the sandbox contains autorun commands for a provider, that provider does not get autorun.

## Non-goals

- Network access for package registries. That is a separate change (default host rules).
- Changes to `Coding` (`--builder`) sessions.
- Changes to file-edit approval. In-root edits already approve automatically.
- Read isolation for Codex. See Security limits.

## Anti-goals

- Autorun must never approve a sandbox escape, a network domain, or a write to Git metadata. A design where one of these can happen without a card is a failure, even if it removes friction.
- Autorun must not hide work. A command that ran without approval still appears in the transcript and in the changes card.

## Current state

- Claude Workspace sets `sandbox.autoAllowBashIfSandboxed: false` (`crates/discard-providers/src/claude/provider.rs:100`). Claude sends `can_use_tool` for each writing command (spike: `touch`, `curl`, and `git add` ask; `git status` runs).
- The managed-settings check rejects `autoAllowBashIfSandboxed == true` from local managed settings (`crates/discard-providers/src/conversation.rs:252`).
- Codex Workspace uses `approvalPolicy: "untrusted"` with `sandbox: "workspace-write"` (`crates/discard-providers/src/codex/provider.rs:146`). Codex has no reliable marker that separates in-sandbox requests from escalation requests. Every Codex Workspace command approval is therefore a `SandboxEscape` card with no Always allow.
- Spikes found that neither provider protects in-root `.git` from approved native edits. Discard classifies those edits as `SandboxEscape`. Claude's `denyWrite` blocks shell writes to Git metadata. Codex has no equivalent shell protection.
- Always-allow rules match simple command prefixes for each agent in each project. They do not match compound commands, which agents use often (`cd x && cargo test`, pipes).

## Decisions

### Setting

1. Add `sandbox_autorun: bool` to `AgentConfig`. The default is false. An old agent file reads it as false.
2. Turning it on is a policy loosening. It uses `AgentChange::needs_user_approval` and the existing confirmation. The confirmation text says: commands run without asking inside the sandbox, the sandbox does not limit reads, and escapes and network access still ask.
3. The setting has effect only when `workspace_write` is on and the policy is `Ask`. When `workspace_write` is off, the control is disabled and says why. This is a state block, not a planned action, so it needs no `TODO`.
4. The setting is for each agent, not for each project. *(Author's call: it matches `workspace_write`. A per-project override can come later if a repository is untrusted.)*

### Eligibility for each response

5. A write response uses autorun when the agent has `sandbox_autorun` and the provider adapter reports `SandboxAutorun::Supported` for this session. Otherwise the response behaves as today. If the agent has the setting and the provider does not support it, the prompt states the reason once, and the channel shows it (decision 13).
6. Main-checkout grants keep per-command approval. Autorun applies only to linked worktrees and detached worktrees. *(Author's call: the main checkout holds the real `.git` directory and the user's working tree. One-response grants there should stay supervised.)*

### Claude

7. In an autorun session, the adapter sets `sandbox.autoAllowBashIfSandboxed: true`. It keeps `sandbox.enabled: true`, `allowUnsandboxedCommands: false`, and `filesystem.denyWrite` on all Git metadata paths.
8. The adapter adds `sandbox.filesystem.denyRead` for sensitive paths: `~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.gnupg`, `~/.netrc`, `~/.docker/config.json`, the Claude and Codex credential paths, and `DISCARD_HOME` except the session's own files. *(Author's call on the list. The builder can extend it.)* The sandbox covers Bash only, so the adapter also adds `permissions.deny` `Read(...)` rules for the same paths to block Claude's native `Read`, `Glob`, and `Grep`. If the pinned Claude version does not enforce either control, autorun is not supported.
9. The managed-settings check still rejects local managed settings that set `autoAllowBashIfSandboxed`. Discard sets the value itself. Managed settings must not set it in either direction.
10. Native `Edit` and `Write` keep their current path classification. `SandboxNetworkAccess` requests keep the network-domain card and host rules. Explicit unsandboxed requests keep the `SandboxEscape` card.

### Codex

11. With `approvalPolicy: "on-request"`, Codex runs commands in the sandbox without asking, and asks only when the model requests escalation. In that mode every command approval is an escalation. This removes the marker problem. It also removes the `.git` gap only if Codex does not write Git metadata without asking.
12. Codex autorun is supported only if a spike shows all of these with codex-cli 0.166.0, `on-request`, and `workspace-write` in a linked worktree:
    - `touch .git` (the pointer file) and `apply_patch` to the `.git` pointer either fail or send an approval request.
    - Writes to the common Git directory fail.
    - `apply_patch` inside the root still sends `item/fileChange/requestApproval`, or the preceding `fileChange` item gives Discard the paths before the change applies.
    - Network access stays off.

    If any check fails, Codex reports `SandboxAutorun::Unsupported` with the reason, and Codex sessions keep today's behavior. Record the results in `docs/provider-codex.md`.

    **Outcome (2026-11-10): all four checks pass.** The spike ran on codex-cli 0.166.0 on macOS with `probes/codex_workspace.py --on-request`. Results are in [Codex autorun spike results](../../provider-codex.md#autorun-spike-results). Codex reports `SandboxAutorun::Supported` on macOS. In an autorun session the adapter uses `approvalPolicy: "on-request"`. Every command approval is an escalation and keeps the `SandboxEscape` card with no Always allow. File-change approvals keep their path classification. One addition: a file-change request with a `reason` is a sandbox retry ("command failed; retry without sandbox?"), so it also gets the `SandboxEscape` card. Check c passed through its second condition: in-root `apply_patch` sends no request, and the `fileChange` item gives the paths before the change applies. The spike covers macOS and linked worktrees only, so the adapter refuses autorun on other platforms and in roots without a `.git` pointer file. Non-autorun Codex sessions keep `untrusted`. A follow-up probe found that `on-request` does not protect nested `.git` paths inside the root: shell and `apply_patch` writes there succeed with no request. The adapter therefore also refuses autorun when the root holds a nested `.git` path at session start. See Security limits.

### Nested .git check

The owner chose a post-response walk on 2026-11-10. It closes the gap that `on-request` Codex and sandboxed Claude shell commands can write under a nested `.git` with no card.

18. One bounded walk, `find_nested_git` in `workspace_session.rs`, serves the engine and the Codex adapter. It returns every `.git` entry below the root (any case), relative and sorted. It skips the root's own `.git`, does not enter a found `.git`, and does not follow symlinks. It fails closed: more than 200,000 entries, or an unreadable directory, is an error.
19. An autorun response walks the root after the provider session starts and again after the turn ends, whatever the outcome. Both walks run off the async runtime. Non-autorun, Conversation, and Coding sessions never walk.
20. Paths in the second walk that the first did not have produce a warning, saved as a system message of the channel after the response. The text lists at most five paths, then "and N more". If either walk fails, one message says so instead.
21. When the first walk of a response finds existing nested `.git` paths, the engine shows a non-persistent notice, once per session for each set of paths. Claude shows it, because Claude does not refuse. Codex refuses at start, so it never shows.
22. A system message is saved for display only. Replay to the provider leaves system messages out, and saving one starts no turn. The changes card is unchanged. Discard does not block, revert, or remove the paths.

### Visibility

13. The agent settings page shows the toggle. The channel header shows "Autorun" beside the write-session indicator while an autorun response runs. If the agent has the setting and the provider or channel does not allow it, the indicator says "Autorun off: <reason>".
14. Tool-call blocks for commands that ran without a card show "Ran in sandbox". The changes card is unchanged. It already lists every change the response made.
15. Add an audit row for each response that ran with autorun: agent, conversation, response, provider, and session root. Commands that ran without a card get no approval rows, because no approval request arrived.

### Prompt

16. The write-session prompt says that shell commands in the session root run without asking, that network access and writes outside the root still need approval, and that the agent should run the project's checks itself before it reports the work done.

### Plugins

17. Plugins can read whether a response uses autorun. Plugins cannot turn autorun on or change `sandbox_autorun`. The Rust refusal list for plugin decisions is unchanged.

## Security limits

- The sandbox limits writes and network access, not reads. `denyRead` (Claude) covers known credential paths only. An agent can read other files that the user's OS user can read.
- With autorun, a prompt injection in repository files can make the agent run commands without a person seeing them first. The sandbox, `denyRead`, and network-off contain the effect: data can reach only files in the session root, which the changes card shows before the user pushes.
- Codex has no read denials. A Codex autorun command can read credential paths such as `~/.ssh`. Network-off and the writes-only-in-root limit contain what it can send out. Do not turn on Codex autorun if the agent must not see those files.
- An autorun command or patch can create a new nested `.git` directory inside the root, or write inside an existing one, with no card. This holds for Codex `apply_patch` and shell, and for Claude shell commands. Claude's `denyWrite` covers only the root pointer, the worktree Git directory, and the common Git directory. Claude's native `Write` and `Edit` still get a card. The effect is a planted hook or Git config that runs only if someone later runs Git inside that nested directory. The Codex adapter refuses autorun when a nested `.git` path exists at session start. The engine walks the worktree before and after every autorun response and warns about new nested `.git` paths (see "Nested .git check"). The warning does not block, revert, or remove the write.
- Build scripts and test runners execute repository code inside the sandbox. That code has the same limits as the agent's commands.

## UI honesty

Follow `AGENTS.md`. A provider that cannot run autorun gets a disabled toggle with the provider reason. That is a state block, not a planned action, so it has no `Soon` label and no `TODO`. The Codex spike passed. Codex shows the reason only where its adapter reports one (a platform other than macOS).

## What is left to the builder

- Where the toggle sits on the agent settings page, and the exact copy.
- The `SandboxAutorun` type shape and how the engine passes it into the session spec.
- The final `denyRead` path list, within decision 8.

## Delivery

1. Claude: `sandbox_autorun`, eligibility, adapter settings, `denyRead`, the managed-settings check, the prompt line, the indicator, the "Ran in sandbox" label, and the audit row.
2. Codex: the spike, then `on-request` support or a recorded `Unsupported` reason. Done: supported on macOS (see decision 12).

Each part needs unit tests, process-fixture tests with the fake providers, an opt-in live probe (Claude: a writing command runs without `can_use_tool`, a write to `.git` fails, a read of a `denyRead` path fails, `curl` still sends `SandboxNetworkAccess`), `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm --dir web check`, `pnpm --dir web test`, and `pnpm --dir web build`.
