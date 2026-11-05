<script setup lang="ts">
import type { SnapshotDto } from '@/bridge/types'
import type { WorkspacePage } from './types'
import ServerRail from '../ServerRail.vue'
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
        :project-id="snapshot.projectId"
        :compact="compact"
        :project="snapshot.project"
        :unread="snapshot.channels.some((channel) => channel.unread)"
        :pending-approvals="
          snapshot.channels.reduce((total, channel) => total + channel.pendingApprovals, 0)
        "
        :direct-messages="page === 'direct-messages'"
        :target="{
          type: 'server',
          id: snapshot.projectId,
          name: snapshot.project,
          path: snapshot.projectPath,
        }"
        @direct-messages="emit('navigate', 'direct-messages')"
        @project="
          (id) =>
            id && id !== snapshot.projectId
              ? emit('action', `select_project:${id}`)
              : emit('navigate', 'conversation')
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
        :project-path="snapshot.projectPath"
        :agent="snapshot.agent"
        :channel="snapshot.channel"
        :unread="snapshot.unread"
        :pending-approvals="snapshot.pendingApprovals"
        :branches="snapshot.branches"
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
