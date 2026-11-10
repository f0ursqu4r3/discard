# Agent workspace writes, slice B — build log

## Act 1 — Decisions

On 2026-11-09, after slice A (`b2ca6c4`), the owner approved three slice B details. They are decisions 20a–20d in [the design spec](../specs/2026-11-09-agent-workspace-writes-design.md):
- the user chooses where the Always-allow prefix ends
- Codex gets one spike for a command marker, and fails closed without one
- network rules match the exact host only

Claude wrote [the slice B plan](../plans/2026-11-09-agent-workspace-writes-slice-b.md). The decisions and the plan are in commit `40634a7`.

## Act 3 — Build

- **Spec file:** `docs/superpowers/plans/2026-11-09-agent-workspace-writes-slice-b.md`
- **Proof command:** `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && pnpm --dir web check && pnpm --dir web test && pnpm --dir web build`
- **Codex:** `codex-cli 0.166.0`, model `gpt-6-astra`, reasoning `high`
- **Thread:** `01a1c232-3c32-7520-8b7b-48aaa883c418`
- **Max fix rounds:** 2
- **Tree at launch:** clean on `codex/agent-workspace-writes` at `40634a7`, except the owner's untracked `.impeccable/` folder (checksum `cec6f57e826418d414d34cf2e962697ebf72352d`).

### Round 1 — Codex build

Codex implemented Tasks 1–8 and reported that every proof command passed (1,314 Rust tests, 422 web tests). Its report is at `/tmp/discard-slice-b-proof/report.md`.

Spike results:
- `codex_in_sandbox_marker` is `none`.
- `codex_protects_git_in_root` and `claude_protects_git_in_root` are both `false`. Approved Codex commands and patches changed `.git`. Claude's `denyWrite` blocked shell Git writes, but an approved native `Write` changed `.git/config`.
- Both adapters refuse main-checkout roots.

Codex's own reviewer found two problems, and Codex fixed both: the Coding-mode prefix UI, and an unused main-checkout grant that survived a failed follow-up.

Deviations:
- a strict `prepare_owned_branch` path for app-approved creation
- `SessionProfile::Coding`
- the approval keeps the channel operation lock until Continue is queued
- the parser also rejects CR, NUL, and path-qualified wrapper programs

### Claude's verdict — round 1

Claude ran the proof, and it exited 0 (1,314 Rust tests, 422 web tests, clippy clean, build).

The diff review confirmed:
- The parser rejects the shell syntax in the review list.
- Rule data exists only for `ProviderTool` and `NetworkDomain` prompts, so escape cards can never match a rule.
- Rule lookups read the store on every request.
- The approval command rechecks the request target before it creates anything.

`.impeccable/hook.cache.json` changed. The impeccable edit hook added an entry for the Codex thread ID, `01a1c232…`, after Codex edited `ApprovalCard.test.ts`. That is the hook's cache, not a Codex edit, and it was left as it is.

Problem: `main_checkout_request_write` is offered and can be approved, but both adapters always fail closed, so the approval can never succeed. The owner chose decision 20e: Claude main-checkout grants are allowed, and Codex agents never see the tool. Fix round 1 was sent with three items: remove the Claude refusal, add a provider capability that gates the tool and its prompt line, and update the docs.

### Round 2 — Codex fix round 1

Codex completed the three items in thread `01a1c232-3c32-7520-8b7b-48aaa883c418`. All six new tests failed before their fixes and passed after them.

### Claude's verdict — round 2

Claude checked the diff:
- The Claude main-checkout refusal is removed, and the Codex refusal remains.
- `Provider::supports_main_checkout_writes()` defaults to false, and only Claude returns true.
- The engine passes the capability into memory preparation. The MCP layer lists `main_checkout_request_write` and its prompt line only when the capability is set, and it refuses calls without it.
- The spec and the plan were not changed by Codex.

Claude ran the proof again, and it failed. `cargo test` exited 101 because one test failed: `claude_process::conversation_process_forces_memory_environment_and_empty_cwd`, with `Setup("conversation mode: Claude version check timed out")`. This test predates slice A (`fa0e5d1`), and its code did not change. Codex saw the same timeout once during Task 6. The test binary passed 5 of 5 focused reruns. The second full proof also failed, on a different test from before: `codex_process::conversation_repairs_missing_stale_and_malformed_catalogs_before_starting_once`, with `Setup("conversation mode: Codex version check timed out")`. Two binaries hit the same 10-second version-check timeout, so Claude treats this as systematic, not as load noise.

Hypothesis, not yet proven: on macOS, Rust creates child pipes without atomic close-on-exec. A child spawned at the same moment from another thread can inherit the write end of a version check's stdout. If that child lives long, for example a `sleep 300` or a provider session, `.output()` never sees end of file and times out. Slice B adds more concurrent process spawns, which would make the race more likely. If the hypothesis is right, the running app has the same race: two channels that start responses at the same time could make a version check time out. Measurement: Claude ran `claude_process` and `codex_process` 10 times each, on their own. 2 of the 20 runs failed, one in each binary, with the same timeout. The race happens inside a single test binary, so load from the rest of the workspace is not the cause.

The race fits the Rust standard library on macOS. There, `anon_pipe` calls `pipe()` and then sets `FD_CLOEXEC` in a separate step, because macOS has no `pipe2`. Discard spawns processes from about 37 call sites in 15 files, with no shared lock. One of the two fix rounds was used. The owner decides whether to fix the race before committing slice B.

The owner chose to fix the race before committing. Fix round 2, the last one, was sent with three items:
1. Reproduce the race with a stress test first, and stop if the test disproves the hypothesis.
2. Serialize every child spawn behind one lock for the whole process, held only around `spawn()`.
3. Enforce the rule with clippy `disallowed-methods`.

The required proof is the stress test failing before the fix and passing after it, 10 clean runs of each process-test binary, and 3 consecutive passes of the full proof chain.

### Round 3 — Codex fix round 2

Codex confirmed the hypothesis. Short children exited in milliseconds, but inherited pipes delayed the end of their output by about 5 seconds.

Codex added `discard_core::process`, with one static lock held only around `spawn()`. Every product spawn now goes through `spawn_std` or `spawn_tokio`, and `discard-lua` and `discard-app` now depend on `discard-core`. The workspace `clippy.toml` blocks direct `spawn`, `output`, and `status` calls on `std::process::Command` and `tokio::process::Command`.

Codex reported:
- The stress test `concurrent_spawns_do_not_hold_other_children_output_open` failed 5 of 5 times before the fix and passed 5 of 5 after it.
- Each process-test binary passed 10 of 10 runs.
- The full proof chain passed 3 of 3 times.

### Claude's verdict — round 3

Claude read the helper, the clippy rule, the stress test, and the converted call sites.

Claude found one regression and fixed it directly, because both fix rounds were used. The Claude conversation `--version` and `--help` checks, and the Codex conversation `--version` check, had lost the null stdin that `.output()` set by default. They now use `stdin(Stdio::null())` again.

Claude's runs:
- The full proof chain exited 0: 1,320 Rust tests, 422 web tests, clippy clean, and the build.
- `claude_process` and `codex_process` each passed 5 of 5 runs. The stress test passed.
- After the stdin fix, `cargo test -p discard-providers` and clippy with `-D warnings` passed.

Known limit, not fixed: macOS also sets close-on-exec on sockets in a separate step, so a socket can also leak into a concurrent child. The spawn lock does not cover sockets that are not created inside `spawn()`.

The build is ready for the owner to approve the diff.
