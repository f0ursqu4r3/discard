<script setup lang="ts">
import type { MessageDto, PromptDto } from '../bridge/types'
import { Avatar, AvatarFallback } from './ui/avatar'
import { Marker, MarkerContent, MarkerIcon } from './ui/marker'
import {
  Info,
  Copy,
  Quote,
  MessagesSquare,
  Forward,
  CircleCheck,
  CornerDownRight,
  Trash,
} from '@lucide/vue'
import { toast } from 'vue-sonner'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip'
import AgentAvatar from './agents/AgentAvatar.vue'
import { useAgentIdentity } from './agents/useAgentIdentity'
import { Button } from './ui/button'
import BlockView from './BlockView.vue'
import ToolCallGroup from './ToolCallGroup.vue'
import ContextMenuHost from './ContextMenuHost.vue'
import ThreadChip from './ThreadChip.vue'
import { postThreadMessage, runAction } from '../bridge/api'
import { computed, ref } from 'vue'
import { useAppearance } from '@/bridge/appearance'
import { serverInitials } from './layout'
import { groupBlocks, messageTime } from './messages'
import { threadAction } from './threads'
import { referenceMenuPlanned, threadActions } from './thread-menu'
const props = defineProps<{
  channelId: string
  message: MessageDto
  agentId?: string
  agent?: string
  agentAvatar?: string | null
  prompts?: PromptDto[]
  showCursor?: boolean
  preview?: boolean
  quoteDisabled?: boolean
  /**
   * Where the row shows. A channel row can start or open a thread. A thread row can post to the
   * channel. The anchor row at the top of a thread panel has neither.
   */
  surface?: 'channel' | 'thread' | 'anchor'
  /** The channel is read only: no new thread and no post to the channel. */
  threadDisabled?: boolean
}>()
const emit = defineEmits<{
  quote: [message: MessageDto]
  startThread: [message: MessageDto]
  openThread: [threadId: string]
}>()
function quote() {
  if (target.value?.hasText && !props.quoteDisabled) emit('quote', props.message)
}
const segments = computed(() => groupBlocks(props.message.blocks))
const date = computed(() => messageTime(props.message))
const timeLabel = computed(() =>
  date.value?.toLocaleTimeString(undefined, {
    hour: 'numeric',
    minute: '2-digit',
  }),
)
const appearance = useAppearance()
const compact = computed(() => appearance.value.messageDensity === 'compact')
// Runtime system notes are plain text. Keep other block types on the full renderer.
const systemNote = computed(
  () =>
    props.message.role === 'system' &&
    props.message.blocks.every((block) => block.type === 'markdown'),
)
const systemText = computed(() =>
  props.message.blocks
    .flatMap((block) => (block.type === 'markdown' ? [block.text] : []))
    .join('\n'),
)
const authorName = computed(() => props.message.author?.name ?? props.agent ?? 'Agent')
const authorAvatar = computed(() =>
  props.message.author ? props.message.author.avatar : props.agentAvatar,
)
const identity = useAgentIdentity(
  () => props.message.author?.agentId ?? props.agentId,
  authorAvatar,
)
const target = computed(() =>
  props.preview || props.message.id === 'live'
    ? null
    : {
        type: 'message' as const,
        id: props.message.id,
        hasText: props.message.blocks.some((block) => 'text' in block && !!block.text),
      },
)
const surface = computed(() => props.surface ?? 'channel')
// A streaming row has no target, so its thread actions are not offered until it is saved.
const threadItem = computed(() => {
  if (!target.value || surface.value !== 'channel') return undefined
  const action = threadAction(props.message)
  return {
    ...action,
    disabled: action.kind === 'start' && !!props.threadDisabled,
  }
})
const posting = ref(false)
const postItem = computed(() =>
  target.value && surface.value === 'thread'
    ? {
        posted: !!props.message.postedToChannel,
        disabled: !!props.threadDisabled || posting.value,
      }
    : undefined,
)
const reference = computed(() => (surface.value === 'channel' ? props.message.postedFrom : null))
const plannedEntries = computed(() => (reference.value ? referenceMenuPlanned : undefined))
function openThread() {
  const item = threadItem.value
  if (!item || item.disabled) return
  if (item.kind === 'open') emit('openThread', item.threadId)
  else emit('startThread', props.message)
}
async function post() {
  if (!postItem.value || postItem.value.posted || postItem.value.disabled) return
  posting.value = true
  try {
    await postThreadMessage(props.message.id)
  } catch (error) {
    toast.error('Could not post to channel', { description: String(error) })
  } finally {
    posting.value = false
  }
}
async function copy() {
  if (!target.value) return
  const text = await runAction('copy_message_text', target.value)
  if (text !== null) await navigator.clipboard.writeText(text)
}
</script>
<template>
  <ContextMenuHost
    :target="target"
    :quote-enabled="!!target?.hasText && !quoteDisabled"
    :thread-action="threadItem"
    :post-action="postItem"
    :planned-entries="plannedEntries"
    @quote="quote"
    @thread="openThread"
    @post="post"
  >
    <Marker
      v-if="systemNote"
      as="article"
      :data-ui-id="`message-${message.id}`"
      aria-label="System note"
      class="px-3 sm:px-6"
      :class="compact ? 'py-1' : 'py-2'"
    >
      <MarkerIcon><Info /></MarkerIcon>
      <MarkerContent class="whitespace-pre-wrap">
        {{ systemText }}
      </MarkerContent>
    </Marker>
    <article
      v-else
      :data-ui-id="`message-${message.id}`"
      :tabindex="target ? 0 : undefined"
      :style="message.role === 'agent' ? { '--agent-color': identity.mark } : undefined"
      class="group relative flex gap-2 px-3 text-sm leading-relaxed hover:bg-muted/30 sm:px-6"
      :class="compact ? 'py-1' : 'py-2'"
    >
      <div
        data-testid="message-time-gutter"
        class="w-[8ch] shrink-0 text-right font-mono text-xs leading-[1.625rem] tabular-nums text-muted-foreground"
      >
        <time
          v-if="date && !message.continuesGroup"
          data-testid="message-timestamp"
          :datetime="date.toISOString()"
          :title="date.toLocaleString()"
          class="whitespace-nowrap"
        >
          {{ timeLabel }}
        </time>
      </div>
      <div
        v-if="appearance.showMessageAvatars"
        data-testid="message-avatar"
        class="w-5 shrink-0 pt-0.5"
      >
        <AgentAvatar
          :agent-id="message.author?.agentId ?? agentId"
          v-if="!message.continuesGroup && message.role === 'agent'"
          :name="authorName"
          :avatar="authorAvatar"
          class="size-5"
        />
        <Avatar v-else-if="!message.continuesGroup" class="size-5">
          <AvatarFallback class="bg-primary/20 text-primary">
            {{ serverInitials(message.role === 'user' ? 'You' : agent || message.role) }}
          </AvatarFallback>
        </Avatar>
      </div>
      <div class="min-w-0 flex-1">
        <button
          v-if="reference"
          type="button"
          data-ui-id="posted-from-thread"
          :aria-label="`From thread ${reference.threadName}`"
          class="mb-0.5 flex max-w-full items-center gap-1 rounded-sm text-xs text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none"
          @click="emit('openThread', reference.threadChannelId)"
        >
          <CornerDownRight class="size-3 shrink-0" aria-hidden="true" />
          <span class="shrink-0">from thread</span>
          <span class="min-w-0 truncate font-medium">{{ reference.threadName }}</span>
        </button>
        <strong
          data-testid="message-identity"
          class="mr-2 font-semibold"
          :style="message.role === 'agent' ? { color: identity.text } : undefined"
        >
          {{ message.role === 'user' ? 'You' : message.role === 'agent' ? authorName : 'System' }}
        </strong>
        <span
          v-if="message.role === 'agent' && message.author?.provider"
          data-testid="message-agent-tag"
          class="mr-2 font-mono text-[0.92em] text-muted-foreground"
        >
          [agent · {{ message.author.provider }}]
        </span>
        <template
          v-for="segment in segments"
          :key="segment.kind === 'tools' ? segment.items[0]!.index : segment.index"
        >
          <ToolCallGroup
            v-if="segment.kind === 'tools'"
            :channel-id="channelId"
            :items="segment.items"
            :running="message.id === 'live' && showCursor"
            :prompts="prompts"
            :message-id="target?.id"
          />
          <BlockView
            v-else
            :channel-id="channelId"
            :block="segment.block"
            :inline="segment.index === 0"
            :running="message.id === 'live' && showCursor"
            :prompts="prompts"
            :message-id="target?.id"
            :index="segment.index"
          />
        </template>
        <span
          v-if="showCursor"
          data-testid="stream-cursor"
          aria-hidden="true"
          class="stream-cursor inline-block h-[1.1em] w-[0.55em] align-[-0.2em] bg-foreground animate-[cursor-blink_1.05s_steps(1)_infinite]"
        />
        <ThreadChip
          v-if="message.thread && surface === 'channel'"
          :thread="message.thread"
          @open="emit('openThread', $event)"
        />
      </div>
      <TooltipProvider v-if="target" :delay-duration="500" :skip-delay-duration="0">
        <!-- Keep button transitions off visibility so the toolbar hides as one unit. -->
        <div
          role="group"
          aria-label="Message actions"
          class="absolute right-0 top-0 -translate-y-3/4 z-10 invisible flex gap-0.5 rounded-md border bg-background p-1 shadow-sm group-hover:visible group-focus-within:visible"
        >
          <Tooltip>
            <TooltipTrigger as-child>
              <Button
                :disabled="!target.hasText"
                variant="ghost"
                size="icon-xs"
                class="transition-colors"
                aria-label="Copy message"
                @click="copy"
              >
                <Copy aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Copy message</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger as-child>
              <Button
                :disabled="quoteDisabled || !target.hasText"
                variant="ghost"
                size="icon-xs"
                class="transition-colors"
                aria-label="Quote message"
                @click="quote"
              >
                <Quote aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Quote message</TooltipContent>
          </Tooltip>
          <Tooltip v-if="threadItem">
            <TooltipTrigger as-child>
              <Button
                :disabled="threadItem.disabled"
                variant="ghost"
                size="icon-xs"
                class="transition-colors"
                :aria-label="threadItem.label"
                @click="openThread"
              >
                <MessagesSquare aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{{ threadItem.label }}</TooltipContent>
          </Tooltip>
          <Tooltip v-if="postItem">
            <TooltipTrigger as-child>
              <!-- The wrapper keeps the tooltip reachable while the button is disabled. -->
              <span class="inline-flex" :tabindex="postItem.posted ? 0 : undefined">
                <Button
                  :disabled="postItem.disabled || postItem.posted"
                  variant="ghost"
                  size="icon-xs"
                  class="transition-colors"
                  :aria-label="postItem.posted ? 'Posted to channel' : 'Post to channel'"
                  @click="post"
                >
                  <CircleCheck v-if="postItem.posted" aria-hidden="true" />
                  <Forward v-else aria-hidden="true" />
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>
              {{ postItem.posted ? 'Posted to channel' : 'Post to channel' }}
            </TooltipContent>
          </Tooltip>
          <Tooltip v-if="reference">
            <TooltipTrigger as-child>
              <span
                class="inline-flex"
                tabindex="0"
                :aria-label="`${threadActions.removeFromChannel.label} — Soon`"
                aria-disabled="true"
              >
                <!-- TODO: Remove the reference row from the channel, with rules for a parent agent that already read it. -->
                <Button
                  disabled
                  variant="ghost"
                  size="icon-xs"
                  class="transition-colors"
                  :aria-label="`${threadActions.removeFromChannel.label} — Soon`"
                >
                  <Trash aria-hidden="true" />
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>{{ threadActions.removeFromChannel.label }} · Soon</TooltipContent>
          </Tooltip>
        </div>
      </TooltipProvider>
    </article>
  </ContextMenuHost>
</template>
