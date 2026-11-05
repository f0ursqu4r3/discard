<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from 'vue'
import { MessageSquare } from '@lucide/vue'
import type { BlockDto, MessageDto, PromptDto } from '@/bridge/types'
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
import { Empty, EmptyHeader, EmptyMedia, EmptyTitle, EmptyDescription } from '../ui/empty'
import MessageRow from '../MessageRow.vue'
import { hasLiveRow, rowCount } from '../messages'
const props = defineProps<{
  messages: MessageDto[]
  live: BlockDto[]
  running: boolean
  newAnchorId: string | null
  agent?: string
  prompts?: PromptDto[]
  conversationId: string
  position?: TranscriptPosition
}>()
const emit = defineEmits<{ savePosition: [id: string, position: TranscriptPosition] }>()
const host = ref<HTMLElement | null>(null)
const scroller = useMessageScrollerContext()
let restoreFrame = 0
const scrollable = useMessageScrollerScrollable()
const visibility = useMessageScrollerVisibility()
const liveMessage = computed<MessageDto>(() => ({
  id: 'live',
  role: 'agent',
  blocks: props.live,
  continuesGroup: props.messages.at(-1)?.role === 'agent',
}))
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
watch([scrollable, visibility, () => props.messages.at(-1)?.id], read, { flush: 'post' })
onMounted(async () => {
  window.addEventListener('focus', read)
  if (!props.position) return
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
      scroller.scrollToMessage(position.messageId, { align: 'start', behavior: 'auto' })
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
    class="relative min-h-0 flex-1"
    :data-row-count="rowCount(messages.length, hasLiveRow(running, live.length))"
  >
    <MessageScroller>
      <MessageScrollerViewport
        data-ui-id="message-viewport"
        class="outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring"
        :preserve-scroll-on-prepend="true"
      >
        <MessageScrollerContent class="gap-1 py-5">
          <MessageScrollerItem
            class="[content-visibility:visible]"
            v-for="message in messages"
            :key="message.id"
            :message-id="message.id"
            :scroll-anchor="message.role === 'user'"
          >
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
              :agent="agent"
              :prompts="prompts"
            />
          </MessageScrollerItem>
          <MessageScrollerItem
            class="[content-visibility:visible]"
            v-if="hasLiveRow(running, live.length)"
            message-id="streaming-reply"
          >
            <MessageRow
              :channel-id="conversationId"
              :message="liveMessage"
              :agent="agent"
              :prompts="prompts"
              show-cursor
            />
          </MessageScrollerItem>
          <Empty v-if="!messages.length && !hasLiveRow(running, live.length)" class="m-auto">
            <EmptyHeader>
              <EmptyMedia variant="icon"><MessageSquare /></EmptyMedia>
              <EmptyTitle>No messages yet</EmptyTitle>
              <EmptyDescription>Send a message to {{ agent || 'your agent' }}.</EmptyDescription>
            </EmptyHeader>
          </Empty>
        </MessageScrollerContent>
      </MessageScrollerViewport>
      <MessageScrollerButton aria-label="Jump to latest" title="Jump to latest" />
    </MessageScroller>
  </div>
</template>
