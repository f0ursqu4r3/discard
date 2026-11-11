<script setup lang="ts">
import { MessagesSquare } from '@lucide/vue'
import type { ThreadLinkDto } from '../bridge/types'
import { openContextMenuFromKeyboard } from './context-menu-keyboard'
import { treePrefix } from './branch-navigation'
import { threadMenuPlanned } from './thread-menu'
import BrailleSpinner from './BrailleSpinner.vue'
import ProjectMenuItems from './ProjectMenuItems.vue'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from './ui/context-menu'

defineProps<{
  link: ThreadLinkDto
  /** The thread panel shows this thread. */
  active?: boolean
  isLast?: boolean
  busy?: boolean
}>()
const emit = defineEmits<{ select: [threadId: string]; leave: [threadId: string] }>()
</script>
<template>
  <div
    data-ui-id="thread-row"
    :data-thread-id="link.id"
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
          <MessagesSquare class="size-3.5 shrink-0" aria-hidden="true" />
          <span class="min-w-0 flex-1 truncate">{{ link.name }}</span>
          <BrailleSpinner v-if="link.running" class="shrink-0" />
          <Badge
            v-if="link.pendingApprovals > 0"
            :aria-label="`${link.pendingApprovals} pending ${link.pendingApprovals === 1 ? 'approval' : 'approvals'}`"
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
        <ProjectMenuItems :entries="threadMenuPlanned" />
      </ContextMenuContent>
    </ContextMenu>
  </div>
</template>
