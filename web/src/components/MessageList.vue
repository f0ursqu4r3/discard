<script setup lang="ts">
import type { BlockDto, MessageDto, PromptDto } from '../bridge/types'
import { MessageScrollerProvider } from './ui/message-scroller'
import type { TranscriptPosition } from './workspace/transcript'
import TranscriptContent from './workspace/TranscriptContent.vue'
defineProps<{ messages: MessageDto[]; live: BlockDto[]; running: boolean; newAnchorId: string | null; agent?: string; prompts?: PromptDto[]; conversationId?: string; position?: TranscriptPosition }>()
const emit = defineEmits<{ savePosition: [id: string, position: TranscriptPosition] }>()
</script>
<template>
  <MessageScrollerProvider auto-scroll default-scroll-position="last-anchor" :scroll-previous-item-peek="64">
    <TranscriptContent v-bind="$props" @save-position="(id, position) => emit('savePosition', id, position)" />
  </MessageScrollerProvider>
</template>
