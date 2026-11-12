<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import {
  BellOff,
  ChevronDown,
  ChevronRight,
  Monitor,
  Users,
  Cpu,
  FileText,
  Settings,
  Hash,
  Cable,
  Boxes,
  ShieldCheck,
  Plus,
} from '@lucide/vue'
import NavigationContextMenu from './NavigationContextMenu.vue'
import { requestAction } from './thread-actions'
import {
  branchMenuState,
  channelBusy,
  channelMenuState,
  isDefaultBranch,
  worktreeMenuState,
} from './navigation-actions'
import BrailleSpinner from './BrailleSpinner.vue'
import SidebarWorktree from './SidebarWorktree.vue'
import ThreadRow from './ThreadRow.vue'
import { channelApprovals, threadLinks } from './threads'
import {
  archivedChannels,
  branchActions,
  branchIdentity,
  channelRowClass,
  groupBranchRows,
} from './branch-navigation'
import CreateTextChannelDialog from './workspace/CreateTextChannelDialog.vue'
import { toast } from 'vue-sonner'
import { setChannelMuted } from '../bridge/api'
import { Button } from './ui/button'
import { Badge } from './ui/badge'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from './ui/dropdown-menu'
import { cn } from '@/lib/utils'
import type {
  BranchRowDto,
  RepositoryDto,
  ChannelDto,
  ContextTargetDto,
  ProjectChannelDto,
} from '../bridge/types'
import type { WorkspacePage } from './workspace/types'
const props = withDefaults(
  defineProps<{
    projectId?: string
    activeProjectId?: string | null
    branches: BranchRowDto[]
    repositories?: RepositoryDto[]
    repositoriesLoading?: boolean
    channels?: ChannelDto[]
    projectChannels?: ProjectChannelDto[]
    /** Show the Archived section. The project setting turns it on. */
    showArchived?: boolean
    channelId?: string
    project?: string
    projectAvatar?: string | null
    projectPath?: string
    projectDisplayPath?: string
    channel?: string
    agent?: string
    unread?: boolean
    pendingApprovals?: number
    target?: ContextTargetDto
    page?: WorkspacePage
    busy?: boolean
    /** An archive runs, or waits for its second confirm, in the selected channel. */
    activeChannelBusy?: boolean
    /** The thread panel shows this thread. */
    openThreadId?: string | null
  }>(),
  { page: 'conversation', channels: () => [], projectChannels: () => [] },
)
const emit = defineEmits<{
  toggleBranch: [key: string]
  navigate: [page: WorkspacePage]
  action: [id: string]
  selectChannel: [id: string]
  inspectChannel: [id: string]
  openBranch: [key: string]
  approvals: []
  selectThread: [threadId: string]
  leaveThread: [threadId: string]
}>()
function threadsOf(channelId: string | undefined) {
  return threadLinks(channelId, props.channels, props.projectChannels)
}
const generalChannel = computed(() =>
  props.projectChannels.find((channel) => channel.kind === 'general' && !channel.archived),
)
const textChannels = computed(() =>
  props.projectChannels.filter((channel) => channel.kind === 'text' && !channel.archived),
)
// `#general` is the first text channel. Text channels follow in creation order.
const textRows = computed(() => [
  { general: true, channel: generalChannel.value },
  ...textChannels.value.map((channel) => ({ general: false, channel })),
])
const createTextOpen = ref(false)
// A muted channel shows no unread marker. The app already sends `unread: false` for it.
const unreadChannelIds = computed(
  () =>
    new Set(
      props.channels
        .filter(
          (channel) => channel.projectId === props.projectId && channel.unread && !channel.muted,
        )
        .map((channel) => channel.id),
    ),
)
function channelBusyNow(id: string | undefined) {
  return channelBusy(
    props.channels.find((channel) => channel.id === id),
    id === props.channelId && !!props.activeChannelBusy,
  )
}
function textMenu(channel: ProjectChannelDto | undefined) {
  return channelMenuState(channel, channelBusyNow(channel?.id))
}
function requestMute(channel: ProjectChannelDto | undefined) {
  if (!channel || !props.projectId) return
  void setChannelMuted(props.projectId, channel.id, !channel.muted).catch((error) =>
    toast.error(channel.muted ? 'Could not unmute channel' : 'Could not mute channel', {
      description: String(error),
    }),
  )
}
function requestArchive(channel: ProjectChannelDto | undefined, name: string) {
  if (!channel || !props.projectId) return
  requestAction({
    kind: 'archive-channel',
    projectId: props.projectId,
    channelId: channel.id,
    name,
  })
}
function requestBranch(
  kind: 'create-worktree' | 'rename-branch' | 'delete-branch',
  row: BranchRowDto,
) {
  if (!props.projectId) return
  requestAction({
    kind,
    projectId: props.projectId,
    repository: row.repository ?? '',
    branch: row.label,
  })
}
function requestWorktree(kind: 'create-branch' | 'remove-worktree', row: BranchRowDto) {
  if (!props.projectId || !row.worktree) return
  const target = {
    projectId: props.projectId,
    repository: row.repository ?? '',
    worktree: row.worktree,
  }
  requestAction(
    kind === 'create-branch' ? { kind, ...target, name: row.label } : { kind, ...target },
  )
}
/** The branch row of a group is checked out when any working tree holds the branch. */
function branchState(group: { row: BranchRowDto; worktrees: BranchRowDto[] }) {
  const channel = branchChannel(group.row)
  return branchMenuState({
    checkedOut: group.worktrees.length > 0,
    isDefault: isDefaultBranch(props.branches, group.row),
    channel,
    busy: channelBusyNow(channel?.id),
  })
}
/** The channels that use a working tree: those with a saved worktree at its path, and the branch channel. */
function worktreeState(row: BranchRowDto) {
  const users = props.projectChannels.filter(
    (channel) =>
      !channel.archived &&
      channel.projectId === props.projectId &&
      !!row.worktree &&
      channel.worktree?.path === row.worktree,
  )
  const branch = row.detached ? undefined : branchChannel(row)
  if (branch && !users.includes(branch)) users.push(branch)
  return worktreeMenuState(
    row,
    users.some((channel) => channelBusyNow(channel.id)),
  )
}
const unreadBranches = computed(
  () =>
    new Set(
      props.projectChannels
        .filter(
          (channel) =>
            channel.projectId === props.projectId &&
            channel.kind === 'branch' &&
            !channel.archived &&
            unreadChannelIds.value.has(channel.id),
        )
        .map((channel) => branchIdentity(channel.repository, channel.branch ?? '')),
    ),
)
const unreadWorktrees = computed(
  () =>
    new Set(
      props.projectChannels
        .filter(
          (channel) =>
            channel.projectId === props.projectId &&
            !channel.archived &&
            channel.kind === 'legacy' &&
            channel.worktree &&
            unreadChannelIds.value.has(channel.id),
        )
        .map((channel) => JSON.stringify([channel.worktree!.path, channel.worktree!.branch])),
    ),
)
function rowUnread(row: BranchRowDto) {
  return row.detached
    ? unreadWorktrees.value.has(JSON.stringify([row.worktree, row.label]))
    : unreadBranches.value.has(branchIdentity(row.repository, row.label))
}
function runner(channelId: string | undefined) {
  return props.channels.find((channel) => channel.id === channelId && channel.running)
}
function branchChannel(row: BranchRowDto) {
  return props.projectChannels.find(
    (channel) =>
      !channel.archived &&
      channel.projectId === props.projectId &&
      (row.detached
        ? channel.kind === 'legacy' &&
          channel.worktree?.path === row.worktree &&
          channel.worktree?.branch === row.label
        : channel.kind === 'branch' &&
          branchIdentity(channel.repository, channel.branch ?? '') ===
            branchIdentity(row.repository, row.label)),
  )
}
function branchRunner(row: BranchRowDto) {
  return runner(branchChannel(row)?.id)
}
const navigation = [
  { id: 'overview', label: 'Overview', icon: Monitor },
  { id: 'agents', label: 'Agents', icon: Users },
  { id: 'memory', label: 'Memory', icon: Cpu },
  { id: 'artifacts', label: 'Artifacts', icon: FileText },
] as const
const sections = reactive<Record<string, boolean>>({
  text: true,
  branches: true,
  worktrees: true,
  integrations: true,
  archived: false,
})
const gitBranches = computed(() => props.branches.filter((row) => !row.detached))
const repositoryGroups = computed(() =>
  (
    props.repositories ?? [{ id: '', label: props.project ?? '', path: props.projectPath ?? '' }]
  ).map((repository) => ({
    ...repository,
    ...groupBranchRows(props.branches.filter((row) => (row.repository ?? '') === repository.id)),
  })),
)
function sectionKey(repository: string, section: string) {
  return repository ? JSON.stringify([repository, section]) : section
}
const archivedList = computed(() => archivedChannels(props.projectChannels, gitBranches.value))
watch(
  () => props.projectPath,
  (path) => {
    for (const key of Object.keys(sections)) delete sections[key]
    Object.assign(sections, {
      text: true,
      branches: true,
      worktrees: true,
      integrations: true,
      archived: false,
    })
    try {
      const saved: unknown = JSON.parse(localStorage.getItem(`discard:sections:${path}`) ?? '{}')
      if (saved && typeof saved === 'object') {
        for (const [key, value] of Object.entries(saved)) {
          if (typeof value === 'boolean') sections[key] = value
        }
      }
    } catch {
      /* Use the default section state. */
    }
  },
  { immediate: true },
)
watch(sections, (value) => {
  try {
    localStorage.setItem(`discard:sections:${props.projectPath}`, JSON.stringify(value))
  } catch {
    /* Navigation remains usable when storage is unavailable. */
  }
})
const approvals = computed(
  () =>
    props.channels.reduce((count, channel) => count + channelApprovals(channel), 0) ||
    props.pendingApprovals ||
    0,
)
</script>
<template>
  <nav
    data-ui-id="channel-panel"
    aria-label="Workspace navigation"
    class="flex h-full min-w-0 flex-col bg-sidebar text-sidebar-foreground"
  >
    <DropdownMenu v-if="projectId">
      <DropdownMenuTrigger as-child>
        <button
          data-ui-id="project-header"
          type="button"
          aria-label="Project menu"
          class="flex h-16 w-full shrink-0 items-center gap-3 border-b border-border/60 px-4 text-left outline-none transition-colors hover:bg-sidebar-accent/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        >
          <span class="min-w-0 flex-1">
            <span class="block truncate text-sm font-semibold">
              {{ project }}
            </span>
            <span
              class="block truncate font-mono text-xs text-muted-foreground"
              :title="projectPath"
            >
              {{ projectDisplayPath ?? projectPath }}
            </span>
          </span>
          <ChevronDown class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent data-ui-id="project-menu" align="start" class="w-60">
        <DropdownMenuItem :disabled="!projectId" @select="emit('navigate', 'overview')">
          <Monitor />
          Project overview
        </DropdownMenuItem>
        <DropdownMenuItem
          data-ui-id="project-menu-new-text-channel"
          :disabled="busy"
          @select="createTextOpen = true"
        >
          <Plus />
          New text channel
        </DropdownMenuItem>
        <DropdownMenuItem @select="emit('action', 'show_friends')">
          <Users />
          Agents
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem :disabled="!projectId" @select="emit('navigate', 'project-settings')">
          <Settings />
          Project settings
        </DropdownMenuItem>
        <DropdownMenuItem @select="emit('action', 'open_provider_settings')">
          <Cable />
          CLI connections
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
    <div
      data-ui-id="channel-panel-scroll"
      class="flex min-h-0 flex-1 flex-col overflow-x-hidden overflow-y-auto overscroll-contain p-2.5"
    >
      <template v-if="projectId">
        <div data-ui-id="project-navigation" class="flex flex-col gap-0.5">
          <Button
            v-for="item in navigation"
            :key="item.id"
            :data-ui-id="`nav-${item.id}`"
            :aria-current="page === item.id ? 'page' : undefined"
            size="sm"
            :variant="page === item.id ? 'secondary' : 'ghost'"
            :class="{ 'nav-active-edge': page === item.id }"
            class="h-8 justify-start gap-2 px-2 font-normal"
            @click="emit('navigate', item.id)"
          >
            <component :is="item.icon" data-icon="inline-start" />
            {{ item.label }}
          </Button>
          <Button
            data-ui-id="nav-settings"
            :variant="page === 'project-settings' ? 'secondary' : 'ghost'"
            :class="{ 'nav-active-edge': page === 'project-settings' }"
            :aria-current="page === 'project-settings' ? 'page' : undefined"
            size="sm"
            class="h-8 justify-start gap-2 px-2 font-normal"
            @click="emit('navigate', 'project-settings')"
          >
            <Settings data-icon="inline-start" />
            Settings
          </Button>
        </div>

        <Collapsible v-model:open="sections.text" data-ui-id="text-channels-list" class="mt-3">
          <div class="flex items-center gap-1">
            <CollapsibleTrigger as-child>
              <button
                type="button"
                data-ui-id="text-channels-toggle"
                class="section-label flex h-7 min-w-0 flex-1 items-center px-1 text-left outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
              >
                <component
                  :is="sections.text ? ChevronDown : ChevronRight"
                  class="size-3 shrink-0 text-muted-foreground"
                />
                Text channels
                <span class="ml-1 font-mono font-normal tabular-nums text-muted-foreground">
                  {{ textRows.length }}
                </span>
              </button>
            </CollapsibleTrigger>
            <TooltipProvider :delay-duration="500" :skip-delay-duration="0">
              <Tooltip>
                <TooltipTrigger as-child>
                  <Button
                    data-ui-id="text-channels-create"
                    variant="ghost"
                    size="icon-xs"
                    aria-label="New text channel"
                    :disabled="busy"
                    @click="createTextOpen = true"
                  >
                    <Plus aria-hidden="true" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>New text channel</TooltipContent>
              </Tooltip>
            </TooltipProvider>
          </div>
          <CollapsibleContent class="flex flex-col gap-0.5">
            <template v-for="row in textRows" :key="row.channel?.id ?? 'general'">
              <NavigationContextMenu
                kind="channel"
                :name="row.general ? 'general' : row.channel!.name"
                :id="row.channel?.id"
                :renamable="!row.general"
                :busy="busy"
                :muted="textMenu(row.channel).muted"
                :archive-disabled="textMenu(row.channel).archiveDisabled"
                @mute="requestMute(row.channel)"
                @archive="requestArchive(row.channel, row.general ? 'general' : row.channel!.name)"
                @rename="
                  row.channel &&
                  projectId &&
                  requestAction({
                    kind: 'rename-channel',
                    projectId,
                    channelId: row.channel.id,
                    name: row.channel.name,
                  })
                "
                @open="
                  row.channel
                    ? emit('selectChannel', row.channel.id)
                    : emit('action', `select_project:${projectId}`)
                "
              >
                <button
                  type="button"
                  :data-ui-id="
                    row.general ? 'project-general-channel' : `text-channel-${row.channel!.id}`
                  "
                  :aria-current="
                    row.channel?.id === channelId && page === 'conversation' ? 'page' : undefined
                  "
                  :disabled="busy"
                  :data-muted="row.channel?.muted ? 'true' : undefined"
                  :class="
                    cn(
                      channelRowClass,
                      row.channel?.id === channelId && page === 'conversation'
                        ? 'nav-active-edge bg-sidebar-accent text-sidebar-accent-foreground'
                        : 'text-muted-foreground',
                      row.channel?.muted && 'opacity-60',
                    )
                  "
                  @click="
                    row.channel
                      ? emit('selectChannel', row.channel.id)
                      : emit('action', `select_project:${projectId}`)
                  "
                >
                  <Hash class="size-4 shrink-0" />
                  <span class="min-w-0 flex-1 truncate font-mono text-[0.92em]">
                    {{ row.general ? 'general' : row.channel!.name }}
                  </span>
                  <BellOff
                    v-if="row.channel?.muted"
                    data-ui-id="channel-muted-icon"
                    role="img"
                    aria-label="Muted"
                    class="size-3.5 shrink-0"
                  />
                  <BrailleSpinner
                    v-if="runner(row.channel?.id)"
                    :agent-id="runner(row.channel?.id)?.agentId"
                    class="shrink-0"
                  />
                  <span
                    v-if="row.channel && unreadChannelIds.has(row.channel.id)"
                    data-ui-id="channel-unread-pip"
                    role="img"
                    aria-label="Unread messages"
                    class="size-2 shrink-0 rounded-full bg-sidebar-foreground"
                  />
                </button>
              </NavigationContextMenu>
              <ThreadRow
                v-for="(link, index) in threadsOf(row.channel?.id)"
                :key="link.id"
                :link="link"
                :is-last="index === threadsOf(row.channel?.id).length - 1"
                :active="link.id === openThreadId && page === 'conversation'"
                :busy="busy"
                @select="emit('selectThread', $event)"
                @leave="emit('leaveThread', $event)"
              />
            </template>
          </CollapsibleContent>
        </Collapsible>

        <p
          v-if="repositoriesLoading && !branches.length"
          role="status"
          class="mt-5 px-2 text-xs text-muted-foreground"
        >
          Scanning repositories…
        </p>
        <p v-else-if="!repositoryGroups.length" class="mt-5 px-2 text-xs text-muted-foreground">
          No repositories found
        </p>
        <Collapsible
          v-for="repository in repositoryGroups"
          :key="repository.id"
          :open="
            repositoryGroups.length === 1 ||
            sections[sectionKey(repository.id, 'repository')] !== false
          "
          @update:open="sections[sectionKey(repository.id, 'repository')] = $event"
          data-ui-id="repository-section"
          :data-repository-id="repository.id"
          class="mt-5"
        >
          <CollapsibleTrigger v-if="repositoryGroups.length > 1" as-child>
            <button
              type="button"
              data-ui-id="repository-toggle"
              :title="repository.path"
              class="section-label flex h-7 w-full min-w-0 items-center px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <component
                :is="
                  sections[sectionKey(repository.id, 'repository')] !== false
                    ? ChevronDown
                    : ChevronRight
                "
                class="size-3 shrink-0"
              />
              <span class="truncate">{{ repository.label }}</span>
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent :class="repositoryGroups.length > 1 ? 'pl-3' : ''">
            <Collapsible
              :open="sections[sectionKey(repository.id, 'branches')] !== false"
              @update:open="sections[sectionKey(repository.id, 'branches')] = $event"
              data-ui-id="branches-list"
              :class="repositoryGroups.length > 1 ? 'mt-2' : ''"
            >
              <div class="flex items-center gap-1">
                <CollapsibleTrigger as-child>
                  <button
                    type="button"
                    data-ui-id="branches-toggle"
                    class="section-label flex h-7 min-w-0 flex-1 items-center px-1 text-left outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <component
                      :is="
                        sections[sectionKey(repository.id, 'branches')] !== false
                          ? ChevronDown
                          : ChevronRight
                      "
                      class="size-3 shrink-0 text-muted-foreground"
                    />
                    Branch channels
                    <span class="ml-1 font-mono font-normal tabular-nums text-muted-foreground">
                      {{ repository.branches.length }}
                    </span>
                  </button>
                </CollapsibleTrigger>
                <TooltipProvider :delay-duration="500" :skip-delay-duration="0">
                  <Tooltip>
                    <TooltipTrigger as-child>
                      <span
                        class="inline-flex"
                        tabindex="0"
                        aria-disabled="true"
                        :aria-label="`${branchActions.create.label} — Soon`"
                      >
                        <Button
                          disabled
                          variant="ghost"
                          size="icon-xs"
                          :aria-label="`${branchActions.create.label} — Soon`"
                        >
                          <Plus aria-hidden="true" />
                        </Button>
                      </span>
                    </TooltipTrigger>
                    <TooltipContent>{{ branchActions.create.label }} · Soon</TooltipContent>
                  </Tooltip>
                </TooltipProvider>
              </div>
              <!-- <p
                data-ui-id="worktree-count"
                class="mb-2 px-4 font-mono text-xs text-muted-foreground"
              >
                {{ repository.worktreeCount }}
                {{ repository.worktreeCount === 1 ? 'worktree' : 'worktrees' }}
              </p> -->
              <CollapsibleContent class="flex flex-col gap-0.5">
                <div
                  v-for="branch in repository.branches"
                  :key="branch.row.key"
                  data-ui-id="branch-group"
                  :data-branch-key="branch.row.key"
                  class="min-w-0"
                >
                  <NavigationContextMenu
                    kind="branch"
                    :name="branch.row.label"
                    :id="branch.row.key"
                    :busy="busy || !projectId"
                    :create-worktree-disabled="branchState(branch).createWorktreeDisabled"
                    :rename-branch-disabled="branchState(branch).renameBranchDisabled"
                    :delete-branch-disabled="branchState(branch).deleteBranchDisabled"
                    @create-worktree="requestBranch('create-worktree', branch.row)"
                    @rename-branch="requestBranch('rename-branch', branch.row)"
                    @delete-branch="requestBranch('delete-branch', branch.row)"
                    @open="emit('openBranch', branch.row.key)"
                  >
                    <Button
                      variant="ghost"
                      :data-ui-id="`branches-row-${branch.row.key}`"
                      :aria-current="
                        branch.row.current && page === 'conversation' ? 'page' : undefined
                      "
                      :disabled="busy || !projectId"
                      :title="branch.row.label"
                      :class="
                        cn(
                          channelRowClass,
                          branch.worktrees.length && 'font-medium',
                          branch.row.current && page === 'conversation'
                            ? 'nav-active-edge bg-sidebar-accent text-sidebar-accent-foreground'
                            : branch.worktrees.length
                              ? 'text-sidebar-foreground'
                              : 'text-muted-foreground',
                        )
                      "
                      @click="emit('openBranch', branch.row.key)"
                    >
                      <Hash class="size-4 shrink-0" aria-hidden="true" />
                      <span class="min-w-0 flex-1 truncate font-mono text-[0.92em]">
                        {{ branch.row.label }}
                      </span>
                      <BrailleSpinner
                        v-if="branchRunner(branch.row)"
                        :agent-id="branchRunner(branch.row)?.agentId"
                        class="shrink-0"
                      />
                      <span
                        v-if="rowUnread(branch.row)"
                        data-ui-id="channel-unread-pip"
                        role="img"
                        aria-label="Unread messages"
                        class="size-2 shrink-0 rounded-full bg-sidebar-foreground"
                      />
                    </Button>
                  </NavigationContextMenu>
                  <ThreadRow
                    v-for="(link, index) in threadsOf(branchChannel(branch.row)?.id)"
                    :key="link.id"
                    :link="link"
                    :is-last="index === threadsOf(branchChannel(branch.row)?.id).length - 1"
                    :active="link.id === openThreadId && page === 'conversation'"
                    :busy="busy"
                    @select="emit('selectThread', $event)"
                    @leave="emit('leaveThread', $event)"
                  />
                  <SidebarWorktree
                    v-for="(worktree, index) in branch.worktrees"
                    :key="worktree.worktree ?? worktree.key"
                    :row="worktree"
                    :is-last="index === branch.worktrees.length - 1"
                    :working-agent-id="branchRunner(worktree)?.agentId"
                    :busy="busy || !projectId"
                    :unread="rowUnread(worktree)"
                    :create-branch-disabled="worktreeState(worktree).createBranchDisabled"
                    :remove-worktree-disabled="worktreeState(worktree).removeWorktreeDisabled"
                    @create-branch="requestWorktree('create-branch', worktree)"
                    @remove-worktree="requestWorktree('remove-worktree', worktree)"
                    @open="emit('openBranch', $event)"
                  />
                </div>
                <p
                  v-if="!repository.branches.length"
                  class="px-2 py-1 text-xs text-muted-foreground"
                >
                  No branches found
                </p>
              </CollapsibleContent>
            </Collapsible>
            <Collapsible
              v-if="repository.detached.length"
              :open="sections[sectionKey(repository.id, 'worktrees')] !== false"
              @update:open="sections[sectionKey(repository.id, 'worktrees')] = $event"
              data-ui-id="detached-worktrees-list"
              class="mt-3"
            >
              <CollapsibleTrigger as-child>
                <button
                  type="button"
                  data-ui-id="detached-worktrees-toggle"
                  class="section-label flex h-7 w-full items-center px-1 text-left outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <component
                    :is="
                      sections[sectionKey(repository.id, 'worktrees')] !== false
                        ? ChevronDown
                        : ChevronRight
                    "
                    class="size-3"
                  />
                  Detached worktrees
                  <span class="ml-1 font-mono font-normal tabular-nums">
                    {{ repository.detached.length }}
                  </span>
                </button>
              </CollapsibleTrigger>
              <CollapsibleContent>
                <template v-for="row in repository.detached" :key="row.worktree ?? row.key">
                  <SidebarWorktree
                    :row="row"
                    :working-agent-id="branchRunner(row)?.agentId"
                    :busy="busy || !projectId"
                    :selected="row.current && page === 'conversation'"
                    :unread="rowUnread(row)"
                    :create-branch-disabled="worktreeState(row).createBranchDisabled"
                    :remove-worktree-disabled="worktreeState(row).removeWorktreeDisabled"
                    @create-branch="requestWorktree('create-branch', row)"
                    @remove-worktree="requestWorktree('remove-worktree', row)"
                    @open="emit('openBranch', $event)"
                  />
                  <ThreadRow
                    v-for="(link, index) in threadsOf(branchChannel(row)?.id)"
                    :key="link.id"
                    :link="link"
                    :is-last="index === threadsOf(branchChannel(row)?.id).length - 1"
                    :active="link.id === openThreadId && page === 'conversation'"
                    :busy="busy"
                    @select="emit('selectThread', $event)"
                    @leave="emit('leaveThread', $event)"
                  />
                </template>
              </CollapsibleContent>
            </Collapsible>
            <!-- <Button
              disabled
              variant="ghost"
              size="sm"
              class="mt-3 w-full justify-start gap-2 px-2 text-muted-foreground"
            >
              <Monitor class="size-4 shrink-0" aria-hidden="true" />
              <span class="min-w-0 truncate">
                {{ branchActions.manage.label }}
              </span>
              <span class="ml-auto text-[10px]">Soon</span>
            </Button> -->
          </CollapsibleContent>
        </Collapsible>
        <Collapsible
          v-if="showArchived && archivedList.length"
          v-model:open="sections.archived"
          data-ui-id="archived-channel-navigation"
          class="mt-3"
        >
          <CollapsibleTrigger as-child>
            <button
              type="button"
              data-ui-id="archived-channel-toggle"
              class="section-label flex h-7 w-full items-center px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <component :is="sections.archived ? ChevronDown : ChevronRight" class="size-3" />
              Archived
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent class="flex flex-col gap-0.5">
            <template v-for="channel in archivedList" :key="channel.id">
              <NavigationContextMenu
                kind="channel"
                :name="channel.branch ?? channel.name"
                :id="channel.id"
                :busy="busy"
                :muted="textMenu(channel).muted"
                :archive-disabled="textMenu(channel).archiveDisabled"
                @mute="requestMute(channel)"
                @archive="requestArchive(channel, channel.branch ?? channel.name)"
                @open="emit('selectChannel', channel.id)"
              >
                <Button
                  variant="ghost"
                  :data-ui-id="`archived-channel-${channel.id}`"
                  :class="{
                    'nav-active-edge bg-sidebar-accent':
                      channel.id === channelId && page === 'conversation',
                    'opacity-60': channel.muted,
                  }"
                  :disabled="busy"
                  :data-muted="channel.muted ? 'true' : undefined"
                  class="flex h-8 w-full min-w-0 items-center gap-2 rounded-md px-2 text-left text-xs text-muted-foreground outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring"
                  @click="emit('selectChannel', channel.id)"
                >
                  <Hash class="size-4 shrink-0" />
                  <span class="min-w-0 flex-1 truncate font-mono text-[0.92em]">
                    {{ channel.branch ?? channel.name }}
                  </span>
                  <BellOff
                    v-if="channel.muted"
                    data-ui-id="channel-muted-icon"
                    role="img"
                    aria-label="Muted"
                    class="size-3.5 shrink-0"
                  />
                  <BrailleSpinner
                    v-if="runner(channel.id)"
                    :agent-id="runner(channel.id)?.agentId"
                    class="shrink-0"
                  />
                  <span
                    v-if="unreadChannelIds.has(channel.id)"
                    data-ui-id="channel-unread-pip"
                    role="img"
                    aria-label="Unread messages"
                    class="size-2 shrink-0 rounded-full bg-sidebar-foreground"
                  />
                  <span class="shrink-0">
                    {{
                      channel.archived ? 'History' : channel.kind === 'legacy' ? 'Saved' : 'Channel'
                    }}
                  </span>
                </Button>
              </NavigationContextMenu>
              <ThreadRow
                v-for="(link, index) in threadsOf(channel.id)"
                :key="link.id"
                :link="link"
                :is-last="index === threadsOf(channel.id).length - 1"
                :active="link.id === openThreadId && page === 'conversation'"
                :busy="busy"
                @select="emit('selectThread', $event)"
                @leave="emit('leaveThread', $event)"
              />
            </template>
          </CollapsibleContent>
        </Collapsible>
        <Collapsible
          v-model:open="sections.integrations"
          data-ui-id="integrations-navigation"
          class="mt-5"
        >
          <CollapsibleTrigger as-child>
            <button
              data-ui-id="integrations-toggle"
              class="section-label flex h-7 w-full items-center px-1 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              <component :is="sections.integrations ? ChevronDown : ChevronRight" class="size-3" />
              Integrations
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent class="flex flex-col gap-0.5">
            <Button
              data-ui-id="nav-connections"
              variant="ghost"
              size="sm"
              class="h-8 justify-start gap-2 px-2 font-normal"
              @click="emit('action', 'open_provider_settings')"
            >
              <Cable data-icon="inline-start" />
              CLI connections
            </Button>
            <Button
              data-ui-id="nav-plugins"
              variant="ghost"
              size="sm"
              class="h-8 justify-start gap-2 px-2 font-normal"
              @click="emit('action', 'review_plugins')"
            >
              <Boxes data-icon="inline-start" />
              Lua plugins
            </Button>
            <Button
              data-ui-id="nav-approvals"
              variant="ghost"
              size="sm"
              class="h-8 justify-start gap-2 px-2 font-normal"
              @click="emit('approvals')"
            >
              <ShieldCheck data-icon="inline-start" />
              Approvals
              <Badge
                v-if="approvals"
                class="ml-auto h-4 min-w-4 font-mono rounded-full px-1 text-xs tabular-nums"
              >
                {{ approvals }}
              </Badge>
            </Button>
          </CollapsibleContent>
        </Collapsible>
      </template>
      <slot />
    </div>
    <CreateTextChannelDialog
      v-if="projectId"
      v-model:open="createTextOpen"
      :project-id="projectId"
    />
  </nav>
</template>
