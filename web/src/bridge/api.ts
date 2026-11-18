import { invoke } from '@tauri-apps/api/core'
import type {
  ArchiveChannelDto,
  AttachmentDto,
  BranchDeletionDto,
  ChangesScope,
  ChannelChangesDto,
  ChannelDetailsDto,
  ChannelMemberDto,
  ChannelTopicDto,
  CommandDto,
  HistoryDto,
  ProjectChannelDto,
  SnapshotDto,
  ThreadHistoryDto,
  ThreadLinkDto,
  WorktreeFilesDto,
  WorktreePlanDto,
  WorktreeRemovalDto,
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
/**
 * Give a thread a new name. Returns the stored name. The app trims it, turns line breaks into spaces and
 * rejects a name of 0 or more than 80 characters.
 */
export const renameThread = (threadId: string, name: string) =>
  invoke<string>('rename_thread', { threadId, name })
/** How many messages of a thread were posted to its channel. A delete keeps these in the channel. */
export const threadPostedCount = (threadId: string) =>
  invoke<number>('thread_posted_count', { threadId })
/**
 * Delete a thread with its messages. A message that was posted to the channel stays there as a normal
 * message. Returns how many stayed. Rejects while a turn runs in the thread, while its sub-agent task
 * runs and while an approval waits.
 */
export const deleteThread = (threadId: string) => invoke<number>('delete_thread', { threadId })
/** Remove a posted reference row from its channel. The message stays in its thread. `messageId` is the row ID. */
export const removePostedMessage = (messageId: string) =>
  invoke<void>('remove_posted_message', { messageId })
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
export const createTextChannel = (projectId: string, name: string) =>
  invoke<string>('create_text_channel', { projectId, name })
/** Rename a text channel with the checks of channel creation. Returns the stored name. */
export const renameTextChannel = (projectId: string, channelId: string, name: string) =>
  invoke<string>('rename_text_channel', { projectId, channelId, name })
/** Mute or unmute a channel. A muted channel shows no unread marker. A thread cannot be muted. */
export const setChannelMuted = (projectId: string, channelId: string, muted: boolean) =>
  invoke<void>('set_channel_muted', { projectId, channelId, muted })
/**
 * Archive the channel of a row. Resolves with `archived: false` and the items that the archive deletes when it
 * needs the second confirm. Call again with `force` to archive. When the open channel is archived, `#general` opens.
 */
export const archiveProjectChannel = (projectId: string, channelId: string, force = false) =>
  invoke<ArchiveChannelDto>('archive_project_channel', { projectId, channelId, force })
/** Where Create Worktree… would put the worktree of a branch. `repository` is empty for the project root. */
export const worktreePlan = (projectId: string, repository: string, branch: string) =>
  invoke<WorktreePlanDto>('worktree_plan', { projectId, repository, branch })
/** Create a linked worktree for a branch. Pass the `channelId` of the plan. Returns the worktree path. */
export const createBranchWorktree = (
  projectId: string,
  repository: string,
  branch: string,
  channelId?: string,
) =>
  invoke<string>('create_branch_worktree', {
    projectId,
    repository,
    branch,
    ...(channelId ? { channelId } : {}),
  })
/** Rename a local branch and move its channel binding. Returns the new name. Rejects with the reason. */
export const renameGitBranch = (
  projectId: string,
  repository: string,
  branch: string,
  newName: string,
) => invoke<string>('rename_git_branch', { projectId, repository, branch, newName })
/** The unmerged commits that deleting a branch would drop. Rejects with the reason a delete is refused. */
export const branchDeletionInfo = (projectId: string, repository: string, branch: string) =>
  invoke<number>('branch_deletion_info', { projectId, repository, branch })
/** Delete a local branch. Unmerged commits need `force`. */
export const deleteGitBranch = (
  projectId: string,
  repository: string,
  branch: string,
  force = false,
) => invoke<BranchDeletionDto>('delete_git_branch', { projectId, repository, branch, force })
/** Start a new branch in a working tree and switch it to the branch. Returns the new name. */
export const createGitBranch = (
  projectId: string,
  repository: string,
  worktree: string,
  name: string,
) => invoke<string>('create_git_branch', { projectId, repository, worktree, name })
/** The files that removing a working tree would discard. Rejects with the reason a removal is refused. */
export const worktreeRemovalInfo = (projectId: string, repository: string, worktree: string) =>
  invoke<WorktreeFilesDto>('worktree_removal_info', { projectId, repository, worktree })
/** Remove a linked working tree. The branch stays. Files that would be lost need `force`. */
export const removeGitWorktree = (
  projectId: string,
  repository: string,
  worktree: string,
  force = false,
) => invoke<WorktreeRemovalDto>('remove_git_worktree', { projectId, repository, worktree, force })
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
  /** The channel list shows its Archived section. Off when absent. */
  showArchived?: boolean
}
export const setProjectShowArchived = (projectId: string, show: boolean) =>
  invoke<void>('set_project_show_archived', { projectId, show })
export const updateProjectProfile = (projectId: string, name: string, avatar: string | null) =>
  invoke<void>('update_project_profile', { projectId, name, avatar })
