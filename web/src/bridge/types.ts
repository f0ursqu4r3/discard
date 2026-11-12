export interface AttachmentDto {
  id: string
  name: string
  mediaType: string
  size: number
  text: string | null
}
export type BlockDto =
  | {
      type: 'changes'
      root: string
      state: 'completed' | 'stopped' | 'failed'
      commits: { id: string; subject: string }[]
      files: {
        path: string
        status: string
        diff: string
        truncated: boolean
        preexisting: boolean
      }[]
      truncated: boolean
    }
  | { type: 'attachment'; attachment: AttachmentDto }
  | { type: 'markdown' | 'diff'; text: string }
  | { type: 'code'; language: string | null; text: string }
  | {
      type: 'tool_call'
      call_id: string
      name: string
      input: unknown
      output: string | null
      is_error?: boolean
      sandboxed?: boolean
    }
  | { type: 'approval'; kind?: string; approval_id: string; tool: string; summary: string }
export interface MessageAuthorDto {
  agentId: string
  name: string
  provider: string
  avatar: string | null
}
/** The newest user or agent message of a thread. */
export interface ThreadLastMessageDto {
  role: string
  /** The agent that wrote it. Null for a user message. */
  authorName: string | null
  /** Plain text on one line, cut at 80 characters. Empty when the message has no text. */
  preview: string
}
/** Why a thread exists. A `sub_agent` thread is a read-only research run that an agent started with `delegate`. */
export type ThreadKind = 'question' | 'sub_agent'
/** The run state of a sub-agent thread. `running` is the only state that changes; the others are final. */
export type ThreadState = 'running' | 'done' | 'failed' | 'cancelled' | 'timed_out'
/**
 * The chip of a thread. A question thread chip sits under its anchor message (`MessageDto.thread`). The chips of
 * sub-agent threads sit under the response that started them (`MessageDto.subAgentThreads`), and while that turn
 * runs they come from `SnapshotDto.liveSubAgentThreads`.
 */
