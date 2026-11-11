<script setup lang="ts">
defineOptions({ name: 'SettingsProviders' })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import KeyValue from '../ui/KeyValue.vue'
import SettingsCard from '../ui/SettingsCard.vue'
import SettingsNotice from '../ui/SettingsNotice.vue'
type Row = {
  id: string
  setup: string
  command: string | null
  capabilities: { label: string; supported: boolean }[]
  hooks: { label: string; mode: string }[]
  blocked: string | null
}
const rows = ref<Row[]>([])
const error = ref('')
let timer: ReturnType<typeof setInterval>
// Colour the status badge only for the setup states the backend names.
function setupTone(setup: string) {
  if (setup === 'Ready') return 'border-success/50 bg-success/10 text-success'
  if (setup === 'Not installed' || setup === 'Not logged in')
    return 'border-warning/50 bg-warning/10 text-warning'
  return 'border-border text-muted-foreground'
}
async function reload() {
  rows.value = await invoke<Row[]>('settings_providers')
}
async function recheck(id: string) {
  try {
    await invoke('settings_recheck_provider', { id })
    await reload()
  } catch (e) {
    error.value = String(e)
  }
}
onMounted(() => {
  void reload()
  timer = setInterval(reload, 700)
})
onUnmounted(() => clearInterval(timer))
</script>
<template>
  <div class="space-y-4">
    <SettingsNotice v-if="error" as="p" role="alert">{{ error }}</SettingsNotice>
    <SettingsCard v-for="row in rows" :key="row.id">
      <template #title>
        <span class="inline-flex items-center gap-2">
          <span>{{ row.id }}</span>
          <span
            class="rounded-sm border px-1.5 text-[10.5px] font-normal"
            :class="setupTone(row.setup)"
          >
            <span class="sr-only">Setup status:</span>
            {{ row.setup }}
          </span>
        </span>
      </template>
      <template #actions>
        <Button size="xs" variant="outline" class="font-sans" @click="recheck(row.id)">
          Re-check
        </Button>
      </template>
      <div class="space-y-4">
        <p
          v-if="row.command"
          class="rounded-md border border-border bg-background px-2 py-1.5 font-mono text-xs break-all"
        >
          <span aria-hidden="true" class="text-muted-foreground">$</span>
          {{ row.command }}
        </p>
        <div v-if="row.capabilities.length" class="space-y-2">
          <h3 class="section-label">Capabilities</h3>
          <dl class="flex flex-col gap-1.5">
            <KeyValue v-for="item in row.capabilities" :key="item.label" :label="item.label">
              <span class="inline-flex flex-row-reverse items-baseline gap-1.5">
                <span>{{ item.supported ? 'Yes' : 'No' }}</span>
                <span aria-hidden="true" :class="item.supported ? 'text-success' : 'text-danger'">
                  {{ item.supported ? '✓' : '✗' }}
                </span>
              </span>
            </KeyValue>
          </dl>
        </div>
        <div v-if="row.hooks.length" class="space-y-2">
          <h3 class="section-label">Hook enforcement</h3>
          <dl class="flex flex-col gap-1.5">
            <KeyValue v-for="item in row.hooks" :key="item.label" :label="item.label">
              {{ item.mode }}
            </KeyValue>
          </dl>
        </div>
        <p v-if="row.blocked" class="text-sm text-warning">{{ row.blocked }}</p>
      </div>
    </SettingsCard>
  </div>
</template>
