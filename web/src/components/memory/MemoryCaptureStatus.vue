<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '../ui/button'
const props = defineProps<{ channelId: string; paused: boolean }>()
const emit = defineEmits<{ changed: [] }>()
const busy = ref(false)
const error = ref('')
async function run(command: string) {
  error.value = ''
  busy.value = true
  try {
    await invoke(command, { channelId: props.channelId })
    emit('changed')
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <section class="space-y-2 rounded border border-border p-3 text-xs">
    <p v-if="paused" class="font-medium">Automatic capture is paused after Forget or Undo.</p>
    <p>
      Transcript messages stay saved. A fresh conversation starts without the old transcript or
      pause.
    </p>
    <div class="flex flex-wrap gap-2">
      <Button v-if="paused" size="sm" :disabled="busy" @click="run('memory_resume')">
        Resume automatic capture
      </Button>
      <Button
        size="sm"
        variant="outline"
        :disabled="busy"
        @click="run('memory_fresh_conversation')"
      >
        Start fresh conversation
      </Button>
    </div>
    <p v-if="error" role="alert">{{ error }}</p>
  </section>
</template>
