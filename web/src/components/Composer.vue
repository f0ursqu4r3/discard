<script setup lang="ts">
defineOptions({ name: 'MessageComposer' })
import { computed, ref } from 'vue'
import { Send, Square, AtSign } from '@lucide/vue'
import { InputGroup, InputGroupAddon, InputGroupTextarea } from './ui/input-group'
import { Button } from './ui/button'
import { maySend, mayStop } from './composer'
const props = defineProps<{
  archived: boolean
  busy: boolean
  running: boolean
  channel: string
  directMessage?: boolean
  agent?: string
  draft?: string
}>()
const emit = defineEmits<{
  send: [text: string]
  stop: []
  'update:draft': [text: string]
}>()
const placeholder = computed(() =>
  props.directMessage ? `Message ${props.agent || 'agent'}` : `Message #${props.channel}`,
)
const localDraft = ref('')
const text = computed({
  get: () => props.draft ?? localDraft.value,
  set: (value) => {
    localDraft.value = value
    emit('update:draft', value)
  },
})
function send() {
  if (props.running || !maySend(props.archived || props.busy, text.value)) return
  emit('send', text.value.trim())
  if (props.draft === undefined) text.value = ''
}
function keydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
    event.preventDefault()
    send()
  }
}
</script>
<template>
  <section
    data-ui-id="message-composer"
    aria-label="Message composer"
    class="mt-auto shrink-0 px-3 pt-2 pb-3"
  >
    <InputGroup class="rounded-lg">
      <InputGroupTextarea
        v-model="text"
        data-ui-id="composer-input"
        class="max-h-40 min-h-12 py-3"
        :rows="1"
        :disabled="archived || busy"
        :aria-label="placeholder"
        :placeholder="placeholder"
        @keydown="keydown"
      />
      <InputGroupAddon align="inline-end" class="self-end pb-2">
        <AtSign aria-hidden="true" class="size-4 text-muted-foreground" />
        <Button
          v-if="mayStop(running)"
          data-ui-id="composer-stop"
          size="sm"
          variant="destructive"
          @click="emit('stop')"
        >
          <Square data-icon="inline-start" />
          Stop
        </Button>
        <Button
          v-else
          data-ui-id="composer-send"
          size="icon-sm"
          :disabled="!maySend(archived || busy, text)"
          aria-label="Send message"
          @click="send"
        >
          <Send />
          <span class="sr-only">Send</span>
        </Button>
      </InputGroupAddon>
    </InputGroup>
    <p v-if="archived && !directMessage" class="mt-2 text-xs text-muted-foreground">
      Channel archived
    </p>
  </section>
</template>
