# Worktrees

Coding sessions that use a worktree run outside the project checkout, under `$DISCARD_HOME/worktrees/` (default
`~/.discard/worktrees/`). This keeps those sessions from overwriting the project checkout.

## Rules

- **Created before a Coding session starts.** When a channel uses a worktree, Discard creates it before starting
  the provider session. A running session is never moved to another directory.
- **Serialized per repository.** Parallel channels create their worktrees one at a time, so git's locks do not
  collide.
- **Branches from `HEAD`.** Each worktree gets a branch `discard/<channel>-<id>` that starts at the repository's
  current `HEAD`. Uncommitted and untracked files in the main checkout are **not** copied. Discard shows a notice
  that lists them.
- **No hooks.** Discard turns off the repository's git hooks for its own worktree commands.
- **Failure stops the turn.** If the worktree cannot be created (no commits, not a repository, a git error), the
  turn does not start and the error appears in the channel. A channel set to use a worktree never falls back to
  the main checkout on its own.
- **Idempotent.** Asking again returns the existing worktree. A worktree directory deleted by hand is recreated
  on the next turn, and the kept branch is reused.

## Choosing the mode

- New channels in a git repository use a worktree by default. Other projects use the main checkout.
- `discard --worktree` or `discard --main-checkout` chooses explicitly.
- Channels from before worktree support stay in the main checkout. The default never moves them.
- Changing an existing channel's mode starts a **new session**, because the provider's old session belongs to the
  old directory. The new session does not carry over the old conversation. A "confirm a summary first" screen is
  not built yet.
- Project channels are durable conversations associated with `general` or a canonical branch. Opening a project
  or branch channel restores its conversation; it does not switch the Git checkout or create a worktree. Normal
  Conversation mode still uses an empty provider working directory. Worktree creation and branch mutation remain
  separate operations.

## Branch list

The channel list shows local branches in a two-level tree. The main checkout and other worktrees are top-level
rows. A branch without a worktree appears below the worktree whose branch tip is its closest ancestor. If no
worktree branch is an ancestor, it appears below the main checkout. Ties go to the main checkout, then the
worktree branch with the first name alphabetically.

A filled marker shows a row with a worktree. Rows without a worktree are dimmed. The current channel's worktree
row is highlighted, and the main checkout has a `main` tag. Click a parent row to collapse or expand its
branches. Click a branch row to open its canonical project conversation. If it has no canonical conversation,
Discard creates one. Opening the conversation does not switch the Git checkout or create a worktree.

Detached worktrees are selectable. Each detached worktree opens a saved conversation bound to its repository,
path, and displayed commit. Two worktrees at the same commit have separate conversations. Opening a detached
worktree does not create a branch or change the checkout. Discard borrows the existing directory. If the checkout
changes, a stale row cannot open it and Coding workspace validation rejects the old binding.

Branch, worktree, and channel rows have custom context menus. Right-click a row, or press Shift+F10 while it has
focus. Menus open the selected row and copy its name or ID. Worktree menus also copy the directory path.
Unread markers on detached worktrees match the saved path and commit, so worktrees at the same commit do not
share an unread marker.

A branch row menu has **Create Worktree…**, **Rename Branch…**, and **Delete Branch…**. A worktree row menu has
**Create Branch…** and **Remove Worktree…**. Each action asks for a confirm or a name first. Discard runs Git
with an argument list, checks the rules again in the backend, and refreshes the branch list afterwards.

- **Create Worktree…** calls the same service as the agent request: the worktree gets the path of an owned
  worktree and a saved record. It is greyed when a working tree holds the branch or the channel has a worktree.
- **Rename Branch…** works for a checked-out branch and moves the channel binding. It is greyed while the channel
  is busy.
- **Delete Branch…** warns about commits that the default branch does not have. You must choose to delete them
  before Discard runs `git branch -D`. It is greyed for the default branch, for a checked-out branch, and while
  the channel is busy.
