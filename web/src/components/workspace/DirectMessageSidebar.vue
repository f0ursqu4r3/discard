<script setup lang="ts">
import type { ChannelDto } from '@/bridge/types'
import AgentsNavigation from './AgentsNavigation.vue'

defineProps<{
  channels?: ChannelDto[]
  channelId?: string
  busy?: boolean
  openThreadId?: string | null
}>()
const emit = defineEmits<{
  selectChannel: [id: string]
  action: [id: string]
  selectThread: [threadId: string]
  leaveThread: [threadId: string]
}>()
</script>

<template>
  <nav
    data-ui-id="direct-message-sidebar"
    aria-label="Direct message navigation"
    class="flex h-full min-w-0 flex-col bg-sidebar text-sidebar-foreground"
  >
    <div
      class="flex min-h-0 flex-1 flex-col overflow-x-hidden overflow-y-auto overscroll-contain p-2.5"
    >
      <AgentsNavigation
        :channels="channels"
        :project-id="null"
        :channel-id="channelId"
        :busy="busy"
        :open-thread-id="openThreadId"
        @select="emit('selectChannel', $event)"
        @action="emit('action', $event)"
        @select-thread="emit('selectThread', $event)"
        @leave-thread="emit('leaveThread', $event)"
      />
    </div>
  </nav>
</template>
