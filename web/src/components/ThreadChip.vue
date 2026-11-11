<script setup lang="ts">
import { computed } from 'vue'
import { toast } from 'vue-sonner'
import { MessagesSquare } from '@lucide/vue'
import type { ThreadChipDto } from '../bridge/types'
import { followThread } from '../bridge/api'
import { Button } from './ui/button'
import { messageCountLabel } from './threads'
const props = defineProps<{ thread: ThreadChipDto }>()
const emit = defineEmits<{ open: [threadId: string] }>()
const last = computed(() => {
  const message = props.thread.lastMessage
  if (!message?.preview) return null
  return { author: message.authorName ?? (message.role === 'user' ? 'You' : 'Agent'), ...message }
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
    class="mt-1.5 flex max-w-xl min-w-0 items-center gap-1 rounded-md border border-border/60 bg-card/50"
  >
    <button
      type="button"
      class="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left text-xs outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
      :aria-label="`Open thread ${thread.name}`"
      @click="emit('open', thread.channelId)"
    >
      <MessagesSquare class="size-3.5 shrink-0 text-primary" aria-hidden="true" />
      <span class="min-w-0 max-w-40 shrink-0 truncate font-semibold">{{ thread.name }}</span>
      <span class="shrink-0 font-mono tabular-nums text-muted-foreground">
        {{ messageCountLabel(thread.messageCount) }}
      </span>
      <span v-if="last" class="min-w-0 flex-1 truncate text-muted-foreground">
        {{ last.author }}: {{ last.preview }}
      </span>
      <span v-else class="flex-1" />
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