- **Create Branch…** switches the working tree to a new branch at its HEAD. Uncommitted changes stay.
- **Remove Worktree…** lists up to ten changed or untracked files and asks you to choose **Discard these
  changes** before it removes the tree. It never deletes the branch. It is greyed for the main checkout and while
  a channel that uses the tree is busy.

A channel is busy while a turn runs, a sub-agent task runs, an approval waits, or an archive runs. A greyed
action shows no **Soon** label. See `docs/settings.md` for the full rules.

## Archiving

`GitWorkspace::archive` removes a channel's worktree only if all of these are true: no uncommitted changes, no
untracked files, no ignored files that hold data. Otherwise it returns what would be lost and changes nothing. A
confirmed call (`force`) removes the directory anyway. **The branch is always kept**, so committed work stays in
git. Commits that no other branch has are reported but do not block removal, because they stay on the kept branch.

**Archive Channel…** in the menu bar and in the row menu of a channel call this function. The row item works on
the channel of the row, is greyed for `#general`, for an archived channel, and for a busy channel, and asks for a
second confirm when the removal would lose files.

## Limits you should know

- Worktrees are a git feature. A project that is not a git repository uses the main checkout.
- Submodules and sparse checkouts are not handled specially.
- Discard does not merge the worktree branches. Archiving and **Remove Worktree…** keep the branch. **Delete
  Branch…** deletes a local branch only after you confirm it.

## Conversation worktree tools

Project conversations with a saved worktree binding expose `worktree_inspect` and `worktree_request_removal` through the scoped Discard MCP server. The tools use the token's conversation and project. They accept no path, shell command, force flag, or approval argument. Direct chats and summary tokens cannot use these tools.

Inspection reads status (including ignored and untracked paths), tracked working changes, or the current commit patch. Each Git command has a ten-second limit and a 32 KiB output limit. Incomplete inspection fails closed. Hooks, external diff programs, text conversion, and interactive Git prompts are disabled.

Removal creates an app approval card. The request does not remove files. Only the user can approve or deny it in the app. Approval is single-use, expires after ten minutes, and is cancelled by an app restart. Pending cards are session state; completed decisions are saved as system notes in the conversation.

Before removal, Discard waits for app responses and archive operations to finish, resolves the saved binding again, and compares the current Git state with the approved snapshot. Main checkouts, locked worktrees, dirty files, untracked files, ignored files, and submodule configurations block removal. Git runs without `--force`. If no local branch or tag contains the commit, the card includes creation of a recovery branch under `discard/retained-worktrees/`. Discard creates that branch before removal. A failed removal can leave this recovery branch in place; the result reports it.

The app serializes these tool operations. External Git processes are not controlled by Discard. Git performs its own final removal checks. Removing a worktree keeps the conversation history available.

## Agent workspace writes (slices A and B)

An agent with **Edit files in branch channels** enabled can answer in a bound project branch or detached worktree channel using `SessionMode::Workspace`. The provider runs in that directory with a small native tool set and OS sandbox. General channels, direct chats, unbound branches, main checkouts without a one-response grant, disabled agents, and unavailable sandbox profiles stay read-only. Discard states the reason. Root `AGENTS.md` and `CLAUDE.md` enter the prompt as bounded, untrusted repository instructions.

In-root file edits receive automatic approval. Shell requests use approval cards. Sandbox escapes require a separate full-access card with no Always allow action. Codex cannot reliably identify command escalation, so every Codex Workspace command approval uses the full-access card. Durable Always-allow rules support simple shell-command prefixes and exact network hosts. The user chooses the prefix cut on the approval card. The default includes the program and the first word after it that is not a flag. Compound shell syntax cannot use Always allow. Rules belong to one agent in one project. Each automatic approval records the rule ID. Project settings list rules and offer Revoke. Revocation affects the next request, including one in a running response. Codex command cards state "Always allow is unavailable for Codex write sessions."

