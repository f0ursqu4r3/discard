# Task 3 report: conversation provider sessions

## Result and interfaces

Added `SessionSpec.mode` with `SessionMode::Coding` and `SessionMode::Conversation`. Existing constructors remain Coding. The engine change is only a constructor compatibility field; Task 4 must select Conversation and supply a fresh transcript replay. Both real providers reject native resume handles in Conversation mode. Both providers create their own fresh empty temporary cwd and retain its owner for the session lifetime. They remove those files when the session drops. Task 4 does not need to create or prove an empty working directory.

Added `Provider::model_context_window(&str) -> Option<u64>`, defaulting to None. Codex returns the exact cached model record's published `context_window`, without invented limits. Claude returns None. No runtime summary or context-selection logic was added.

The conversation MCP file must contain exactly one `discard-memory` HTTP server with local URL and string-valued headers. Extra servers, executable MCP transport and malformed/missing configuration fail before a session starts. No authentication files, user settings or source model catalogs are changed.

## Claude controls

Require the tested 2.1.318 version and required CLI help controls. Pass `--tools ""`, `--strict-mcp-config`, `--restricted`, `--disable-slash-commands`, and `--system-prompt-snapshot off`. Supply the exact selected model, refreshed appended system prompt and scoped memory configuration. Set both memory-disable environment variables to 1 after provider test/environment overrides. Supply the same environment values through explicit session settings. Disable hooks, disable automatic memory, use managed-only instruction selection for the built-in AGENTS plugin, and clear inherited explicit plugin directories. Keep ordinary CLI subscription authentication; do not use `--bare`.

Inspect documented local managed settings files and fragments. Reject detected conflicting memory environment values, explicit hook enablement, or managed plugins. Do not change administrator policy.

The configured provider and its administrator are trusted. Remote/MDM policy can be opaque and can change after startup. The CLI has no supported effective-settings dump for it. This is an explicit evidence limit, not an OS or administrator isolation claim. Parent ruled that a hypothetical unobserved remote override must not disable an otherwise supported adapter.

## Codex controls

Require tested CLI 0.166.0, an exact selected model and a compatible installed model cache. Resolve the actual provider home from provider environment overrides, then process `CODEX_HOME`, then `HOME/.codex`. Missing cache errors tell the user to open Codex, refresh its model list and retry. There is no invented catalog or model fallback.

Extract complete model records from the cache. Exclude cache wrapper/identity fields. Preserve every unrelated model field. Change only documented capabilities: `apply_patch_tool_type:null`, `shell_type:"disabled"`, `experimental_supported_tools:[]`, `multi_agent_version:"disabled"`, `node_repl_disabled:true`, and `tool_mode:"direct"`. Write the catalog under a private temporary session directory. Pass `model_catalog_json` at process startup and verify the effective path. Never edit the source catalog. Thread start is ephemeral and sets `allowProviderModelFallback:false` only in Conversation mode.

Disable native shell, unified execution, images, delegation, plugins, apps, browser/computer use, web, image generation, file copy, code mode, automatic memories, memory-generation tools, history-note tools, skills, discovery, hooks, plugin suggestions, goals, planning and provider request-user-input. Use never approval and a read-only sandbox as additional controls.

Read effective config in a config-only process, without creating a thread or supplying memory. Enumerate every inherited named MCP server. Start the real process with explicit per-server disables. Verify final effective controls before thread/start. Carry the complete controls and disabled-server map into thread/start alongside the scoped memory entry. An inherited server already named `discard-memory` fails because its transport/settings could merge into the scoped entry. The six memory tools use an explicit allowlist and direct exposure. All other servers stay disabled.

Public `config/read` omits two utility fields from its typed `tools` object. Recover those fields from ordered raw configuration layers before checking them. Managed requirements expose feature controls, but do not define those two utility fields. All other controls use the reported effective config.

Trusted global provider instructions such as `CODEX_HOME/AGENTS.md` still apply. Source inspection found that app-server loads them unconditionally. Parent explicitly accepted that trusted provider-level default. Keep the provider home and authentication unchanged. Disable project instruction discovery with an empty cwd and `project_doc_max_bytes:0`.

## RED / GREEN evidence

