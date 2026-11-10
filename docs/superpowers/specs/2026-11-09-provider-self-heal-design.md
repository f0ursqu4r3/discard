# Provider self-heal: CLI version drift and unreachable API address

Date: 2026-11-09. Status: frozen for build.

## Why

Two failures stopped real use on one day:

1. The Claude CLI auto-updated from 2.1.318 to 2.1.321. Every chat then failed with "Claude Code 2.1.318 is required for the verified conversation controls". The same exact pin exists for Codex 0.166.0.
2. `ANTHROPIC_BASE_URL` pointed to a local proxy that was not running. The Claude CLI retried without output, and the request waited for the full timeout.

## Decisions (made by the user)

- **Version drift: verify, then accept.** A CLI newer than the tested version may run when every required isolation control is verified. Block only when a control is missing or verification fails. Show a notice with the tested and running versions.
- **Unreachable API address: fail fast with the cause.** Do not change where requests go.

## Decisions (spec author's calls)

- A CLI **older** than the tested version stays blocked. The message tells the user to update the CLI.
- An unparseable version stays blocked.
- The notice appears in the conversation as a marker, like the existing "Model changed to …" marker, at most once per engine instance per distinct text.
- The reachability check covers Claude only, and only the `ANTHROPIC_BASE_URL` environment variable.

## Part 1 — Version gate

### Shared

Add a small pure helper in `discard-providers` (for example `version.rs`):

- Parse `major.minor.patch` from `--version` output (`"2.1.321 (Claude Code)"`, `"codex-cli 0.166.0"`).
- `enum VersionCheck { Tested, Newer { running: String }, Older { running: String }, Unknown }` from comparing with the tested version.
- Constants for the tested versions: Claude `2.1.318`, Codex `0.166.0`. Replace the scattered string literals in `src/` with these constants.
- Unit tests: equal, newer patch/minor/major, older, garbage, extra text.

### Claude (`claude/provider.rs`, conversation and workspace modes)

- `Tested` → current behavior.
- `Newer` → continue to the control checks. The control check must be stricter than today: every `--flag` token that `build_args` emits for this spec must appear in `claude --help`. Derive the list from the built arguments, so the two cannot drift. Keep the current fixed list as a minimum. Keep `check_claude_managed_files`.
- `Older` → `Setup` error: "Claude Code {running} is older than the tested version {tested}. Update Claude Code, then retry."
- `Unknown` or a failed `--version` → `Setup` error that says the version could not be read.
- A missing control → `Setup` error that names the missing flag and both versions.
- `claude/text.rs` (text generation) has no version gate today. Leave it as is.

### Codex (`codex/provider.rs`, `codex/text.rs`, `conversation.rs`)

- Same `Tested` / `Newer` / `Older` / `Unknown` rule for the `--version` gate in conversation, workspace, and text generation.
- Model cache: `sanitize_catalog` and `load_catalog` currently require `client_version == "0.166.0"`. Change them to require the cache version to equal the **running** CLI version. Pass the running version in. Keep the existing bounded cache refresh/recovery path; it must refresh a cache written by another version. Update the error text to name the running version.
- The existing effective-configuration checks (`codex_controls`, `validate_controls`, disabled inherited MCP servers) are the verification for a newer version. Do not weaken them. A failure there on a newer version must produce an error that names both versions.

### Notice

- Add to `ProviderSession` (in `discard-core/src/provider.rs`): `fn notice(&self) -> Option<String> { None }`.
- Claude and Codex sessions return, on `Newer` only: "{CLI name} {running} is newer than the tested version {tested}. Discard verified the isolation controls before this response."
- `FakeProvider` gets a builder to set a notice for tests.
- The engine (`discard-core/src/engine.rs`) writes the notice with the existing `marker(...)` mechanism after a session starts. It writes the same text at most once per engine instance. A notice must not fail or delay the turn.

## Part 2 — Unreachable API address (Claude)

- Move `api_address` from `claude/text.rs` to a place both paths can use. Keep `explain_timeout` as the fallback.
- New async check: if `ANTHROPIC_BASE_URL` is set (provider env override first, then process env; an explicit removal counts as unset) and parses to a URL with a host, open a TCP connection to host and port (default 443 for `https`, 80 for `http`) with a 2 s timeout. Do not send data.
- Failure → `ProviderError::Setup("Claude cannot reach the API address {origin} (ANTHROPIC_BASE_URL). Start that service or remove the variable, then retry.")`. `{origin}` is scheme, host, and port only.
- Unset or unparseable → skip the check.
- Run the check in `ClaudeProvider::start_session` (all modes) and `generate_text`, before the CLI process starts.
- Tests: unreachable address (bind a listener, take its port, drop it) fails in under 3 s with the message; reachable local listener passes; unset skips; the message never contains path or user information.

## Tests to change

- `tests/codex_process.rs` asserts "Codex 0.166.0 is required". Replace that assertion with the new behavior: a newer fake version passes when controls verify and exposes the notice; an older fake version fails with the update message. `fixtures/fake-codex.py` already reads `FAKE_CODEX_VERSION`.
- `conversation.rs` cache-version unit tests follow the new rule.
- Add Claude tests with a fake CLI script in the style of the existing tests: newer version with all flags → starts; newer version with one flag missing from `--help` → `Setup` error naming the flag; older → update message.
- Engine test with `FakeProvider`: the notice marker appears once across two turns.
- Do not change the opt-in probes in `tests/probes/`.

## Docs

Update `docs/provider-claude.md` and `docs/provider-codex.md`: replace "requires exactly" wording with the tested-version rule, the control verification, the notice, and (Claude) the reachability check. State the residual risk: a newer CLI can change the behavior of a control without removing it.

## Non-goals

- No automatic CLI install, downgrade, or update.
- No fallback to another API address. No change to environment variables.
- No change to the settings provider cards.
- No reachability check for Codex, proxies (`HTTPS_PROXY`), or settings-file values.

## Constraints

- Follow `AGENTS.md`. Do not commit. No dependency changes unless the build fails without them (report it).
- The tree already holds uncommitted personality-generation work. Keep it working; do not revert or reformat it.
- Fail closed: any verification error blocks the session. Never skip a control check because the version is newer.
- Do not touch `.impeccable/`, `PLAN.md`, `.claude/`. Do not run `--ignored` live tests.

## Proof

```bash
cargo clippy --workspace --all-targets && cargo test --workspace && pnpm --dir web check && pnpm --dir web test
```
