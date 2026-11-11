<script setup lang="ts">
import type { BlockDto, MessageDto, MessageAuthorDto, PromptDto } from '../bridge/types'
import { MessageScrollerProvider } from './ui/message-scroller'
import type { TranscriptPosition } from './workspace/transcript'
import TranscriptContent from './workspace/TranscriptContent.vue'
defineProps<{
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
  surface?: 'channel' | 'thread'
  anchor?: MessageDto | null
  anchorChannelId?: string
  threadDisabled?: boolean
}>()
const emit = defineEmits<{
  savePosition: [id: string, position: TranscriptPosition]
  quote: [message: MessageDto]
  startThread: [message: MessageDto]
  openThread: [threadId: string]
}>()
</script>
<template>
  <MessageScrollerProvider
    auto-scroll
    default-scroll-position="last-anchor"
    :scroll-previous-item-peek="64"
  >
    <TranscriptContent
      v-bind="$props"
      @quote="emit('quote', $event)"
      @start-thread="emit('startThread', $event)"
      @open-thread="emit('openThread', $event)"
      @save-position="(id, position) => emit('savePosition', id, position)"
    />
  </MessageScrollerProvider>
</template>