Sandbox autorun is a per-agent setting, **Run sandboxed commands without asking**. It needs **Edit files in branch channels** and applies only in linked and detached worktrees. Main-checkout grants keep per-command approval. Turning it on needs the user's confirmation. With autorun, shell commands inside the OS sandbox run without a card. Sandbox escapes, network domains, edits to Git metadata or outside paths, and the other Discard cards still ask. Autorun never approves them.

Autorun has limits. The sandbox limits writes and network access, not reads. Claude denies reads of known credential paths: `~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.gnupg`, `~/.netrc`, `~/.docker/config.json`, the Claude and Codex config paths, and Discard files outside the session root. Symlinks in these paths are resolved: the real target of a symlinked entry, and of a symlink found up to 4 levels inside a credential directory, is denied too. A symlink that leads into the session root is not denied. The agent can still read other files that the user's account can read. Native `Read` and `Glob` of a denied path fail. Native `Grep` outside the session root fails because `--restricted` confines the file tools to the root. Claude's own `projects/<session root>` directory stays readable, so the agent can read back large tool outputs. The list of denied entries is built when the session starts. Entries created later in the Discard data directory or the Claude config directory, such as another session's transcript directory, are not denied. Claude's sandbox always allows the Keychain service. Discard denies `Bash(security:*)` and removes `GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN`, and `GITHUB_ENTERPRISE_TOKEN` from the Claude process. A script can still call the Keychain directly. Do not turn on autorun if your Keychain holds tokens that the agent must not see. Autorun turns off Git credential helpers and Git prompts, so Git itself does not read the Keychain. Claude loads no settings files from the worktree, so an agent cannot widen the sandbox through `.claude/settings.json`. A prompt injection in repository files can make the agent run commands that nobody sees first. The sandbox, the read denials, and the network block contain the effect. The changes card shows every change before the user pushes. Build scripts and test runners run repository code under the same limits. Codex also supports autorun, on macOS and in linked worktrees without nested `.git` paths only. An autorun session can still create a nested `.git` directory with no card. For every provider, Discard walks the worktree before and after each autorun response. It warns in the channel when the response created nested `.git` paths, and it shows a notice when a Claude autorun response starts in a worktree that already has them. The warning does not block, revert, or remove them. See [Codex](provider-codex.md#nested-git-paths). A Codex autorun session uses `approvalPolicy: "on-request"`. Codex then runs sandboxed commands without a card and asks only when the model requests an escalation. Every Codex command approval in that mode is an escalation. It uses the full-access card with no Always allow action. Codex has no read denials: the sandbox does not limit what a Codex command can read, including credential paths. A Codex file-change request that retries without the sandbox also gets the full-access card. The agent settings page shows the toggle disabled with the provider reason when a provider cannot run autorun. The reason is a state block, not planned work. See [Codex](provider-codex.md#autorun-spike-results).

Autorun is visible in three places. The channel's context panel shows an "Autorun" badge while autorun applies. It shows "Autorun off: <reason>" when the agent has the setting but the provider or channel cannot use it. Shell tool calls that ran without a card show "Ran in sandbox". Each autorun response also gets an audit row with the agent, provider, response, and session root. Commands that ran without a card have no approval rows.

One response at a time holds a canonical worktree path. Other responses wait and show the current channel. Cancellation, setup failure, and provider failure release the lock. This lock covers Discard responses; it does not control external editors or Git processes.

Workspace tokens expose `worktree_commit` with a commit message and optional relative paths. The tool rechecks the agent, channel, worktree binding, and response lock. It runs outside the provider sandbox with hooks disabled. It preserves the user’s Git identity and signing settings. It rejects main checkouts without an active grant, detached HEAD, traversal, and paths outside the root. Selected-path commits exclude unrelated staged files. Commit audit records link to the saved response. Conversation and summary tokens do not expose this tool.

Each Workspace response ends with a protected changes card, including stopped and failed responses. Discard draws the card, so plugins cannot change it, but it uses the plain card colors: it reports and asks for nothing. The Changes tab of the channel lists the same cards. The card lists commits and changed files with expandable, bounded diffs. It excludes unchanged pre-existing dirty files and labels pre-existing files that changed. A collection failure saves a visible notice and releases the lock.

Slice B is implemented. An opted-in agent with policy Ask can call
`worktree_request_creation` in an unbound branch channel. The card shows the branch
and target folder. Approval rechecks the branch and creates an owned worktree.
A checked-out or deleted branch, or an existing target folder, prevents creation.
Denial creates nothing. Approval starts `Command::Continue` for the requesting
agent, with current eligibility and no new user message.

A Claude agent in a branch channel bound to the main checkout can call
`main_checkout_request_write`. Approval grants writes to one agent in one channel
for one response. A grant expires after ten minutes and cannot cross channels.
Commits require the same response grant. Claude's `denyWrite` blocks shell writes
to `.git`. Native file-tool edits under `.git` require the full-access card.

Codex agents cannot request or receive main-checkout writes. The spike showed
that approved Codex commands and patches can change `.git`. The MCP server hides
the request tool and the prompt tells the agent to explain this limit once if the
user asks for changes. The engine rechecks provider support before using a grant.
Codex still refuses Workspace roots with a `.git` directory.

Both request types require app approval. They are single-use, expire after ten
minutes, and are cancelled by app restart. The agent, plugins, and Always-allow
rules cannot approve them. The user must wait for the current response to finish
before approving.

Slices A and B do not change the `--builder` Coding path.

## Conversation moves (slice C)

The project's Primary / Moderator agent (the project main agent) can ask to move a
conversation. It calls `conversation_request_move`. No other agent has this tool.
Discard checks the agent when it lists the tool, when the agent calls it, and again
at approval. Other agents can suggest a move in text.

The call names one target:

- **Existing channel.** The agent gives the channel name.
- **New branch.** The agent gives the branch name and, optionally, a base branch.
  The default is the default branch. Discard creates the new branch in the
  repository of the source channel.

The call also gives a handoff. The handoff must stand alone, because Discard does
not copy the source transcript. It has these fields and limits:

- `goal` is required. It has at most 600 characters.
- `constraints`, `decisions`, and `open_questions` are optional lists. Each list
  has at most 12 items. Each item has at most 400 characters.
- The whole handoff has at most 4096 bytes.

The request uses the write-request store of slice B. It expires after ten minutes,
is single-use, and is cancelled by an app restart. Only the user approves. The
protected card appears in the source channel and shows the target and the handoff as
plain text.

Approval runs these steps in order:

1. Discard checks the moderator gate, the source message, and the target.
2. For a new branch, Discard creates the branch at the base commit shown on the card.
   It then creates an owned worktree and a branch channel. The handoff becomes the
   topic of the channel. The worktree record keeps the branch name and the base
   commit.
3. In the target, Discard posts a "Moved from" link. It then posts the handoff as a
   message authored by the moderator.
4. Discard sends the user's latest message, with its attachments, to the target as a
   normal user message. This starts a response in the target.
5. In the source, Discard posts a "Moved to" link.

Discard does not copy the source transcript. The user's view stays in the source
channel. No response starts in the source.

Discard refuses approval while a response runs in the source. It also refuses
approval when the target is busy (a running turn or a waiting approval). In both
cases the request stays pending, and the user can approve again before the request
expires.

Two more refusals fail the move:

- The user sent another message in the source after the request. The card moves
  the latest message, so the agent must ask again.
- The base commit of a new branch no longer exists. The agent must ask again.

In both cases the request fails and the source gets a note.

When a step fails, the source gets the note "Conversation move failed: <error>". If
Discard cannot create the worktree, it deletes the new branch again. No branch or
channel remains. Denial adds the note "Conversation move denied." to the source.
Nothing else changes.

Not built:

- "Create a branch here" for detached worktrees (spec decision 17). It needs a
  different gate.
- A move to an existing branch that has no channel.

Native provider resume per agent is also planned.
