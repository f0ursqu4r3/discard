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
export interface MessageDto {
  /** Unix milliseconds; absent for messages without a recorded creation time. */
  createdAt?: number | null
  author?: MessageAuthorDto | null
  id: string
  role: string
  blocks: BlockDto[]
  continuesGroup: boolean
}
export interface HistoryDto {
  channelId: string
  rev: number
  messages: MessageDto[]
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
  unread: boolean
  pendingApprovals: number
}
export interface ProjectChannelDto {
  worktree?: { path: string; branch: string } | null
  repository?: string
  id: string
  projectId: string
  name: string
  agentId: string
  workspaceMode: string
  kind: 'general' | 'branch' | 'archived_legacy' | 'legacy'
  branch: string | null
  archived: boolean
  archivedInto: string | null
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