Initial regression tests failed to compile because SessionMode and SessionSpec.mode did not exist. After adding the interfaces, the Claude control test failed on missing `--strict-mcp-config`. Catalog, inherited-server, missing-model/version, malformed-config and cleanup tests were added before their helper implementations.

The installed-binary synthetic probe found two defects that argv fixtures did not reveal:

1. Codex CLI splits dotted override paths literally. Quoted path segments were silently ignored. Use unquoted top-level keys with complete TOML inline tables. This also handles server names containing dots safely. A unit regression checks the exact safe serialization.
2. A per-thread top-level `mcp_servers` override can replace the startup override table and revive inherited entries from lower layers. Carry all disabled entries into the per-thread config. A process regression checks those entries. The installed-binary probe also configures a synthetic inherited HTTP endpoint and asserts that it receives no initialization request.

The Python fixture initially required Python 3.11 tomllib, but the system fixture interpreter is Python 3.9. Replaced that dependency with a bounded parser for the emitted synthetic overrides. Existing coding-process tests then passed again.

## Installed-binary boundary checks

Retained opt-in synthetic checks:

- `task-3-codex-probe.py`: Codex 0.166.0. Uses an empty temporary provider home, complete sanitized model metadata, local synthetic model API, local synthetic memory MCP server and a disabled synthetic inherited server. Reads no source authentication or global instructions. Sends no user/project content. Checks required effective controls and asserts the exact model-facing tool allowlist. Supports Responses Lite `AdditionalTools` encoding. Does not print prompt bodies or credentials.
- `task-3-claude-probe.py`: Claude Code 2.1.318. Uses an empty temporary configuration directory, empty cwd, synthetic API key and local model/MCP endpoints. Checks the actual model-facing tools. Sends no user/project content and prints only tool names and MCP method names.

Actual Claude tool list:

- `mcp__discard-memory__memory_edit`
- `mcp__discard-memory__memory_forget`
- `mcp__discard-memory__memory_list`
- `mcp__discard-memory__memory_read`
- `mcp__discard-memory__memory_save`
- `mcp__discard-memory__memory_search`

Actual Codex tool list:

- `functions.list_mcp_resources`
- `functions.list_mcp_resource_templates`
- `functions.read_mcp_resource`
- `mcp__discard_memory.memory_edit`
- `mcp__discard_memory.memory_forget`
- `mcp__discard_memory.memory_list`
- `mcp__discard_memory.memory_read`
- `mcp__discard_memory.memory_save`
- `mcp__discard_memory.memory_search`

Codex registers generic resource helpers whenever an MCP server exists. Parent accepted these scoped adapters because only Discard memory is enabled; they add no filesystem or subprocess authority. The synthetic inherited endpoint received no request after the corrected startup/thread configuration. The local model request contained no shell, patch, image, delegation, goal, user-input or code-mode tool.

`node_repl_disabled` is forwarded by source through turn metadata. The proprietary backend's enforcement is not independently visible in the local test. The observed native tool surface contains no node/code-mode executor; documented direct tool mode removes local code-mode registration. This does not prove administrator/remote-provider behavior or an OS sandbox boundary.

These are local synthetic tool-surface checks, not cloud model calls or credential-flow tests. Existing setup/login and ordinary coding-session process tests remain covered. No broad safety claim rests on fake argv tests alone.

## Verification

Used `CARGO_TARGET_DIR=/Users/la.kyle.dougan/git/personal/discard/target` throughout.

- Provider unit tests: 63 passed.
- Claude process tests: 14 passed.
- Codex process tests: 20 passed.
- Capability fixture tests: 2 passed.
- Existing engine integration tests: 28 passed.
- Both retained installed-binary synthetic allowlist probes: passed.
- `cargo check --workspace`: passed.
- `cargo test --workspace --no-run`: passed for all test targets.
- `git diff --check`: passed.

Provider docs describe setup requirements, supported controls, exact-mode behavior, trusted provider policy and evidence limits. No frontend, runtime context builder, memory service or new engine behavior was implemented in this task.

The shared installed cache briefly reported another client version during one final probe run. The version check rejected it. A later read reported 0.166.0 and the retained probe passed. Concurrent provider cache refreshes can therefore produce a visible setup error; Discard does not alter the cache to bypass that check.
