import { invoke } from '@tauri-apps/api/core'
import type {
  ChannelDetailsDto,
  ChannelMemberDto,
  ChannelTopicDto,
  HistoryDto,
  ProjectChannelDto,
  SnapshotDto,
} from './types'
export const getSnapshot = () => invoke<SnapshotDto>('get_snapshot')
export const getHistory = () => invoke<HistoryDto>('get_history')
export const sendMessage = (channelId: string, text: string) =>
  invoke<void>('send_message', { channelId, text })
export const decideApproval = (channelId: string, approvalId: string, decision: string) =>
  invoke<void>('decide_approval', { channelId, approvalId, decision })
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
  archived: boolean
}
export interface ConversationEntry {
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
