# Agent workspace writes, slice A — build log

## Act 1 — Design interview

On 2026-11-09, the owner and Claude settled the design in four rounds of questions. The decisions are in [the design spec](../specs/2026-11-09-agent-workspace-writes-design.md). Claude wrote [the slice A plan](../plans/2026-11-09-agent-workspace-writes-slice-a.md) from it. Both documents are in commit `201cbdd`.

## Act 3 — Build

- **Spec file:** `docs/superpowers/plans/2026-11-09-agent-workspace-writes-slice-a.md`
- **Proof command:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build`
- **Codex:** `codex-cli 0.166.0`, model `gpt-6-astra`, reasoning `high` (from `~/.codex/config.toml`)
- **Thread:** `01a1c1e8-6a5b-7fe1-bafb-4e9e252345b2`
- **Max fix rounds:** 2
- **Tree at launch:** clean on `codex/agent-workspace-writes` at `201cbdd`, except the untracked `.impeccable/` folder, which belongs to the owner and is out of scope. Its checksum at launch was `cec6f57e826418d414d34cf2e962697ebf72352d`.
- **Launch note:** The auto-mode classifier blocked the first launch. The owner approved the local allow rule `Bash(codex exec *)` in `.claude/settings.local.json`.

### Round 1 — Codex build

Codex implemented Tasks 1–10 and reported that every proof command passed. It changed 69 tracked files (1,231 lines added, 203 removed) and added 13 files. Its report is at `/tmp/discard-workspace-proof/report.md`. Deviations and spike findings:

- The schema was already at version 8. Codex extended the version 8 migration with column checks instead of adding version 9.
- With the three planned keys, Claude's sandbox did not protect Git metadata. Codex added an explicit `sandbox.filesystem.denyWrite` for the Git folders.
- Claude's native `Write` can change `.git`, and Codex rename destinations are in `kind.move_path`. The driver classifies any edit that targets `.git` or leaves the root as `SandboxEscape`.
- Codex sends no reliable escalation marker. Every Codex command approval in a write session uses `SandboxEscape`, which is the plan's fallback.
- The approval kind is also saved in `Block::Approval`, and `CommitResult` has an optional `warning`.

### Claude's verdict — round 1

The proof passed when Claude ran it. Claude read the full diff and confirmed that `.impeccable/` is unchanged. Fix round 1 was sent for these problems:

1. `eligibility` runs up to five Git processes while the global store lock is held, and it runs them before the cheap agent checks.
2. The Codex Workspace path skips `conversation_catalog` and its cache refresh. With the 0.167.0 cache on this machine, every Codex write session would fall back to read-only.
3. Non-sandbox setup errors are reported as `SandboxUnavailable`, and the session silently retries read-only.
4. Git verification failures are labelled as sandbox failures. A new reason, `WorktreeUnverified`, is needed.
5. The Claude `build_args` can omit `denyWrite` without an error.
6. `WorkspaceLocks::holder()` can block an async worker for the whole length of a commit.
7. The changes card says "1 files" and "1 commits".
8. `workspace_write: false` is formatted inline in `AgentConfig` literals that span several lines.

### Round 2 — Codex fix round 1

Codex fixed all eight items in the same thread, `01a1c1e8-6a5b-7fe1-bafb-4e9e252345b2`. Each behavior test failed before its fix and passed after it. Codex reported no functional deviations.

### Claude's verdict — round 2

Claude read the fix diff:

- Eligibility now reads the store under the lock and runs `verify_binding` outside it, on `spawn_blocking` in the engine.
- Codex Workspace sessions use `conversation_catalog(.., CatalogTools::Workspace)`.
- Only control failures map to `SandboxUnavailable`.
- `WorktreeUnverified` was added.
- `build_args` returns an error when it gets no protected Git paths.
- The holder identity is separate from `commit_barrier`.
- The changes card labels use the singular for one file and one commit.
- The `AgentConfig` literals are formatted consistently.

Claude ran the proof again, and it exited 0:

- 1,264 Rust tests passed, with 0 failed.
- Clippy with `-D warnings` was clean.
- `pnpm --dir web check` passed.
- The web tests passed: 58 files, 410 tests.
- The web build passed.

`.impeccable/` is unchanged. One of the two fix rounds was used. The build is ready for the owner to approve the diff.
