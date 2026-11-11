<script setup lang="ts">
import { computed } from 'vue'
import { Bot, MessagesSquare } from '@lucide/vue'
import type { ThreadLinkDto } from '../bridge/types'
import { openContextMenuFromKeyboard } from './context-menu-keyboard'
import { treePrefix } from './branch-navigation'
import { threadActionLabels, threadDeleteBlock } from './thread-menu'
import { requestAction } from './thread-actions'
import { pendingApprovalsLabel, threadStates } from './threads'
import BrailleSpinner from './BrailleSpinner.vue'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from './ui/context-menu'

const props = defineProps<{
  link: ThreadLinkDto
  /** The thread panel shows this thread. */
  active?: boolean
  isLast?: boolean
  busy?: boolean
}>()
const emit = defineEmits<{ select: [threadId: string]; leave: [threadId: string] }>()
const subAgent = computed(() => props.link.kind === 'sub_agent')
const running = computed(() => props.link.running || props.link.state === 'running')
// A running turn or task, or a pending approval, blocks a delete for now. This is a state, not a planned action.
const deleteBlock = computed(() =>
  threadDeleteBlock({
    running: props.link.running,
    state: props.link.state,
    pendingApprovals: props.link.pendingApprovals,
  }),
)
// A finished run that did not succeed keeps a mark. A done run is quiet, like a thread with no news.
const endState = computed(() => {
  const state = props.link.state
  return state && state !== 'running' && state !== 'done' ? threadStates[state] : null
})
</script>
<template>
  <div
    data-ui-id="thread-row"
    :data-thread-id="link.id"
    :data-kind="link.kind"
    class="flex min-w-0 items-center gap-1 pl-3"
  >
    <span
      aria-hidden="true"
      data-testid="tree-prefix"
      class="w-[1ch] shrink-0 font-mono text-muted-foreground/70"
    >
      {{ treePrefix(!!isLast) }}
    </span>
    <ContextMenu>
      <ContextMenuTrigger as-child @keydown="openContextMenuFromKeyboard">
        <Button
          variant="ghost"
          :aria-current="active ? 'page' : undefined"
          :disabled="busy"
          :title="link.name"
          class="flex h-8 min-w-0 flex-1 items-center justify-start gap-2 rounded-md px-1 text-left text-xs outline-none hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring"
          :class="
            active
              ? 'nav-active-edge bg-sidebar-accent text-sidebar-accent-foreground'
              : 'text-muted-foreground'
          "
          @click="emit('select', link.id)"
        >
          <Bot v-if="subAgent" class="size-3.5 shrink-0" aria-hidden="true" />
          <MessagesSquare v-else class="size-3.5 shrink-0" aria-hidden="true" />
          <span class="min-w-0 flex-1 truncate">{{ link.name }}</span>
          <BrailleSpinner v-if="running" class="shrink-0" />
          <component
            :is="endState.icon"
            v-else-if="endState?.icon"
            role="img"
            :aria-label="endState.label"
            class="size-3 shrink-0"
            :class="endState.class"
          />
          <Badge
            v-if="link.pendingApprovals > 0"
            :aria-label="pendingApprovalsLabel(link.pendingApprovals)"
            class="h-4 min-w-4 shrink-0 rounded-full px-1 font-mono text-xs tabular-nums"
          >
            {{ link.pendingApprovals }}
          </Badge>
          <span
            v-if="link.unread"
            data-ui-id="channel-unread-pip"
            role="img"
            aria-label="Unread messages"
            class="size-2 shrink-0 rounded-full bg-sidebar-foreground"
          />
        </Button>
      </ContextMenuTrigger>
      <ContextMenuContent data-ui-id="thread-context-menu" class="min-w-56" :collision-padding="8">
        <ContextMenuItem :disabled="busy" @select="!busy && emit('select', link.id)">
          Open thread
        </ContextMenuItem>
        <ContextMenuItem @select="emit('leave', link.id)">Leave thread</ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          @select="requestAction({ kind: 'rename-thread', threadId: link.id, name: link.name })"
        >
          {{ threadActionLabels.rename }}
        </ContextMenuItem>
        <ContextMenuItem
          variant="destructive"
          :disabled="!!deleteBlock"
          :title="deleteBlock ?? undefined"
          @select="
            !deleteBlock &&
            requestAction({ kind: 'delete-thread', threadId: link.id, name: link.name })
          "
        >
          {{ threadActionLabels.remove }}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  </div>
</template>
