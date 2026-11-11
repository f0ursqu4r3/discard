import { invoke } from '@tauri-apps/api/core'
import type {
  AttachmentDto,
  ChannelDetailsDto,
  ChannelMemberDto,
  ChannelTopicDto,
  HistoryDto,
  ProjectChannelDto,
  SnapshotDto,
  ThreadHistoryDto,
  ThreadLinkDto,
} from './types'
export const getSnapshot = () => invoke<SnapshotDto>('get_snapshot')
export const getHistory = () => invoke<HistoryDto>('get_history')
/** Send a message. In a thread, `alsoPost` also posts it to the parent channel. Outside a thread it is an error. */
export const sendMessage = (
  channelId: string,
  text: string,
  attachmentIds: string[] = [],
  alsoPost = false,
) =>
  invoke<void>('send_message', {
    channelId,
    text,
    ...(attachmentIds.length ? { attachmentIds } : {}),
    ...(alsoPost ? { alsoPost } : {}),
  })
/** The open thread's transcript, or null when no thread is open. */
export const getThreadHistory = () => invoke<ThreadHistoryDto | null>('get_thread_history')
/**
 * Start a thread on a saved message of the selected channel, send its first message and open it.
 * No thread exists before this call. Returns the thread channel ID. Rejects when the anchor
 * already has a thread, is not a user or agent message, or is a thread or posted message.
 */
export const startThread = (
  parentChannelId: string,
  anchorMessageId: string,
  text: string,
  attachmentIds: string[] = [],
  alsoPost = false,
) =>
  invoke<string>('start_thread', {
    parentChannelId,
    anchorMessageId,
    text,
    ...(attachmentIds.length ? { attachmentIds } : {}),
    ...(alsoPost ? { alsoPost } : {}),
  })
/** Show a thread of the selected channel beside it. */
export const openThread = (threadId: string) => invoke<void>('open_thread', { threadId })
export const closeThread = () => invoke<void>('close_thread')
/** Select the parent channel of a followed thread and open the thread. Returns the parent channel ID. */
export const selectThread = (threadId: string) => invoke<string>('select_thread', { threadId })
/** Post a thread message (any role) to the parent channel. Idempotent. Returns the reference row ID. */
export const postThreadMessage = (messageId: string) =>
  invoke<string>('post_thread_message', { messageId })
export const followThread = (threadId: string) => invoke<void>('follow_thread', { threadId })
export const leaveThread = (threadId: string) => invoke<void>('leave_thread', { threadId })
export const chooseAttachments = () => invoke<AttachmentDto[]>('choose_attachments')
export async function importAttachmentFiles(files: File[]): Promise<AttachmentDto[]> {
  if (files.length > 8) throw new Error('Attach up to 8 files per message.')
  if (files.some((file) => file.size > 5 * 1024 * 1024))
    throw new Error('Each attachment must be 5 MB or smaller.')
  if (files.reduce((total, file) => total + file.size, 0) > 20 * 1024 * 1024)
    throw new Error('Attachments must total 20 MB or less.')
  const uploads = await Promise.all(
    files.map(
      (file) =>
        new Promise<{ name: string; data: string }>((resolve, reject) => {
          const reader = new FileReader()
          reader.onload = () =>
            resolve({ name: file.name, data: String(reader.result).split(',')[1] ?? '' })
          reader.onerror = reader.onabort = () => reject(new Error(`Could not read ${file.name}.`))
          reader.readAsDataURL(file)
        }),
    ),
  )
  return invoke<AttachmentDto[]>('import_attachments', { files: uploads })
}
export const attachmentPreview = (id: string) => invoke<string>('attachment_preview', { id })
export const decideApproval = (
  channelId: string,
  approvalId: string,
  decision: string,
  ruleCut?: number,
) => invoke<void>('decide_approval', { channelId, approvalId, decision, ruleCut })
export const cancelTurn = (channelId: string) => invoke<void>('cancel_turn', { channelId })
export const setPanels = (channelList: number, memberList: number) =>
  invoke<void>('set_panels', { channelList, memberList })
export const setMemberListVisible = (visible: boolean) =>
  invoke<void>('set_member_list_visible', { visible })
export const toggleBranch = (key: string) => invoke<void>('toggle_branch', { key })
export const markRead = (channelId: string, messageId: string) =>
  invoke<void>('mark_read', { channelId, messageId })
export const confirmResolve = (accept: boolean) => invoke<void>('confirm_resolve', { accept })
export const archiveConfirm = (accept: boolean) => invoke<void>('archive_confirm', { accept })
export const pluginAction = (plugin: string, action: string) =>
  invoke<void>('plugin_action', { plugin, action })
export const contextMenu = (target: import('./types').ContextTargetDto) =>
  invoke<import('./types').MenuEntryDto[]>('context_menu', { target })
export const runAction = (
  actionId: string,
  target: import('./types').ContextTargetDto | null = null,
) => invoke<string | null>('run_action', { actionId, target })
export const pluginMenuAction = (
  plugin: string,
  index: number,
  target: import('./types').ContextTargetDto,
) => invoke<void>('plugin_menu_action', { plugin, index, target })

export const createChannel = (name: string, workspaceMode: string, sourceChannel: string) =>
  invoke<string>('create_channel', { name, workspaceMode, sourceChannel })
