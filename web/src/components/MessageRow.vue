<script setup lang="ts">
import type { MessageDto, PromptDto } from '../bridge/types'
import { Avatar, AvatarFallback } from './ui/avatar'
import { Marker, MarkerContent, MarkerIcon } from './ui/marker'
import { Info } from '@lucide/vue'
import AgentAvatar from './agents/AgentAvatar.vue'
import { Button } from './ui/button'
import BlockView from './BlockView.vue'
import ContextMenuHost from './ContextMenuHost.vue'
import { runAction } from '../bridge/api'
import { computed } from 'vue'
import { useAppearance } from '@/bridge/appearance'
import { serverInitials } from './layout'
const props = defineProps<{
  channelId: string
  message: MessageDto
  agent?: string
  agentAvatar?: string | null
  prompts?: PromptDto[]
  showCursor?: boolean
  preview?: boolean
}>()
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
const target = computed(() =>
  props.preview || props.message.id === 'live'
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
  <ContextMenuHost :target="target">
    <Marker
      v-if="systemNote"
      as="article"
      :data-ui-id="`message-${message.id}`"
      aria-label="System note"
      class="px-3 sm:px-6"
      :class="compact ? 'py-1' : 'py-2'"
    >
      <MarkerIcon><Info /></MarkerIcon>
      <MarkerContent class="whitespace-pre-wrap">{{ systemText }}</MarkerContent>
    </Marker>
    <article
      v-else
      :data-ui-id="`message-${message.id}`"
      class="group relative flex gap-3 px-3 text-sm leading-relaxed hover:bg-muted/30 sm:px-6"
      :class="compact ? 'py-1' : 'py-2'"
    >
      <div v-if="appearance.showMessageAvatars" data-testid="message-avatar" class="w-9 shrink-0">
        <AgentAvatar
          v-if="!message.continuesGroup && message.role === 'agent'"
          :name="authorName"
          :avatar="authorAvatar"
          class="size-9"
        />
        <Avatar v-else-if="!message.continuesGroup" class="size-9">
          <AvatarFallback class="bg-primary/20 text-primary">
            {{ serverInitials(message.role === 'user' ? 'You' : agent || message.role) }}
          </AvatarFallback>
        </Avatar>
      </div>
      <div class="flex min-w-0 flex-1 flex-col" :class="compact ? 'gap-0' : 'gap-1'">
        <div
          v-if="!message.continuesGroup"
          data-testid="message-identity"
          class="flex items-center gap-2 font-semibold"
        >
          {{ message.role === 'user' ? 'You' : message.role === 'agent' ? authorName : 'System' }}
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
      >
        Copy
      </Button>
    </article>
  </ContextMenuHost>
</template>
