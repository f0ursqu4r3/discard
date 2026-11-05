<script setup lang="ts">
import type { MessageDto, PromptDto } from '../bridge/types'
import { Avatar, AvatarFallback } from './ui/avatar'
import { Button } from './ui/button'
import BlockView from './BlockView.vue'
import ContextMenuHost from './ContextMenuHost.vue'
import { runAction } from '../bridge/api'
import { computed } from 'vue'
import { serverInitials } from './layout'
const props = defineProps<{ message: MessageDto; agent?: string; prompts?: PromptDto[]; showCursor?: boolean }>()
const target = computed(() => props.message.id === 'live' ? null : { type: 'message' as const, id: props.message.id, hasText: props.message.blocks.some(block => 'text' in block && !!block.text) })
async function copy() { if (!target.value) return; const text = await runAction('copy_message_text', target.value); if (text !== null) await navigator.clipboard.writeText(text) }
</script>
<template>
  <ContextMenuHost :target="target"><article class="group relative flex gap-3 px-4 py-2 hover:bg-muted motion-safe:animate-in motion-safe:fade-in motion-safe:duration-150">
    <div class="w-10 shrink-0"><Avatar v-if="!message.continuesGroup"><AvatarFallback>{{ serverInitials(message.role === 'user' ? 'You' : agent || message.role) }}</AvatarFallback></Avatar></div>
    <div class="min-w-0 flex-1 space-y-1">
      <div v-if="!message.continuesGroup" data-testid="message-identity" class="font-bold">{{ message.role === 'user' ? 'You' : message.role === 'agent' ? agent || 'Agent' : 'System' }}</div>
      <BlockView v-for="(block, i) in message.blocks" :key="i" :block="block" :prompts="prompts" :message-id="target?.id" :index="i" />
      <span v-if="showCursor" data-testid="stream-cursor" class="text-muted-foreground">▍</span>
    </div>
    <Button v-if="target" :disabled="!target.hasText" variant="secondary" size="sm" class="absolute right-2 top-0 invisible group-hover:visible focus-visible:visible" @click="copy">Copy</Button>
  </article></ContextMenuHost>
</template>
