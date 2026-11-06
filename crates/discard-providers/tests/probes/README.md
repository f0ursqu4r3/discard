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