export interface ThreadChipDto {
  /** The thread channel ID. Pass it to `openThread`, `followThread` and `leaveThread`. */
  channelId: string
  name: string
  /** User and agent messages in the thread. */
  messageCount: number
  lastMessage: ThreadLastMessageDto | null
  followed: boolean
  unread: boolean
  kind: ThreadKind
  /** Null for a question thread. */
  state: ThreadState | null
  /** The thread's agent: the sub-agent of a sub-agent thread, the default responder of a question thread. */
  agentName: string
  /** Approvals that wait in the thread. */
  pendingApprovals: number
}
/** A followed thread in the channel bar, indented under its parent channel. */
export interface ThreadLinkDto {
  id: string
  parentChannelId: string
  name: string
  unread: boolean
  running: boolean
  pendingApprovals: number
  kind: ThreadKind
  /** Null for a question thread. */
  state: ThreadState | null
}
/** A thread with an approval that waits, followed or not. The Approvals page lists these. */
export interface ThreadApprovalDto {
  threadId: string
  parentChannelId: string
  name: string
  kind: ThreadKind
  agentName: string
  pendingApprovals: number
}
/** Where a reference row in a parent channel comes from. */
export interface PostedFromDto {
  sourceMessageId: string
  threadChannelId: string
  threadName: string
}
export interface MessageDto {
  /** Unix milliseconds; absent for messages without a recorded creation time. */
  createdAt?: number | null
  author?: MessageAuthorDto | null
  id: string
  role: string
  blocks: BlockDto[]
  continuesGroup: boolean
  /** The question thread that starts from this message. Set only on a message of a normal channel. */
  thread?: ThreadChipDto | null
  /** The sub-agent threads of this response, in task order. Empty on any other message. */
  subAgentThreads?: ThreadChipDto[]
  /** Set on a reference row: the thread message it shows ("from thread"). */
  postedFrom?: PostedFromDto | null
  /** Set on a thread message that is also posted to the parent channel. */
  postedToChannel?: boolean
}
export interface HistoryDto {
  channelId: string
  rev: number
  messages: MessageDto[]
}
/** The transcript of the open thread. */
export interface ThreadHistoryDto {
  /** The thread channel ID. */
  channelId: string
  parentChannelId: string
  rev: number
  /** The message the thread starts from. Null when the parent transcript no longer holds it. */
  anchor: MessageDto | null
  messages: MessageDto[]
}
/** The thread panel: one open thread beside the selected channel. */
export interface OpenThreadDto {
  channelId: string
  parentChannelId: string
  name: string
  followed: boolean
  kind: ThreadKind
  /** The run state of a sub-agent thread. Null for a question thread. */
  state: ThreadState | null
  /** The thread's agent name (the sub-agent of a sub-agent thread). */
  agentName: string
  /** Null for a sub-agent thread whose parent turn still runs. */
  anchorMessageId: string | null
  /** The revision of the transcript this snapshot goes with. Hold the `ThreadHistoryDto` whose `rev` is at least this. */
  historyRev: number
  agentId: string
  agent: string
  provider: string
  responder?: MessageAuthorDto | null
  agentStatus: string
  turnRunning: boolean
  notice: string | null
  liveBlocks: BlockDto[]
  prompts: PromptDto[]
  pendingApprovals: number
  unread: boolean
  composerDisabled: boolean
  /**
   * Why the composer is off: `read_only` when the thread, its agent or its channel is archived or unavailable.
   * Null when the composer is on. A sub-agent thread has its composer on.
   */
  composerDisabledReason: 'read_only' | null
  /** Says that threads are read-only. Always present. */
  readOnlyNote: string
}
export interface ChannelDto {
  agentId: string
  projectId: string | null
  id: string
  name: string
  agent: string
  provider: string
  workspaceMode: string
  running: boolean
  /** An agent message follows the read marker. Always false for a muted channel. */
  unread: boolean
  /** A muted channel shows no unread marker and counts for no unread marker of its project. */
  muted?: boolean
  pendingApprovals: number
  /** Approvals that wait in the threads of this channel, followed or not. `pendingApprovals` counts the channel only. The app always sends it. */
  threadPendingApprovals?: number
  /** Followed threads of this channel with live state, oldest first. */
  threads?: ThreadLinkDto[]
}
export interface ProjectChannelDto {
  worktree?: { path: string; branch: string } | null
  repository?: string
  id: string
  projectId: string
  name: string
  agentId: string
  workspaceMode: string
  kind: 'general' | 'text' | 'branch' | 'archived_legacy' | 'legacy'
  branch: string | null
  archived: boolean
  archivedInto: string | null
  /** The channel, or a thread of the channel, has a saved message. */
  hasMessages: boolean
  /** The channel is muted. The app always sends it. */
  muted?: boolean
  /** Followed threads of this channel, from storage. */
  threads?: ThreadLinkDto[]
}
export interface ChannelTopicDto {
  channelId: string
  text: string
  version: number
}
export interface ChannelTopicRevisionDto {
  channelId: string
  text: string
  version: number
  authorKind: string
  authorId: string | null
}
export interface ChannelMemberDto {
  id: string
  name: string
  provider: string
  archived: boolean
}
export interface ChannelDetailsDto {
  channel: ProjectChannelDto
  topic: ChannelTopicDto
  topicHistory: ChannelTopicRevisionDto[]
  members: ChannelMemberDto[]
  mainAgentId: string | null
}
export interface PromptDto {
  kind: string
  alwaysAllow: string
  ruleWords?: string[] | null
  defaultCut?: number | null
  ruleHost?: string | null
  alwaysAllowNote?: string | null
  approvalId: string
  tool: string
  summary: string
  view: string
}
export interface RepositoryDto {
  id: string
  label: string
  path: string
}
export interface BranchRowDto {
  repository?: string
  key: string
  label: string
  kind: string
  depth: number
  current: boolean
  collapsed: boolean
  hasChildren: boolean
  detached?: boolean
  worktree?: string | null
}
export interface ThemeDto {
  monoFont?: string | null
  mode: string
  tokens: Record<string, string>
  radius: number | null
  fontScale: number
  spacingScale: number
  protectedBg: string
  protectedFg: string
  protectedBorder: string
  diffColors?: { added: string; removed: string; inlineAdded: string; inlineRemoved: string }
  /** The integrated terminal colors. An empty palette means the web keeps its fallback. */
  terminal?: TerminalColorsDto
}
export interface TerminalColorsDto {
  /** ANSI colors 0 through 15. */
  palette: string[]
  background: string
  foreground: string
  cursor: string
  cursorAccent: string
  selectionBackground: string
  selectionForeground: string | null
}
export interface ElementDto {
  tag: string
  props: Record<string, boolean | number | string>
  children: ({ Text: string } | { Element: ElementDto })[]
}
export interface SlotDto {
  plugin: string
  element: ElementDto
}
export interface BannersDto {
  reviewCount: number
  reviewNames?: string[]
  problemLines: string[]
  confirm: string | null
  archive: { message: string } | null
  safeMode: boolean
}
export interface SnapshotDto {
  activeProjectId: string | null
  composerDisabled: boolean
  rev: number
  historyRev: number
  memoryRev: number
  project: string
  projectId: string
  projectPath: string
  projectDisplayPath: string
  channelId: string
  agentId: string
  channel: string
  agent: string
  provider: string
  responder?: MessageAuthorDto | null
  agentStatus: string
  autorun: { state: 'on' | 'unavailable'; reason: string | null } | null
  turnRunning: boolean
  notice: string | null
  liveBlocks: BlockDto[]
  prompts: PromptDto[]
  pendingApprovals: number
  unread: boolean
  newAnchorId: string | null
  archived: boolean
  archiveBusy: boolean
  memberListVisible: boolean
  panels: { channelList: number; memberList: number }
  branches: BranchRowDto[]
  repositories?: RepositoryDto[]
  repositoriesLoading?: boolean
  channels: ChannelDto[]
  /**
   * The sub-agent threads of the running turn of the selected channel, in task order. They have no anchor yet.
   * When the turn ends they move to `MessageDto.subAgentThreads` of the saved response. The app always sends it.
   */
  liveSubAgentThreads?: ThreadChipDto[]
  /**
   * Every thread of the selected channel, followed or not, latest activity first. A question thread with no
   * message is not listed. The Threads list reads this. The app always sends it.
   */
  channelThreads?: ThreadChipDto[]
  /** Threads with an approval that waits, followed or not, in the loaded channels. The app always sends it. */
  threadApprovals?: ThreadApprovalDto[]
  /** The thread beside the selected channel, or null. */
  openThread?: OpenThreadDto | null
  slots: Record<string, SlotDto>
  banners: BannersDto
  theme: ThemeDto
}
export type ContextTargetDto =
  | { type: 'message'; id: string; hasText: boolean }
  | { type: 'code_block' | 'diff_block'; messageId: string; block: number }
  | { type: 'tool_call'; messageId: string; block: number; hasOutput: boolean }
  | { type: 'channel'; id: string }
  | { type: 'agent'; id: string; name: string }
  | { type: 'server'; id: string; name: string; path: string }
export type MenuEntryDto =
  | { kind: 'action'; id: string; label: string; enabled: boolean }
  | { kind: 'plugin'; plugin: string; index: number; label: string }
  | { kind: 'separator' }

/** Where Create Worktree… puts the worktree. A branch with no channel gets a channel with this ID. */
export interface WorktreePlanDto {
  path: string
  channelId: string
}
/** `deleted` is false when the branch has unmerged commits and the request did not force the delete. */
export interface BranchDeletionDto {
  deleted: boolean
  unmerged: number
}
/** Files that live only in a worktree directory: the first ten, and the count of the rest. */
export interface WorktreeFilesDto {
  files: string[]
  more: number
}
/** `removed` is false when files would be lost and the request did not force the removal. */
export interface WorktreeRemovalDto extends WorktreeFilesDto {
  removed: boolean
}
/** `archived` is false when the archive needs the second confirm. `items` lists what it deletes. */
export interface ArchiveChannelDto {
  archived: boolean
  items: string[]
}

/** One slash command in the registry. `args` is a hint for the arguments. `planned` commands do nothing yet. */
export interface CommandDto {
  name: string
  description: string
  args: string | null
  source: string
  planned: boolean
}
