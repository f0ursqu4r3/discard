<script setup lang="ts">
import type { SnapshotDto } from '@/bridge/types'
import type { ProjectChannelDto } from '@/bridge/types'
import type { WorkspacePage } from './types'
import ServerRail from '../ServerRail.vue'
import type { ProjectMenuAction } from '../project-menu'
import type { ProjectEntry, RailLayoutDto } from '@/bridge/api'
import type { RailChange } from '../rail-layout'
import ChannelList from '../ChannelList.vue'
import UserBar from './UserBar.vue'
import DirectMessageSidebar from './DirectMessageSidebar.vue'
import { channelApprovals } from '../threads'
withDefaults(
  defineProps<{
    projects?: import('@/bridge/api').ProjectEntry[]
    railLayout?: RailLayoutDto | null
    snapshot: SnapshotDto
    projectChannels?: ProjectChannelDto[]
    page: WorkspacePage
    compact?: boolean
    busy?: boolean
    /** A project window has no project rail: no project switch, direct messages entry, or Add Project control. */
    rail?: boolean
  }>(),
  { rail: true },
)
const emit = defineEmits<{
  navigate: [page: WorkspacePage]
  action: [id: string]
  toggleBranch: [key: string]
  selectChannel: [id: string]
  inspectChannel: [id: string]
  openBranch: [key: string]
  approvals: []
  selectThread: [threadId: string]
  leaveThread: [threadId: string]
  projectAction: [action: ProjectMenuAction, project: ProjectEntry]
  railChange: [change: RailChange]
}>()
</script>
<template>
  <section
    data-ui-id="navigation-panel"
    class="flex h-full min-h-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-sidebar"
  >
    <div class="flex min-h-0 flex-1">
      <ServerRail
        v-if="rail"
        :projects="projects"
        :layout="railLayout"
        :channels="snapshot.channels"
        :busy="busy || snapshot.archiveBusy || snapshot.channels.some((channel) => channel.running)"
        @project-action="(action, project) => emit('projectAction', action, project)"
        @rail-change="(change) => emit('railChange', change)"
        :project-id="snapshot.projectId"
        :compact="compact"
        :project="snapshot.project"
        :unread="snapshot.channels.some((channel) => channel.unread && !channel.muted)"
        :pending-approvals="
          snapshot.channels.reduce((total, channel) => total + channelApprovals(channel), 0)
        "
        :direct-messages="page === 'direct-messages'"
        @direct-messages="emit('navigate', 'direct-messages')"
        @project="
          (id) =>
            id && id !== snapshot.projectId
              ? emit('action', `select_project:${id}`)
              : emit('navigate', 'overview')
        "
        @add-project="emit('action', 'import_project')"
      />
      <DirectMessageSidebar
        :channels="snapshot.channels"
        v-if="!compact && page === 'direct-messages'"
        class="min-w-0 flex-1"
        :channel-id="snapshot.channelId"
        :busy="busy"
        :open-thread-id="snapshot.openThread?.channelId"
        @select-channel="emit('selectChannel', $event)"
        @action="emit('action', $event)"
        @select-thread="emit('selectThread', $event)"
        @leave-thread="emit('leaveThread', $event)"
      />
      <ChannelList
        v-else-if="!compact"
        class="min-w-0 flex-1"
        :page="page"
        :busy="busy"
        :active-channel-busy="snapshot.archiveBusy || !!snapshot.banners.archive"
        :channels="snapshot.channels"
        :project-channels="projectChannels"
        :show-archived="
          projects?.find((entry) => entry.id === snapshot.projectId)?.showArchived ?? false
        "
        :channel-id="snapshot.channelId"
        :project="snapshot.project"
        :project-avatar="projects?.find((entry) => entry.id === snapshot.projectId)?.avatar"
        :project-id="snapshot.projectId"
        :active-project-id="snapshot.activeProjectId"
        :project-path="snapshot.projectPath"
        :project-display-path="snapshot.projectDisplayPath"
        :agent="snapshot.agent"
        :channel="snapshot.channel"
        :unread="snapshot.unread"
        :pending-approvals="snapshot.pendingApprovals"
        :branches="snapshot.branches"
        :repositories="snapshot.repositories"
        :repositories-loading="snapshot.repositoriesLoading"
        :target="{ type: 'channel', id: snapshot.channelId }"
        :open-thread-id="snapshot.openThread?.channelId"
        @navigate="emit('navigate', $event)"
        @action="emit('action', $event)"
        @toggle-branch="emit('toggleBranch', $event)"
        @select-channel="emit('selectChannel', $event)"
        @inspect-channel="emit('inspectChannel', $event)"
        @open-branch="emit('openBranch', $event)"
        @approvals="emit('approvals')"
        @select-thread="emit('selectThread', $event)"
        @leave-thread="emit('leaveThread', $event)"
      />
    </div>
    <UserBar
      :compact="compact"
      :review-count="snapshot.banners.reviewCount"
      @settings="emit('action', 'open_settings')"
    />
  </section>
</template>
