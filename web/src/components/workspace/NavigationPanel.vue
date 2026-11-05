<script setup lang="ts">
import type { SnapshotDto } from '@/bridge/types'
import type { WorkspacePage } from './types'
import ServerRail from '../ServerRail.vue'
import ChannelList from '../ChannelList.vue'
import UserBar from './UserBar.vue'
import { settingsTarget } from '../layout'
defineProps<{ snapshot: SnapshotDto; page: WorkspacePage; compact?: boolean }>()
const emit = defineEmits<{ navigate: [page: WorkspacePage]; action: [id: string]; toggleBranch: [key: string] }>()
</script>
<template>
  <section data-ui-id="navigation-panel" class="flex h-full min-h-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-sidebar">
    <div class="flex min-h-0 flex-1">
      <ServerRail :compact="compact" :project="snapshot.project" :unread="snapshot.unread" :pending-approvals="snapshot.pendingApprovals" :direct-messages="page === 'direct-messages'" :target="{ type: 'server', id: snapshot.projectId, name: snapshot.project, path: snapshot.projectPath }" @direct-messages="emit('navigate', 'direct-messages')" @project="emit('navigate', 'conversation')" @add-project="emit('action', 'open_workspace_settings')" />
      <ChannelList v-if="!compact" class="min-w-0 flex-1" :page="page" :project="snapshot.project" :project-path="snapshot.projectPath" :agent="snapshot.agent" :channel="snapshot.channel" :unread="snapshot.unread" :pending-approvals="snapshot.pendingApprovals" :branches="snapshot.branches" :target="{ type: 'channel', id: snapshot.channelId }" @navigate="emit('navigate', $event)" @action="emit('action', $event)" @toggle-branch="emit('toggleBranch', $event)" />
    </div>
    <UserBar :compact="compact" :review-count="snapshot.banners.reviewCount" @settings="emit('action', settingsTarget(snapshot.banners.reviewCount)[0])" />
  </section>
</template>
