<script setup lang="ts">
import type { SnapshotDto } from '@/bridge/types'
import type { WorkspacePage } from './types'
import ServerRail from '../ServerRail.vue'
import type { ProjectMenuAction } from '../project-menu'
import type { ProjectEntry } from '@/bridge/api'
import ChannelList from '../ChannelList.vue'
import UserBar from './UserBar.vue'
defineProps<{
  projects?: import('@/bridge/api').ProjectEntry[]
  snapshot: SnapshotDto
  page: WorkspacePage
  compact?: boolean
  busy?: boolean
}>()
const emit = defineEmits<{
  navigate: [page: WorkspacePage]
  action: [id: string]
  toggleBranch: [key: string]
  selectChannel: [id: string]
  openBranch: [key: string]
  createChannel: []
  approvals: []
  projectAction: [action: ProjectMenuAction, project: ProjectEntry]
}>()
</script>
<template>
  <section
    data-ui-id="navigation-panel"
    class="flex h-full min-h-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-sidebar"
  >
    <div class="flex min-h-0 flex-1">
      <ServerRail
        :projects="projects"
        :busy="busy || snapshot.archiveBusy || snapshot.channels.some((channel) => channel.running)"
        @project-action="(action, project) => emit('projectAction', action, project)"
        :project-id="snapshot.projectId"
        :compact="compact"
        :project="snapshot.project"
        :unread="snapshot.channels.some((channel) => channel.unread)"
        :pending-approvals="
          snapshot.channels.reduce((total, channel) => total + channel.pendingApprovals, 0)
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
      <ChannelList
        v-if="!compact"
        class="min-w-0 flex-1"
        :page="page"
        :busy="busy"
        :channels="snapshot.channels"
        :channel-id="snapshot.channelId"
        :project="snapshot.project"
        :project-id="snapshot.projectId"
        :active-project-id="snapshot.activeProjectId"
        :project-path="snapshot.projectPath"
        :agent="snapshot.agent"
        :channel="snapshot.channel"
        :unread="snapshot.unread"
        :pending-approvals="snapshot.pendingApprovals"
        :branches="snapshot.activeProjectId === snapshot.projectId ? snapshot.branches : []"
        :target="{ type: 'channel', id: snapshot.channelId }"
        @navigate="emit('navigate', $event)"
        @action="emit('action', $event)"
        @toggle-branch="emit('toggleBranch', $event)"
        @select-channel="emit('selectChannel', $event)"
        @open-branch="emit('openBranch', $event)"
        @create-channel="emit('createChannel')"
        @approvals="emit('approvals')"
      />
    </div>
    <UserBar
      :compact="compact"
      :review-count="snapshot.banners.reviewCount"
      @settings="emit('action', 'open_settings')"
    />
  </section>
</template>
