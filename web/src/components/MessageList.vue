<script setup lang="ts">
import type {
  BlockDto,
  MessageDto,
  MessageAuthorDto,
  PromptDto,
  ThreadChipDto,
} from '../bridge/types'
import type { AgentRequest } from '../bridge/api'
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
  replyDisabled?: boolean
  surface?: 'channel' | 'thread'
  anchor?: MessageDto | null
  anchorChannelId?: string
  threadDisabled?: boolean
  liveSubAgentThreads?: ThreadChipDto[]
  agentRequests?: AgentRequest[]
  agentRequestBusy?: string | null
}>()
const emit = defineEmits<{
  savePosition: [id: string, position: TranscriptPosition]
  reply: [message: MessageDto]
  startThread: [message: MessageDto]
  openThread: [threadId: string]
  decideAgentRequest: [requestId: string, decision: 'add' | 'deny']
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
      @reply="emit('reply', $event)"
      @start-thread="emit('startThread', $event)"
      @open-thread="emit('openThread', $event)"
      @decide-agent-request="(id, decision) => emit('decideAgentRequest', id, decision)"
      @save-position="(id, position) => emit('savePosition', id, position)"
    />
  </MessageScrollerProvider>
</template>
