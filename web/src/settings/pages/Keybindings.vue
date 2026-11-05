<script setup lang="ts">
defineOptions({ name: 'SettingsKeybindings' })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { runAction } from '@/bridge/api'
import { Button } from '@/components/ui/button'
import { captureChord } from '../key_capture'
type Row = {
  id: string
  label: string
  chord: string
  bound: boolean
  custom: boolean
  rule: string
}
const rows = ref<Row[]>([]),
  capture = ref<string | null>(null),
  error = ref(''),
  errorId = ref(''),
  saved = ref(false)
async function reload() {
  rows.value = await invoke<Row[]>('settings_keymap_rows')
}
async function setKey(id: string, chord: string | null, reset = false) {
  try {
    await invoke('settings_keymap_set', { actionId: id, chord, reset })
    error.value = ''
    saved.value = true
    await reload()
  } catch (e) {
    errorId.value = id
    error.value = String(e)
  }
}
function keydown(event: KeyboardEvent) {
  if (!capture.value) return
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Escape') {
    capture.value = null
    return
  }
  const chord = captureChord(event)
  if (!chord) return
  const id = capture.value
  capture.value = null
  void setKey(id, chord)
}
onMounted(() => {
  void reload()
  window.addEventListener('keydown', keydown, true)
})
onUnmounted(() => window.removeEventListener('keydown', keydown, true))
</script>
<template>
  <div class="space-y-3">
    <div v-for="row in rows" :key="row.id">
      <div class="flex items-center gap-2">
        <span class="flex-1">{{ row.label }}</span
        ><span>{{ capture === row.id ? 'Press a key (Escape to cancel)' : row.chord }}</span
        ><span v-if="row.rule === 'locked'">Locked</span
        ><template v-else
          ><Button
            variant="outline"
            @click="
              () => {
                capture = row.id
                error = ''
              }
            "
            >Record</Button
          ><Button v-if="row.custom" variant="outline" @click="setKey(row.id, null, true)"
            >Reset</Button
          ><Button
            v-if="row.rule === 'optional' && row.bound"
            variant="outline"
            @click="setKey(row.id, null)"
            >Unbind</Button
          ></template
        >
      </div>
      <p
        v-if="error && errorId === row.id"
        role="alert"
        data-protected="true"
        class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"
      >
        {{ error }}
      </p>
    </div>
    <div v-if="saved">
      <p>Applies at next launch</p>
      <Button @click="runAction('relaunch')">Relaunch</Button>
    </div>
  </div>
</template>
