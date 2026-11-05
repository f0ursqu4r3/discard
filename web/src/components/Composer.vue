<script setup lang="ts">
defineOptions({ name: "MessageComposer" })
import { ref } from 'vue'
import { Textarea } from './ui/textarea'
import { Button } from './ui/button'
import { maySend, mayStop } from './composer'
const props = defineProps<{ archived: boolean; busy: boolean; running: boolean; channel: string }>()
const emit = defineEmits<{ send: [text: string]; stop: [] }>()
const text = ref('')
function send() {
  if (props.running || !maySend(props.archived || props.busy, text.value)) return
  emit('send', text.value.trim())
  text.value = ''
}
function keydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) { event.preventDefault(); send() }
}
</script>
<template>
  <div class="flex items-center gap-2 border-t border-border p-3">
    <Textarea v-model="text" :disabled="archived || busy" :placeholder="`Message #${channel}`" @keydown="keydown" />
    <Button v-if="mayStop(running)" variant="destructive" @click="emit('stop')">Stop</Button>
    <Button v-else :disabled="!maySend(archived || busy, text)" @click="send">Send</Button>
  </div>
</template>
