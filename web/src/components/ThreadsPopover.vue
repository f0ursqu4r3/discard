<script setup lang="ts">
import { ref } from 'vue'
import { toast } from 'vue-sonner'
import { Bot, BellOff, BellPlus, MessagesSquare } from '@lucide/vue'
import type { ThreadChipDto } from '../bridge/types'
import { followThread, leaveThread } from '../bridge/api'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
import ThreadStateMark from './ThreadStateMark.vue'
import { messageCountLabel, pendingApprovalsLabel } from './threads'
import { threadActionLabels } from './thread-menu'

/** The Threads list of a channel: every thread, followed or not, in the order that the app gives. */
defineProps<{ threads: ThreadChipDto[] }>()
const emit = defineEmits<{ open: [threadId: string] }>()
const open = ref(false)
function show(id: string) {
  open.value = false
  emit('open', id)
}
async function change(thread: ThreadChipDto) {
  try {
    await (thread.followed ? leaveThread : followThread)(thread.channelId)
  } catch (error) {
    toast.error(thread.followed ? 'Could not leave thread' : 'Could not follow thread', {
      description: String(error),
    })
  }
}
</script>
<template>
  <Popover v-model:open="open">
    <TooltipProvider :delay-duration="500" :skip-delay-duration="0">
      <Tooltip>
        <TooltipTrigger as-child>
          <PopoverTrigger as-child>
            <Button
              data-ui-id="threads-list-button"
              variant="ghost"
              size="icon-sm"
              :aria-label="threadActionLabels.list"
            >
              <MessagesSquare aria-hidden="true" />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>{{ threadActionLabels.list }}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
    <PopoverContent
      data-ui-id="threads-list"
      align="end"
      class="w-96 max-w-[calc(100vw-1rem)] gap-0 p-0"
    >
      <h3 class="border-b border-border/60 px-3 py-2 text-xs font-semibold">
        {{ threadActionLabels.list }}
        <span class="ml-1 font-mono font-normal tabular-nums text-muted-foreground">
          {{ threads.length }}
        </span>
      </h3>
      <p v-if="!threads.length" class="px-3 py-4 text-sm text-muted-foreground">No threads yet.</p>
      <ul v-else class="flex max-h-80 flex-col overflow-y-auto py-1">
        <li
          v-for="thread in threads"
          :key="thread.channelId"
          data-ui-id="threads-list-row"
          :data-thread-id="thread.channelId"
          :data-kind="thread.kind"
          class="flex min-w-0 items-center gap-1 pr-1.5"
        >
          <button
            type="button"
            class="flex min-w-0 flex-1 items-start gap-2 rounded-md px-3 py-1.5 text-left text-xs outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
            :aria-label="`Open thread ${thread.name}`"
            @click="show(thread.channelId)"
          >
            <Bot
              v-if="thread.kind === 'sub_agent'"
              class="mt-0.5 size-3.5 shrink-0 text-primary"
              aria-hidden="true"
            />
            <MessagesSquare
              v-else
              class="mt-0.5 size-3.5 shrink-0 text-primary"
              aria-hidden="true"
            />
            <span class="flex min-w-0 flex-1 flex-col gap-0.5">
              <span class="flex min-w-0 items-center gap-1.5">
                <span class="min-w-0 truncate font-semibold">{{ thread.name }}</span>
                <span
                  v-if="thread.unread"
                  data-ui-id="threads-list-unread"
                  role="img"
                  aria-label="Unread messages"
                  class="size-2 shrink-0 rounded-full bg-primary"
                />
                <Badge
                  v-if="thread.pendingApprovals > 0"
                  :aria-label="pendingApprovalsLabel(thread.pendingApprovals)"
                  class="h-4 min-w-4 shrink-0 rounded-full px-1 font-mono text-xs tabular-nums"
                >
                  {{ thread.pendingApprovals }}
                </Badge>
              </span>
              <span
                class="flex min-w-0 items-center gap-1.5 font-mono text-muted-foreground"
                data-ui-id="threads-list-meta"
              >
                <span data-ui-id="threads-list-kind">
                  {{ thread.kind === 'sub_agent' ? 'Sub-agent' : 'Question' }}
                </span>
                <ThreadStateMark v-if="thread.state" :state="thread.state" />
                <span class="tabular-nums">{{ messageCountLabel(thread.messageCount) }}</span>
              </span>
            </span>
          </button>
          <Button
            variant="ghost"
            size="xs"
            class="shrink-0"
            :aria-label="`${thread.followed ? 'Leave' : 'Follow'} thread ${thread.name}`"
            @click="change(thread)"
          >
            <BellOff v-if="thread.followed" data-icon="inline-start" aria-hidden="true" />
            <BellPlus v-else data-icon="inline-start" aria-hidden="true" />
            {{ thread.followed ? 'Leave' : 'Follow' }}
          </Button>
        </li>
      </ul>
    </PopoverContent>
  </Popover>
</template>
