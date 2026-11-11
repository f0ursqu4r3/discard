# Provider conversation probes

These opt-in probes use local synthetic model and memory endpoints. They do not make cloud model calls or copy real credentials. Run them with the supported CLI versions: Codex 0.166.0 and Claude Code 2.1.318.

From the repository root:

```sh
python3 crates/discard-providers/tests/probes/codex_conversation.py
python3 crates/discard-providers/tests/probes/claude_conversation.py
python3 -m unittest discover -s crates/discard-providers/tests/probes -p 'test_*.py' -v
```

The scripts also work from another directory when invoked by their full path. Each script locates repository files from its own location.

Use `--codex /path/to/codex` or `--claude /path/to/claude` to select a CLI executable. The defaults use `DISCARD_CODEX_BIN` or `DISCARD_CLAUDE_BIN`, then PATH. Codex reads model metadata from `--codex-home /path/to/provider-home`, `CODEX_HOME`, or `~/.codex`. It writes its test catalog and configuration to a temporary directory. It leaves the source cache unchanged.

Codex asserts all required effective controls before it creates a thread. Both probes assert the model-facing tool allowlist. The Codex probe also checks that a disabled inherited synthetic MCP endpoint receives no initialization request. A failed assertion exits with a nonzero status. Unit tests cover matching, missing and mismatched control values without starting a provider or using a network.

Each probe drives a deterministic private `memory_save`, private `memory_read`, and shared `memory_save` sequence. It requires all three `tools/call` receipts and their results in the next model request. It also checks that the synthetic shared proposal stays pending and that shared memory does not change. The Claude probe uses no permission-prompt surface; the Codex probe keeps never approval and a read-only sandbox. Neither probe grants an unexpected provider approval request.

These checks test provider permissions and transport using a synthetic in-memory MCP server. They do not replace the real memory service's scope and shared-write approval tests. Unit regressions reject tool exposure without execution, missing result delivery, early shared writes and applied proposals. The probes do not print model prompts, unrelated result content or credentials.

## Codex autorun spike

```sh
python3 crates/discard-providers/tests/probes/codex_workspace.py --on-request
```

`--on-request` runs the spec decision 12 checks. It starts Codex with `approvalPolicy: "on-request"` and `workspace-write` in a linked Git worktree inside a temporary directory. The model is a local test endpoint that scripts the tool calls, so it uses no credentials and no model turn. The script points the child `TMPDIR` at a temporary home directory, so the main checkout is not writable through a system temporary directory. It declines every approval request and records it.

The script prints one row per check and exits with a nonzero status if a check fails. Checks `a` to `d` are the spec conditions. Checks `e` and `f` confirm that only an escalation asks. Row `g` is information only. Rows `h1` to `h4` test nested `.git` paths in a worktree that holds a nested repository. They set a separate `nested .git gap` line and do not change the exit status. Unit tests in `test_codex_probe.py` cover the pass and fail logic without starting Codex.

The run saves the turn messages to `crates/discard-providers/fixtures/codex-autorun-turn.jsonl`. Rust tests replay that file. Keep the fixture item IDs in step with the operation order in `on_request_checks.py`.
