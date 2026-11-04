# Discard

An agent harness for managing projects and several agents, with an interface modeled on the Discord desktop
client. The interface and behavior are Lua plugins. Rust and `gpui-kit`. macOS first.

## Run

```
cargo run -p discard-app                      # Claude in this directory
cargo run -p discard-app -- --provider codex  # Codex
cargo run -p discard-app -- --builder         # the Builder agent, which can write plugins for you to review
cargo run -p discard-app -- --safe-mode       # no user plugins
```

Sign in with the official CLIs first: `claude auth login`, `codex login`. Discard uses your subscription through
those CLIs and never handles credentials. Data lives in `~/.discard` (set `DISCARD_HOME` to change it).

## Test

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Live tests use the real CLIs and your subscription. They are ignored by default:

```
cargo test -p discard-providers --test claude_live    -- --ignored
cargo test -p discard-providers --test codex_live     -- --ignored
cargo test -p discard-providers --test memory_live    -- --ignored
cargo test -p discard-providers --test plugin_live    -- --ignored
cargo test -p discard-providers --test worktree_live  -- --ignored
```

## Layout

| Crate | Role |
| --- | --- |
| `discard-core` | Events, turn and approval state machines, storage, memory, worktrees, engine. No GPUI. |
| `discard-providers` | Claude and Codex providers, and a scripted fake for tests. |
| `discard-lua` | Luau plugin host, theme tokens, drafts, review, approval ledger. |
| `discard-mcp` | Local MCP server: memory tools and plugin-authoring tools. |
| `discard-ui` | GPUI views. |
| `discard-app` | The binary. |

## Docs

- `PLAN.md`: the design and the milestone list.
- `docs/provider-claude.md`, `docs/provider-codex.md`: what was learned about each CLI.
- `docs/memory.md`, `docs/plugins.md`, `docs/worktrees.md`: how each part works and its limits.
- `docs/settings.md`: the settings file, the menu bar, key bindings, and the Settings window.
- `docs/ui-juice-spec.md`: the UI polish spec (surfaces 1 to 5 and the chrome changes).
