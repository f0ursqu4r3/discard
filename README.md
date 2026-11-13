# Discard

An agent harness for managing projects and several agents, with an interface modeled on the Discord desktop
client. The interface and behavior are Lua plugins. Rust, Tauri 2, and a Vue 3 front end with shadcn-vue. macOS first; Linux is supported with the limits below.

## Run

```
cargo run -p discard-app                      # Claude in this directory
cargo run -p discard-app -- --provider codex  # Codex
cargo run -p discard-app -- --builder         # the Builder agent, which can write plugins for you to review
cargo run -p discard-app -- --safe-mode       # no user plugins
```

On Linux, install the Tauri system packages first. On Debian or Ubuntu:

```
sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libxdo-dev
```

Sign in with the official CLIs first: `claude auth login`, `codex login`. Discard uses your subscription through
those CLIs and never handles credentials. Data lives in `~/.discard` (set `DISCARD_HOME` to change it).

The web app uses Vite+ for development, builds, tests, linting, and formatting.
Install dependencies with `pnpm --dir web install --frozen-lockfile`. No global Vite+ install is required.

For front end work, run the Vite+ dev server with hot reload:

```
(cd crates/discard-app && cargo tauri dev)      # needs `cargo install tauri-cli --version '^2'`
pnpm --dir web dev                               # web only, in a browser
cargo run -p discard-app --no-default-features   # app window against a running Vite dev server
```

A plain `cargo run` embeds the built front end from `web/dist`. It builds `web/` first when `web/dist` is missing.

## Linux limits

- Agents cannot write to a workspace. Claude workspace sessions and Codex autorun were checked on macOS only, so
  they fail closed on Linux and the session is read-only.
- The plugin signing key lives in the Secret Service (GNOME Keyring, KWallet). With no Secret Service, set
  `DISCARD_PLUGIN_PASSPHRASE`.
- The menu bar has no Services, Hide, Hide Others, Show All, Undo, or Redo entries.
- `cmd` in a shortcut is the Control key. Close Window is Ctrl+Shift+W and Minimize has no key, so the shell in the
  terminal keeps Ctrl+W, Ctrl+M, and Ctrl+H.

## Test

```
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
pnpm --dir web check                         # format, lint, and Vue type checks
pnpm --dir web test
pnpm --dir web build
```

Use `pnpm --dir web fmt` to format the web app. Formatting and Oxlint settings live in
`web/vite.config.ts`. ESLint remains enabled for Vue rules that Oxlint does not support.
`vue-tsc` checks Vue templates; the build runs it before bundling.

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
| `discard-core` | Events, turn and approval state machines, storage, memory, worktrees, engine. No GUI. |
| `discard-providers` | Claude and Codex providers, and a scripted fake for tests. |
| `discard-lua` | Luau plugin host, theme tokens, drafts, review, approval ledger. |
| `discard-mcp` | Local MCP server: memory tools and plugin-authoring tools. |
| `discard-app` | The Tauri app. The `desktop` module bridges the engine to the webview: snapshots out, commands in. |
| `web/` | The Vue 3 front end: main window, Settings window, shadcn-vue components, plugin element renderer. |

## Docs

- `PLAN.md`: the design and the milestone list.
- `docs/provider-claude.md`, `docs/provider-codex.md`: what was learned about each CLI.
- `docs/memory.md`, `docs/plugins.md`, `docs/worktrees.md`: how each part works and its limits.
- `docs/settings.md`: the settings file, the menu bar, key bindings, and the Settings window.
- `docs/attachments.md`: message attachments, previews, supported files, and storage limits.
- `docs/ui-juice-spec.md`: the UI polish spec (surfaces 1 to 5 and the chrome changes).
- `docs/runtime/conversation-context.md`: conversation context, project channels, and topics.
- `docs/project-channel-status.md`: implemented project-channel behavior and the remaining orchestration roadmap.

- `docs/superpowers/specs/2026-11-09-agent-workspace-writes-design.md`: agent workspace writes; slice A is implemented, slices B and C remain planned.
