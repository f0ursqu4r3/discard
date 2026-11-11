<script setup lang="ts">
defineOptions({ name: 'SettingsKeybindings' })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { runAction } from '@/bridge/api'
import { Button } from '@/components/ui/button'
import { captureChord } from '../key_capture'
import SettingsCard from '../ui/SettingsCard.vue'
import SettingsNotice from '../ui/SettingsNotice.vue'
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
  <div class="flex flex-col">
    <SettingsCard title="Shortcuts" flush>
      <template v-if="rows.length" #actions>
        <span class="text-muted-foreground">
          {{ rows.length }} {{ rows.length === 1 ? 'shortcut' : 'shortcuts' }}
        </span>
      </template>
      <div
        v-for="row in rows"
        :key="row.id"
        class="border-b border-dashed border-border px-3 py-2.5 last:border-b-0 hover:bg-muted/40"
      >
        <div class="flex items-center gap-2">
          <span class="min-w-0 flex-1 font-medium">{{ row.label }}</span>
          <span v-if="capture === row.id" class="font-mono text-xs text-primary">
            <span aria-hidden="true">›</span>
            Press a key (Escape to cancel)
            <span
              aria-hidden="true"
              class="ml-1 inline-block h-[1.1em] w-[0.55em] animate-[cursor-blink_1.05s_steps(1)_infinite] bg-foreground align-[-0.2em]"
            />
          </span>
          <kbd
            v-else
            class="rounded-md border border-border bg-muted px-1.5 py-0.5 font-mono text-xs"
          >
            {{ row.chord }}
          </kbd>
          <span v-if="row.rule === 'locked'" class="font-mono text-xs text-muted-foreground">
            Locked
          </span>
          <template v-else>
            <Button
              size="sm"
              variant="outline"
              @click="
                () => {
                  capture = row.id
                  error = ''
                }
              "
            >
              Record
            </Button>
            <Button v-if="row.custom" size="sm" variant="ghost" @click="setKey(row.id, null, true)">
              Reset
            </Button>
            <Button
              v-if="row.rule === 'optional' && row.bound"
              size="sm"
              variant="ghost"
              @click="setKey(row.id, null)"
            >
              Unbind
            </Button>
          </template>
        </div>
        <SettingsNotice v-if="error && errorId === row.id" as="p" role="alert" class="mt-2">
          {{ error }}
        </SettingsNotice>
      </div>
    </SettingsCard>
    <div
      v-if="saved"
      class="sticky bottom-0 mt-4 flex items-center justify-between gap-3 rounded-lg border border-border bg-card px-3 py-2"
    >
      <p class="text-sm">Applies at next launch</p>
      <Button size="sm" variant="outline" @click="runAction('relaunch')">Relaunch</Button>
    </div>
  </div>
</template>
