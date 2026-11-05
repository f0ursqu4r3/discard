<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import type { BlockDto, MessageDto, PromptDto } from '../bridge/types'
import { markRead } from '../bridge/api'
import { Button } from './ui/button'
import { ScrollArea } from './ui/scroll-area'
import MessageRow from './MessageRow.vue'
import { hasLiveRow, rowCount } from './messages'
const props = defineProps<{ messages: MessageDto[]; live: BlockDto[]; running: boolean; newAnchorId: string | null; agent?: string; prompts?: PromptDto[] }>()
const root = ref<InstanceType<typeof ScrollArea> | null>(null)
const atBottom = ref(true)
let viewport: HTMLElement | null = null
function read() {
  const last = props.messages.at(-1)
  if (atBottom.value && document.hasFocus() && last) void markRead(last.id)
}
function onScroll() {
  if (!viewport) return
  atBottom.value = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < 8
  read()
}
function jump() { if (viewport) viewport.scrollTop = viewport.scrollHeight; atBottom.value = true; read() }
const liveMessage = computed<MessageDto>(() => ({ id: 'live', role: 'agent', blocks: props.live, continuesGroup: props.messages.at(-1)?.role === 'agent' }))
onMounted(() => {
  viewport = root.value?.$el.querySelector('[data-slot="scroll-area-viewport"]') ?? null
  viewport?.addEventListener('scroll', onScroll)
  window.addEventListener('focus', read)
  void nextTick(jump)
})
onUnmounted(() => { viewport?.removeEventListener('scroll', onScroll); window.removeEventListener('focus', read) })
watch(() => [props.messages.length, props.live], async () => { if (atBottom.value) { await nextTick(); jump() } }, { deep: true })
</script>
<template>
  <div class="relative min-h-0 flex-1" :data-row-count="rowCount(messages.length, hasLiveRow(running, live.length))">
    <ScrollArea ref="root" class="h-full">
      <template v-for="message in messages" :key="message.id">
        <div v-if="message.id === newAnchorId" data-testid="new-divider" class="flex items-center gap-2 px-4 text-sm text-primary"><span class="flex-1 border-t border-primary" />New</div>
        <MessageRow :message="message" :agent="agent" :prompts="prompts" />
      </template>
      <div v-if="hasLiveRow(running, live.length)"><MessageRow :message="liveMessage" :agent="agent" :prompts="prompts" show-cursor /></div>
    </ScrollArea>
    <Button v-if="!atBottom" class="absolute bottom-2 right-4" @click="jump">Jump to latest</Button>
  </div>
</template>