export const switchChannel = (channelId: string) => invoke<void>('switch_channel', { channelId })
export const openBranchChannel = (key: string) => invoke<string>('open_branch_channel', { key })
export const getProjectChannels = (projectId: string) =>
  invoke<ProjectChannelDto[]>('get_project_channels', { projectId })
export const getProjectMembers = (projectId: string) =>
  invoke<ChannelMemberDto[]>('get_project_members', { projectId })
export const getChannelDetails = (projectId: string, channelId: string) =>
  invoke<ChannelDetailsDto>('get_channel_details', { projectId, channelId })
export const updateChannelTopic = (
  projectId: string,
  channelId: string,
  expectedVersion: number,
  text: string,
) =>
  invoke<ChannelTopicDto>('update_channel_topic', {
    projectId,
    channelId,
    expectedVersion,
    text,
  })
export const setProjectMainAgent = (projectId: string, agentId: string) =>
  invoke<void>('set_project_main_agent', { projectId, agentId })
export const selectCanonicalProjectChannel = (projectId: string, channelId: string) =>
  invoke<void>('select_canonical_project_channel', { projectId, channelId })
export const selectCanonicalBranch = (projectId: string, branch: string, channelId: string) =>
  invoke<void>('select_canonical_branch', { projectId, branch, channelId })

export const chooseProjectFolder = (changeCurrent: boolean) =>
  invoke<boolean>('choose_project_folder', { changeCurrent })

export interface ProjectEntry {
  id: string
  name: string
  path: string
  avatar?: string | null
  mainAgentId?: string | null
}
export const updateProjectProfile = (projectId: string, name: string, avatar: string | null) =>
  invoke<void>('update_project_profile', { projectId, name, avatar })
export const getProjects = () => invoke<ProjectEntry[]>('get_projects')
export const selectProject = (projectId: string) => invoke<string>('select_project', { projectId })
export const removeProject = (projectId: string) => invoke<void>('remove_project', { projectId })
export const revealProject = (path: string) => invoke<void>('settings_reveal_workspace', { path })

export interface ReasoningOption {
  value: string
  description: string
}
export interface ProviderModel {
  id: string
  name: string
  description: string
  reasoningEfforts?: ReasoningOption[]
}
export interface AgentEntry {
  id: string
  name: string
  avatar: string | null
  provider: string
  model: string
  reasoningEffort?: string | null
  systemPrompt: string
  policy: string
  pluginWrite: boolean
  workspaceWrite: boolean
  sandboxAutorun: boolean
  sandboxAutorunUnavailable: string | null
  /** True when the provider does not support autorun yet. False for a missing provider. */
  sandboxAutorunPlanned: boolean
  archived: boolean
}
export interface ConversationEntry {
  avatar?: string | null
  inDmList: boolean
  isDefault: boolean
  workspaceMode: string
  id: string
  name: string
  agentId: string
  agent: string
  provider: string
  model: string | null
  projectId: string | null
  archived: boolean
  agentArchived: boolean
  deleted: boolean
  /** Followed threads of this conversation, from storage. */
  threads?: ThreadLinkDto[]
}
export const getAgents = () => invoke<AgentEntry[]>('settings_agent_get')
export const getConversations = () => invoke<ConversationEntry[]>('get_conversations')
export const openAgentChat = (agentId: string) => invoke<string>('open_agent_chat', { agentId })
export const createConversation = (agentId: string, projectId: string | null, name: string) =>
  invoke<string>('create_conversation', { agentId, projectId, name })

export const restoreConversation = (channelId: string) =>
  invoke<void>('restore_conversation', { channelId })

export interface HistoryMatch {
  channelId: string
  channelName: string
  messageId: string
  role: string
  preview: string
}
export interface HistorySearch {
  results: HistoryMatch[]
  hasMore: boolean
}
export const searchAgentHistory = (agentId: string, query: string) =>
  invoke<HistorySearch>('search_agent_history', { agentId, query })

export interface WorktreeRemovalRequest {
  id: string
  channelId: string
  agentId: string
  state: 'pending' | 'running' | 'completed' | 'denied' | 'failed' | 'expired'
  result: string | null
  inspection: { path: string; head: string; recoveryBranch: string | null }
}
export const getWorktreeRequests = (channelId: string) =>
  invoke<WorktreeRemovalRequest[]>('get_worktree_requests', { channelId })
export const decideWorktreeRemoval = (channelId: string, requestId: string, approve: boolean) =>
  invoke<WorktreeRemovalRequest>('decide_worktree_removal', { channelId, requestId, approve })

export interface RuleDto {
  id: string
  agentId: string
  agentName: string
  kind: 'command' | 'host'
  text: string
  createdAt: number
}
export const getAlwaysAllowRules = (projectId: string) =>
  invoke<RuleDto[]>('get_always_allow_rules', { projectId })
export const revokeAlwaysAllowRule = (projectId: string, ruleId: string) =>
  invoke<boolean>('revoke_always_allow_rule', { projectId, ruleId })

export interface WriteRequest {
  id: string
  kind: 'worktree_create' | 'main_checkout_write'
  channelId: string
  agentId: string
  branch: string
  path: string
  state: string
  result: string | null
}
export const getWriteRequests = (channelId: string) =>
  invoke<WriteRequest[]>('get_write_requests', { channelId })
export const decideWriteRequest = (channelId: string, requestId: string, approve: boolean) =>
  invoke<WriteRequest>('decide_write_request', { channelId, requestId, approve })
