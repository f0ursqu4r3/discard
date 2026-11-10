<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from 'vue'
import { MessageSquare } from '@lucide/vue'
import type { BlockDto, MessageDto, MessageAuthorDto, PromptDto } from '@/bridge/types'
import type { TranscriptPosition } from './transcript'
import { useMessageScrollerContext } from '../ui/message-scroller/useMessageScroller'
import { markRead } from '@/bridge/api'
import {
  MessageScroller,
  MessageScrollerViewport,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerButton,
  useMessageScrollerScrollable,
  useMessageScrollerVisibility,
} from '../ui/message-scroller'
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle } from '../ui/empty'
import MessageRow from '../MessageRow.vue'
import { useAppearance } from '@/bridge/appearance'
import WorktreeApprovals from './WorktreeApprovals.vue'
import WriteAccessApprovals from './WriteAccessApprovals.vue'
import MemoryResponse from '../memory/MemoryResponse.vue'
import MemoryCaptureStatus from '../memory/MemoryCaptureStatus.vue'
import { useMemory } from '../memory/useMemory'
import { Button } from '../ui/button'
import { hasLiveRow, rowCount, messagePresentation, messageDateLabel } from '../messages'
const props = defineProps<{
  messages: MessageDto[]
  live: BlockDto[]
  liveAuthor?: MessageAuthorDto | null
  running: boolean
  newAnchorId: string | null
  agentId?: string
  agent?: string
  agentAvatar?: string | null
  prompts?: PromptDto[]
  memoryRevision?: number
  conversationId: string
  projectId?: string | null
  jumpTo?: { messageId: string; request: number }
  position?: TranscriptPosition
  quoteDisabled?: boolean
}>()
const appearance = useAppearance()
const memory = useMemory(
  () => props.conversationId || null,
  () => !!props.conversationId,
)
watch([() => props.messages.at(-1)?.id, () => props.running, () => props.memoryRevision], () => {
  void memory.reload()
})
const emit = defineEmits<{
  savePosition: [id: string, position: TranscriptPosition]
  quote: [message: MessageDto]
}>()
const host = ref<HTMLElement | null>(null)
const scroller = useMessageScrollerContext()
let restoreFrame = 0
watch(
  () => props.jumpTo,
  async (jump) => {
    if (!jump) return
    await nextTick()
    cancelAnimationFrame(restoreFrame)
    restoreFrame = requestAnimationFrame(() => {
      scroller.userScrollIntent()
      scroller.scrollToMessage(jump.messageId, {
        align: 'center',
        behavior: 'auto',
      })
      scroller.syncAfterScroll()
    })
  },
  { immediate: true, flush: 'post' },
)
const scrollable = useMessageScrollerScrollable()
const visibility = useMessageScrollerVisibility()
const historyRows = computed(() =>
  props.messages.map((message, index) => {
    const presentation = messagePresentation(message, props.messages[index - 1])
    return { message: { ...message, continuesGroup: presentation.continuesGroup }, ...presentation }
  }),
)
const liveStartedAt = ref<number | null>(null)
watch(
  [() => props.conversationId, () => hasLiveRow(props.running, props.live.length)],
  ([, visible]) => {
    liveStartedAt.value = visible ? Date.now() : null
  },
  { immediate: true, flush: 'sync' },
)
const liveMessage = computed<MessageDto>(() => ({
  id: 'live',
  createdAt: liveStartedAt.value,
  role: 'agent',
  blocks: props.live,
  author: props.liveAuthor,
  continuesGroup:
    props.messages.at(-1)?.role === 'agent' &&
    props.messages.at(-1)?.author?.agentId === props.liveAuthor?.agentId,
}))
const livePresentation = computed(() =>
  messagePresentation(liveMessage.value, props.messages.at(-1)),
)
let lastRead: string | undefined
function read() {
  const last = props.messages.at(-1)
  if (
    !last ||
    lastRead === last.id ||
    scrollable.value.end ||
    !document.hasFocus() ||
    !visibility.value.visibleMessageIds.includes(last.id)
  )
    return
  lastRead = last.id
  void markRead(props.conversationId ?? '', last.id).catch(() => {
    lastRead = undefined
  })
}
watch([scrollable, visibility, () => props.messages.at(-1)?.id], read, {
  flush: 'post',
})
onMounted(async () => {
  window.addEventListener('focus', read)
  if (!props.position || props.jumpTo) return
  await nextTick()
  restoreFrame = requestAnimationFrame(() => {
    const viewport = host.value?.querySelector<HTMLElement>(
      '[data-slot="message-scroller-viewport"]',
    )
    const position = props.position
    if (!viewport || !position) return
    if (position.following) {
      scroller.scrollToEnd()
      return
    }
    if (
      position.messageId &&
      scroller.scrollToMessage(position.messageId, {
        align: 'start',
        behavior: 'auto',
      })
    ) {
      const anchor = [...viewport.querySelectorAll<HTMLElement>('[data-message-id]')].find(
        (item) => item.dataset.messageId === position.messageId,
      )
      if (anchor)
        viewport.scrollTop +=
          anchor.getBoundingClientRect().top -
          viewport.getBoundingClientRect().top -
          position.offset
    } else viewport.scrollTop = position.top
    scroller.userScrollIntent()
    scroller.syncAfterScroll()
  })
})
onBeforeUnmount(() => {
  cancelAnimationFrame(restoreFrame)
  const viewport = host.value?.querySelector<HTMLElement>('[data-slot="message-scroller-viewport"]')
  if (!viewport || !props.conversationId) return
  const top = viewport.getBoundingClientRect().top
  const item = [...viewport.querySelectorAll<HTMLElement>('[data-message-id]')].find(
    (element) => element.getBoundingClientRect().bottom > top,
  )
  emit('savePosition', props.conversationId, {
    messageId: item?.dataset.messageId ?? null,
    offset: item ? item.getBoundingClientRect().top - top : 0,
    top: viewport.scrollTop,
    following: !scrollable.value.end,
  })
})
onUnmounted(() => window.removeEventListener('focus', read))
</script>
<template>
  <div
    ref="host"
    data-ui-id="message-view"
    class="relative flex min-h-0 flex-1 flex-col"
    :data-row-count="rowCount(messages.length, hasLiveRow(running, live.length))"
  >
    <div v-if="memory.data.value.paused" class="mx-6 my-2">
      <MemoryCaptureStatus :channel-id="conversationId" :paused="true" @changed="memory.reload" />
    </div>
    <p v-if="memory.error.value" role="alert" class="mx-6 text-xs">
      Memory could not load: {{ memory.error.value }}
    </p>
    <div v-if="memory.data.value.warnings?.length" class="mx-6 space-y-2 text-xs">
      <p v-for="warning in memory.data.value.warnings" :key="warning" role="status">
        {{ warning }}
      </p>
      <Button size="sm" variant="outline" @click="memory.reload">Retry memory notice</Button>
    </div>
    <MessageScroller>
      <MessageScrollerViewport
        data-ui-id="message-viewport"
        class="outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring"
        :preserve-scroll-on-prepend="true"
      >
        <MessageScrollerContent
          class="py-2"
          :class="appearance.messageDensity === 'compact' ? 'gap-0' : 'gap-1'"
        >
          <MessageScrollerItem
            class="[content-visibility:visible]"
            v-for="{ message, date, startsDate } in historyRows"
            :key="message.id"
            :message-id="message.id"
            :data-search-match="message.id === jumpTo?.messageId || undefined"
            :class="{
              'bg-accent ring-1 ring-inset ring-ring': message.id === jumpTo?.messageId,
            }"
            :scroll-anchor="message.role === 'user'"
          >
            <div
              v-if="startsDate && date"
              data-testid="date-divider"
              role="separator"
              :aria-label="messageDateLabel(date)"
              class="my-4 flex items-center gap-3 px-3 text-xs font-semibold text-muted-foreground sm:px-6"
            >
              <span class="flex-1 border-t border-border" />
              <time :datetime="date.toLocaleDateString('en-CA')">{{ messageDateLabel(date) }}</time>
              <span class="flex-1 border-t border-border" />
            </div>
            <div
              v-if="message.id === newAnchorId"
              data-testid="new-divider"
              class="my-3 flex items-center gap-2 px-6 text-xs font-medium text-primary"
            >
              <span class="flex-1 border-t border-primary" />
              New
              <span class="w-4 border-t border-primary" />
            </div>
            <MessageRow
              :channel-id="conversationId"
              :message="message"
              :quote-disabled="quoteDisabled"
              @quote="emit('quote', $event)"
              :agent="agent"
              :agent-id="agentId"
              :agent-avatar="agentAvatar"
              :prompts="prompts"
            />
            <MemoryResponse
              :records="memory.data.value.responses.filter((r) => r.responseId === message.id)"
              :proposals="memory.data.value.proposals"
              :channel-id="conversationId"
              @changed="memory.reload"
            />
          </MessageScrollerItem>
          <MessageScrollerItem
            class="[content-visibility:visible]"
            v-if="hasLiveRow(running, live.length)"
            message-id="streaming-reply"
          >
            <div
              v-if="livePresentation.startsDate && livePresentation.date"
              data-testid="date-divider"
              role="separator"
              :aria-label="messageDateLabel(livePresentation.date)"
              class="my-4 flex items-center gap-3 px-3 text-xs font-semibold text-muted-foreground sm:px-6"
            >
              <span class="flex-1 border-t border-border" />
              <time :datetime="livePresentation.date.toLocaleDateString('en-CA')">
                {{ messageDateLabel(livePresentation.date) }}
              </time>
              <span class="flex-1 border-t border-border" />
            </div>
            <MessageRow
              :channel-id="conversationId"
              :message="{ ...liveMessage, continuesGroup: livePresentation.continuesGroup }"
              :agent="agent"
              :agent-id="agentId"
              :agent-avatar="agentAvatar"
              :prompts="prompts"
              :show-cursor="running"
            />
          </MessageScrollerItem>
          <WorktreeApprovals v-if="projectId" :channel-id="conversationId" :running="running" />
          <WriteAccessApprovals v-if="projectId" :channel-id="conversationId" :running="running" />
          <Empty v-if="!messages.length && !hasLiveRow(running, live.length)" class="m-auto">
            <EmptyHeader>
              <EmptyMedia variant="icon"><MessageSquare /></EmptyMedia>
              <EmptyTitle>No messages yet</EmptyTitle>
            </EmptyHeader>
          </Empty>
        </MessageScrollerContent>
      </MessageScrollerViewport>
      <MessageScrollerButton aria-label="Jump to latest" title="Jump to latest" />
    </MessageScroller>
  </div>
</template>