export const getProjects = () => invoke<ProjectEntry[]>('get_projects')
/** One top-level item of the project rail. A group lists its member projects in order. */
export type RailItemDto =
  | { kind: 'project'; projectId: string }
  | {
      kind: 'group'
      id: string
      /** May be empty. The rail then builds a label from the member names. */
      name: string
      /** An `ansi-1` to `ansi-6` theme slot or `#rrggbb`. Null means the default tint. */
      color: string | null
      collapsed: boolean
      projectIds: string[]
    }
export interface RailLayoutDto {
  items: RailItemDto[]
}
export const getRailLayout = () => invoke<RailLayoutDto>('get_rail_layout')
/** `index` counts the target list without the moved project. A null group is the top level. */
export const moveRailProject = (projectId: string, groupId: string | null, index: number) =>
  invoke<void>('move_rail_project', { projectId, groupId, index })
export const groupRailProjects = (draggedId: string, targetId: string) =>
  invoke<string>('group_rail_projects', { draggedId, targetId })
export const createRailGroup = (projectId: string) =>
  invoke<string>('create_rail_group', { projectId })
export const moveRailGroup = (groupId: string, index: number) =>
  invoke<void>('move_rail_group', { groupId, index })
export const renameRailGroup = (groupId: string, name: string) =>
  invoke<void>('rename_rail_group', { groupId, name })
export const setRailGroupColor = (groupId: string, color: string | null) =>
  invoke<void>('set_rail_group_color', { groupId, color })
export const setRailGroupCollapsed = (groupId: string, collapsed: boolean) =>
  invoke<void>('set_rail_group_collapsed', { groupId, collapsed })
export const ungroupRailGroup = (groupId: string) => invoke<void>('ungroup_rail_group', { groupId })
export const selectProject = (projectId: string) => invoke<string>('select_project', { projectId })
export const removeProject = (projectId: string) => invoke<void>('remove_project', { projectId })
/** Opens the project window of a project, or focuses it if it is open. */
export const openProjectWindow = (projectId: string) =>
  invoke<void>('open_project_window', { projectId })
/** Shows and focuses the main window. With `page`, the main window also navigates to that page. */
export const showMainWindow = (page?: string) => invoke<void>('show_main_window', { page })
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
  /**
   * The agent may start read-only sub-agents with the `delegate` tool in Conversation and Workspace responses.
   * Off by default; the app always sends it. Turning it on goes through the policy-loosening confirmation (`settings_agent_review`
   * lists it as "Delegation"). A new agent cannot start with it on.
   */
  delegation?: boolean
  /** Why autorun cannot run for this provider. A state block, not planned work. `null` when it can. */
  sandboxAutorunUnavailable: string | null
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

export interface MoveHandoff {
  goal: string
  constraints: string[]
  decisions: string[]
  openQuestions: string[]
}
export type MoveTarget =
  | { kind: 'channel'; channelId: string; name: string }
  | {
      kind: 'new_branch'
      repository: string
      repoPath: string
      branch: string
      base: string
      baseCommit: string
    }
export interface MoveDetails {
  target: MoveTarget
  handoff: MoveHandoff
  sourceMessageId: string
}
export interface WriteRequest {
  id: string
  kind: 'worktree_create' | 'main_checkout_write' | 'conversation_move'
  channelId: string
  agentId: string
  branch: string
  path: string
  state: string
  result: string | null
  /** Set only when `kind` is `conversation_move`. */
  move?: MoveDetails
}
export const getWriteRequests = (channelId: string) =>
  invoke<WriteRequest[]>('get_write_requests', { channelId })
export const decideWriteRequest = (channelId: string, requestId: string, approve: boolean) =>
  invoke<WriteRequest>('decide_write_request', { channelId, requestId, approve })

/** A request from an agent to add another agent to the project. Requests are not stored: none survives a restart. */
export interface AgentRequest {
  id: string
  channelId: string
  requestingAgentId: string
  requestingAgentName: string
  /** The agent to add. */
  agentId: string
  agentName: string
  provider: string
  model: string | null
  /** What the requesting agent wrote. At most 500 characters. */
  reason: string
  state: 'pending' | 'added' | 'denied' | 'failed' | 'expired'
  result: string | null
  /** Unix milliseconds. */
  createdAt: number
}
/** The requests of a channel, oldest first, decided or not. */
export const getAgentRequests = (channelId: string) =>
  invoke<AgentRequest[]>('get_agent_requests', { channelId })
/**
 * Decide a request. `add` makes the agent a project member and continues the requesting agent; `deny` continues it
 * with the note that the user declined. The channel must be the selected channel and idle. Refreshes
 * `discard://channel-details` and `discard://conversations`.
 */
export const decideAgentRequest = (
  channelId: string,
  requestId: string,
  decision: 'add' | 'deny',
) => invoke<AgentRequest>('decide_agent_request', { channelId, requestId, decision })

/** The slash command registry: built-ins first, then plugin commands. */
export const getCommands = () => invoke<CommandDto[]>('list_commands')
/** Runs `/name args` in the channel. Returns the text to show, or null when the command shows none. */
export const runCommand = (channelId: string, name: string, args: string) =>
  invoke<string | null>('run_command', { channelId, name, args })

/**
 * The live changes of the folder of a channel: the working tree against `HEAD` (`uncommitted`) or against the
 * point where the branch left the default branch (`branch`). Rejects for a direct message and for a folder that is
 * not a Git repository.
 */
export const channelChanges = (channelId: string, scope: ChangesScope) =>
  invoke<ChannelChangesDto>('channel_changes', { channelId, scope })
