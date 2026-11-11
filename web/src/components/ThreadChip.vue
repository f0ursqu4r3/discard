<script setup lang="ts">
import { computed } from 'vue'
import { toast } from 'vue-sonner'
import { Bot, MessagesSquare } from '@lucide/vue'
import type { ThreadChipDto } from '../bridge/types'
import { followThread } from '../bridge/api'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import ThreadStateMark from './ThreadStateMark.vue'
import { messageCountLabel, pendingApprovalsLabel, threadStates } from './threads'
const props = defineProps<{ thread: ThreadChipDto }>()
const emit = defineEmits<{ open: [threadId: string] }>()
const last = computed(() => {
  const message = props.thread.lastMessage
  if (!message?.preview) return null
  return { author: message.authorName ?? (message.role === 'user' ? 'You' : 'Agent'), ...message }
})
const subAgent = computed(() => props.thread.kind === 'sub_agent')
// A sub-agent chip names its agent and state. A question thread has neither state nor an agent name to show.
const openLabel = computed(() => {
  const { name, agentName, state } = props.thread
  if (!subAgent.value) return `Open thread ${name}`
  return [`Open thread ${name}`, agentName, state ? threadStates[state].label : '']
    .filter(Boolean)
    .join(', ')
})
async function follow() {
  try {
    await followThread(props.thread.channelId)
  } catch (error) {
    toast.error('Could not follow thread', { description: String(error) })
  }
}
</script>
<template>
  <div
    data-ui-id="thread-chip"
    :data-kind="thread.kind"
    class="mt-1.5 flex max-w-xl min-w-0 items-center gap-1 rounded-md border border-border/60 bg-card/50"
  >
    <button
      type="button"
      class="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left text-xs outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
      :aria-label="openLabel"
      @click="emit('open', thread.channelId)"
    >
      <Bot v-if="subAgent" class="size-3.5 shrink-0 text-primary" aria-hidden="true" />
      <MessagesSquare v-else class="size-3.5 shrink-0 text-primary" aria-hidden="true" />
      <span class="min-w-0 max-w-40 shrink-0 truncate font-semibold">{{ thread.name }}</span>
      <template v-if="subAgent">
        <span
          data-ui-id="thread-agent"
          class="min-w-0 max-w-24 shrink-0 truncate font-mono text-muted-foreground"
        >
          {{ thread.agentName }}
        </span>
        <ThreadStateMark v-if="thread.state" :state="thread.state" />
      </template>
      <span v-else class="shrink-0 font-mono tabular-nums text-muted-foreground">
        {{ messageCountLabel(thread.messageCount) }}
      </span>
      <span v-if="last" class="min-w-0 flex-1 truncate text-muted-foreground">
        {{ last.author }}: {{ last.preview }}
      </span>
      <span v-else class="flex-1" />
      <Badge
        v-if="thread.pendingApprovals > 0"
        data-ui-id="thread-pending-approvals"
        :aria-label="pendingApprovalsLabel(thread.pendingApprovals)"
        class="h-4 min-w-4 shrink-0 rounded-full px-1 font-mono text-xs tabular-nums"
      >
        {{ thread.pendingApprovals }}
      </Badge>
      <span
        v-if="thread.unread"
        data-ui-id="thread-unread-pip"
        role="img"
        aria-label="Unread messages"
        class="size-2 shrink-0 rounded-full bg-primary"
      />
    </button>
    <Button
      v-if="!thread.followed"
      variant="ghost"
      size="xs"
      class="mr-1 shrink-0"
      :aria-label="`Follow thread ${thread.name}`"
      @click="follow"
    >
      Follow
    </Button>
  </div>
</template>
