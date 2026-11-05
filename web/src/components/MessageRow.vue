<script setup lang="ts">
import type { MessageDto, PromptDto } from '../bridge/types'
import { Avatar, AvatarFallback } from './ui/avatar'
import { Button } from './ui/button'
import { Badge } from './ui/badge'
import BlockView from './BlockView.vue'
import ContextMenuHost from './ContextMenuHost.vue'
import { runAction } from '../bridge/api'
import { computed } from 'vue'
import { serverInitials } from './layout'
const props = defineProps<{
  channelId: string
  message: MessageDto
  agent?: string
  prompts?: PromptDto[]
  showCursor?: boolean
}>()
const target = computed(() =>
  props.message.id === 'live'
    ? null
    : {
        type: 'message' as const,
        id: props.message.id,
        hasText: props.message.blocks.some((block) => 'text' in block && !!block.text),
      },
)
async function copy() {
  if (!target.value) return
  const text = await runAction('copy_message_text', target.value)
  if (text !== null) await navigator.clipboard.writeText(text)
}
</script>
<template>
  <ContextMenuHost :target="target"
    ><article
      :data-ui-id="`message-${message.id}`"
      class="group relative flex gap-3 px-3 py-3 text-xs leading-6 hover:bg-muted/30 sm:px-6"
    >
      <div class="w-9 shrink-0">
        <Avatar v-if="!message.continuesGroup" class="size-9"
          ><AvatarFallback class="bg-primary/20 text-primary">{{
            serverInitials(message.role === 'user' ? 'You' : agent || message.role)
          }}</AvatarFallback></Avatar
        >
      </div>
      <div class="flex min-w-0 flex-1 flex-col gap-1">
        <div
          v-if="!message.continuesGroup"
          data-testid="message-identity"
          class="flex items-center gap-2 font-semibold"
        >
          {{
            message.role === 'user'
              ? 'You'
              : message.role === 'agent'
                ? agent || 'Agent'
                : 'System'
          }}<Badge v-if="message.role === 'agent'" variant="secondary" class="text-xs font-normal"
            >Agent</Badge
          >
        </div>
        <BlockView
          :channel-id="channelId"
          v-for="(block, i) in message.blocks"
          :key="i"
          :block="block"
          :prompts="prompts"
          :message-id="target?.id"
          :index="i"
        />
        <span v-if="showCursor" data-testid="stream-cursor" class="text-muted-foreground">▍</span>
      </div>
      <Button
        v-if="target"
        :disabled="!target.hasText"
        variant="secondary"
        size="sm"
        class="absolute right-2 top-0 invisible group-hover:visible focus-visible:visible"
        @click="copy"
        >Copy</Button
      >
    </article></ContextMenuHost
  >
</template>
