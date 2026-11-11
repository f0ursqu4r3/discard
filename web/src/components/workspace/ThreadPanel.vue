<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { AtSign, BellOff, BellPlus, Bot, Ellipsis, Hash, MessagesSquare, X } from '@lucide/vue'
import type { AttachmentDto, MessageDto, OpenThreadDto, ThreadHistoryDto } from '@/bridge/types'
import { followThread, leaveThread } from '@/bridge/api'
import { thinkingLine } from '../messages'
import { threadMenuPlanned } from '../thread-menu'
import BrailleSpinner from '../BrailleSpinner.vue'
import Composer from '../Composer.vue'
import ThreadStateMark from '../ThreadStateMark.vue'
import MessageList from '../MessageList.vue'
import type { PendingReply } from '../reply'
import { Alert, AlertDescription } from '../ui/alert'
import { Button } from '../ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '../ui/tooltip'

const props = defineProps<{
  /** The open thread. Null while the panel shows a draft that has no thread yet. */
  thread: OpenThreadDto | null
  history: ThreadHistoryDto | null
  /** The anchor message of a draft. A draft has no thread row, no transcript and no name. */
  draftAnchor?: MessageDto | null
  parentChannelId: string
  parentName: string
  /** The parent channel's agent. Names the anchor of a draft, which has no thread agent yet. */
  parentAgentId?: string
  parentAgent?: string
  directMessage?: boolean
  projectId?: string | null
  agentAvatar?: string | null
  /** The parent channel cannot take messages. An open thread reports this itself. */
  composerDisabled?: boolean
  busy?: boolean
  replyDisabled?: boolean
  draft?: string
  attachments?: AttachmentDto[]
  /** The reply that waits for the thread composer. */
  reply?: PendingReply | null
  jumpTo?: { messageId: string; request: number }
}>()
const emit = defineEmits<{
  close: []
  send: [text: string, attachments: AttachmentDto[], alsoPost: boolean]
  stop: []
  cancelReply: []
  reply: [message: MessageDto]
  'update:draft': [text: string]
  'update:attachments': [attachments: AttachmentDto[]]
}>()
const host = ref<HTMLElement | null>(null)
const composer = ref<InstanceType<typeof Composer> | null>(null)
// A transcript from another thread does not show while the open thread loads.
const loaded = computed(() => !!props.thread && props.history?.channelId === props.thread.channelId)
const messages = computed(() => (loaded.value ? props.history!.messages : []))
const anchor = computed(() =>
  props.thread ? (loaded.value ? props.history!.anchor : null) : (props.draftAnchor ?? null),
)
const key = computed(() => props.thread?.channelId ?? `draft-${props.draftAnchor?.id ?? ''}`)
const disabled = computed(() =>
  props.thread ? props.thread.composerDisabled : !!props.composerDisabled,
)
const subAgent = computed(() => props.thread?.kind === 'sub_agent')
// While the task of a sub-agent runs, a message steers it: Send stays next to Stop.
const steering = computed(() => subAgent.value && props.thread?.state === 'running')
const placeholder = computed(() =>
  steering.value
    ? 'Steer this sub-agent'
    : props.thread
      ? 'Reply in thread'
      : 'Ask about this message',
)
const typing = computed(() =>
  thinkingLine(!!props.thread?.turnRunning, [
    props.thread?.responder?.name ?? props.thread?.agent ?? '',
  ]),
)
async function change(follow: boolean) {
  if (!props.thread) return
  try {
    await (follow ? followThread : leaveThread)(props.thread.channelId)
  } catch (error) {
    toast.error(follow ? 'Could not follow thread' : 'Could not leave thread', {
      description: String(error),
    })
  }
}
// Focus the composer when a thread or draft opens.
watch(
  key,
  async () => {
    await nextTick()
    void composer.value?.focusEnd?.()
  },
  { immediate: true, flush: 'post' },
)
defineExpose({ focusComposer: () => composer.value?.focusEnd?.() })
</script>
<template>
  <section
    ref="host"
    data-ui-id="thread-panel"
    aria-label="Thread"
    class="relative flex h-full min-h-0 min-w-0 flex-col overflow-hidden bg-workspace"
  >
    <header
      data-ui-id="thread-header"
      class="flex h-16 shrink-0 items-center gap-1 border-b border-border/60 pr-2 pl-4"
    >
      <Bot v-if="subAgent" class="mr-2 size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
      <MessagesSquare
        v-else
        class="mr-2 size-5 shrink-0 text-muted-foreground"
        aria-hidden="true"
      />
      <div class="min-w-0 flex-1">
        <h2 class="truncate text-sm font-semibold">{{ thread?.name ?? 'New thread' }}</h2>
        <p
          v-if="subAgent && thread"
          data-ui-id="thread-subagent-line"
          class="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground"
        >
          <span data-ui-id="thread-agent" class="min-w-0 truncate font-mono">
            {{ thread.agentName }}
          </span>
          <ThreadStateMark v-if="thread.state" :state="thread.state" :agent-id="thread.agentId" />
        </p>
        <p class="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
          <component
            :is="directMessage ? AtSign : Hash"
            class="size-3 shrink-0"
            aria-hidden="true"
          />
          <span class="truncate" :class="{ 'font-mono text-[0.92em]': !directMessage }">
            {{ parentName }}
          </span>
        </p>
      </div>
      <TooltipProvider :delay-duration="500" :skip-delay-duration="0">
        <Button
          v-if="thread"
          variant="ghost"
          size="sm"
          :aria-label="thread.followed ? 'Leave thread' : 'Follow thread'"
          @click="change(!thread.followed)"
        >
          <BellOff v-if="thread.followed" data-icon="inline-start" aria-hidden="true" />
          <BellPlus v-else data-icon="inline-start" aria-hidden="true" />
          {{ thread.followed ? 'Leave' : 'Follow' }}
        </Button>
        <DropdownMenu v-if="thread">
          <Tooltip>
            <TooltipTrigger as-child>
              <DropdownMenuTrigger as-child>
                <Button variant="ghost" size="icon-sm" aria-label="Thread menu">
                  <Ellipsis aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent>Thread menu</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="end" data-ui-id="thread-menu" class="w-48">
            <DropdownMenuItem v-for="entry in threadMenuPlanned" :key="entry.label" disabled>
              {{ entry.label }}
              <span class="ml-auto text-xs text-muted-foreground">Soon</span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <Tooltip>
          <TooltipTrigger as-child>
            <Button variant="ghost" size="icon-sm" aria-label="Close thread" @click="emit('close')">
              <X aria-hidden="true" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Close thread</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </header>
    <div
      v-if="thread && !loaded"
      role="status"
      class="flex min-h-0 flex-1 items-center justify-center gap-2 text-sm text-muted-foreground"
    >
      <BrailleSpinner label="Opening thread" />
      Opening thread…
    </div>
    <MessageList
      v-else
      :key="key"
      surface="thread"
      :conversation-id="thread?.channelId ?? ''"
      :project-id="null"
      :anchor="anchor"
      :anchor-channel-id="parentChannelId"
      :messages="messages"
      :live="thread?.liveBlocks ?? []"
      :live-author="thread?.responder"
      :running="!!thread?.turnRunning"
      :new-anchor-id="null"
      :agent-id="thread?.agentId ?? parentAgentId"
      :agent="thread?.agent ?? parentAgent"
      :agent-avatar="agentAvatar"
      :prompts="thread?.prompts"
      :reply-disabled="replyDisabled"
      :thread-disabled="disabled"
      :jump-to="jumpTo"
      @reply="emit('reply', $event)"
    />
    <Alert v-if="thread?.notice" class="mx-4 mb-2 min-w-0 w-auto border-warning text-foreground">
      <AlertDescription class="min-w-0 whitespace-pre-wrap [overflow-wrap:anywhere]">
        {{ thread.notice }}
      </AlertDescription>
    </Alert>
    <p v-if="typing" class="flex items-center gap-2 px-5 py-1 text-xs text-muted-foreground">
      <BrailleSpinner :agent-id="thread?.responder?.agentId ?? thread?.agentId" />
      {{ typing }}
    </p>
    <p
      v-if="thread?.readOnlyNote"
      data-ui-id="thread-read-only-note"
      class="px-5 pt-1 text-xs text-muted-foreground"
    >
      {{ thread.readOnlyNote }}
    </p>
    <p v-if="disabled" class="px-5 py-2 text-xs text-muted-foreground">Read only.</p>
    <Composer
      ref="composer"
      :key="key"
      :drop-target="host"
      :draft="draft"
      :attachments="attachments ?? []"
      :channel="parentName"
      :direct-message="directMessage"
      :project-id="projectId"
      :agent="thread?.agent"
      :archived="disabled"
      :busy="!!busy"
      :running="!!thread?.turnRunning"
      :send-while-running="steering"
      :placeholder="placeholder"
      :also-post-label="directMessage ? 'the conversation' : `#${parentName}`"
      @update:draft="emit('update:draft', $event)"
      @update:attachments="emit('update:attachments', $event)"
      @send="(text, files, alsoPost) => emit('send', text, files ?? [], !!alsoPost)"
      :reply="reply"
      @stop="emit('stop')"
      @cancel-reply="emit('cancelReply')"
    />
  </section>
</template>
