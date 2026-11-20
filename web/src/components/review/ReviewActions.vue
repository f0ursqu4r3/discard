<script setup lang="ts">
import { ref } from 'vue'
import { Button } from '../ui/button'
import { Textarea } from '../ui/textarea'

const props = defineProps<{
  /** Approve is off, for example when the channel is archived. */
  approveOff: boolean
  /** Request changes and the summary are off: the channel is busy or archived. */
  busy: boolean
  /** An action is in flight. */
  pending: boolean
}>()
const emit = defineEmits<{ approve: []; request: [note: string]; summary: [] }>()

/** The most characters of a note. The backend refuses more. */
const NOTE_LIMIT = 4000
const writing = ref(false)
const note = ref('')

const trimmed = () => note.value.trim()
function send() {
  if (trimmed() === '' || props.busy || props.pending) return
  emit('request', trimmed())
}
/** Close the note field and clear it. The parent calls this after a decision is saved. */
function close() {
  writing.value = false
  note.value = ''
}
defineExpose({ close })
</script>

<template>
  <div data-ui-id="review-actions" class="flex flex-col gap-2">
    <div class="flex flex-wrap items-center gap-2">
      <Button type="button" size="sm" :disabled="approveOff || pending" @click="emit('approve')">
        Approve
      </Button>
      <Button
        type="button"
        variant="outline"
        size="sm"
        :disabled="busy || pending"
        :aria-expanded="writing"
        @click="writing = !writing"
      >
        Request changes
      </Button>
      <Button
        type="button"
        variant="outline"
        size="sm"
        :disabled="busy || pending"
        @click="emit('summary')"
      >
        Ask for a summary
      </Button>
    </div>
    <form v-if="writing" class="flex flex-col gap-2" @submit.prevent="send">
      <Textarea
        v-model="note"
        :maxlength="NOTE_LIMIT"
        rows="4"
        aria-label="Note for the agent"
        placeholder="What should change?"
      />
      <div class="flex items-center gap-2">
        <Button type="submit" size="sm" :disabled="trimmed() === '' || busy || pending">
          Send request
        </Button>
        <Button type="button" variant="ghost" size="sm" @click="close">Cancel</Button>
      </div>
    </form>
  </div>
</template>
