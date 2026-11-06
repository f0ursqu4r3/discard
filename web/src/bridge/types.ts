export type BlockDto =
  | { type: 'markdown' | 'diff'; text: string }
  | { type: 'code'; language: string | null; text: string }
  | { type: 'tool_call'; call_id: string; name: string; input: unknown; output: string | null }
  | { type: 'approval'; approval_id: string; tool: string; summary: string }
export interface MessageDto {
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
export interface PromptDto {
  approvalId: string
  tool: string
  summary: string
  view: string
}
export interface BranchRowDto {
  key: string
  label: string
  kind: string
  depth: number
  current: boolean
  collapsed: boolean
  hasChildren: boolean
  detached?: boolean
}
export interface ThemeDto {
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
  agentStatus: string
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
