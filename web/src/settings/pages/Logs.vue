<script setup lang="ts">
defineOptions({ name: 'SettingsLogs' })
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import SettingsNotice from '../ui/SettingsNotice.vue'
import { parseLogLine, type LogLevel } from '../log_line'
const level = ref('INFO'),
  component = ref('All'),
  id = ref(''),
  follow = ref(true),
  lines = ref<string[]>([]),
  error = ref(''),
  container = ref<HTMLElement | null>(null)
let timer: ReturnType<typeof setInterval>,
  generation = 0
async function reload() {
  const current = ++generation
  try {
    const result = await invoke<string[]>('settings_logs', {
      level: level.value,
      component: component.value,
      id: id.value,
    })
    if (current !== generation) return
    const changed = result.join('\n') !== lines.value.join('\n')
    lines.value = result
    if (follow.value && changed) {
      await nextTick()
      container.value?.scrollTo(0, container.value.scrollHeight)
    }
  } catch (e) {
    error.value = String(e)
  }
}
// Parse once per poll, not per render. A row carries its date separator, when the date changes.
const rows = computed(() => {
  let date = ''
  return lines.value.map((line, key) => {
    const parsed = parseLogLine(line)
    const separator = parsed.date && parsed.date !== date ? parsed.date : ''
    if (parsed.date) date = parsed.date
    return { key, parsed, separator }
  })
})
const levelColor: Record<LogLevel, string> = {
  Error: 'text-danger',
  Warn: 'text-warning',
  Info: 'text-foreground',
  Debug: 'text-muted-foreground',
  Trace: 'text-muted-foreground',
}
const dim = new Set<LogLevel>(['Debug', 'Trace'])
// Error and warn rows get a faint tint and a 2px left edge in the level token.
const rowTone: Record<LogLevel, string> = {
  Error:
    'bg-[color-mix(in_srgb,var(--color-danger)_9%,transparent)] shadow-[inset_2px_0_0_var(--color-danger)] hover:bg-[color-mix(in_srgb,var(--color-danger)_14%,transparent)]',
  Warn: 'bg-[color-mix(in_srgb,var(--color-warning)_9%,transparent)] shadow-[inset_2px_0_0_var(--color-warning)] hover:bg-[color-mix(in_srgb,var(--color-warning)_14%,transparent)]',
  Info: 'hover:bg-muted/40',
  Debug: 'hover:bg-muted/40',
  Trace: 'hover:bg-muted/40',
}
const levels = ['TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR']
const components = ['All', 'Provider', 'Plugin', 'Approval', 'Storage', 'MCP', 'Workspace', 'Other']
watch([level, component, id], () => {
  void reload()
})
onMounted(() => {
  void reload()
  timer = setInterval(reload, 700)
})
onUnmounted(() => clearInterval(timer))
</script>
<template>
  <div class="flex min-h-0 flex-1 flex-col gap-3">
    <div class="flex items-center gap-3 font-mono text-xs">
      <Select :model-value="level" @update:model-value="level = String($event)">
        <SelectTrigger size="sm" aria-label="Log level" class="h-8 shrink-0 font-mono text-xs">
          <span class="text-muted-foreground">level</span>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem v-for="item in levels" :key="item" :value="item" class="font-mono text-xs">
            {{ item }}
          </SelectItem>
        </SelectContent>
      </Select>
      <Select :model-value="component" @update:model-value="component = String($event)">
        <SelectTrigger size="sm" aria-label="Log component" class="h-8 shrink-0 font-mono text-xs">
          <span class="text-muted-foreground">component</span>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem
            v-for="item in components"
            :key="item"
            :value="item"
            class="font-mono text-xs"
          >
            {{ item }}
          </SelectItem>
        </SelectContent>
      </Select>
      <Input
        v-model="id"
        aria-label="Filter by session, turn, or plugin ID"
        placeholder="Session, turn, or plugin ID"
        class="h-8 min-w-0 flex-1 font-mono text-xs"
      />
      <div class="flex shrink-0 items-center gap-2">
        <label for="logs-follow" class="text-muted-foreground">follow</label>
        <Switch id="logs-follow" v-model="follow" aria-label="Follow" />
      </div>
      <span class="shrink-0 text-muted-foreground tabular-nums">
        {{ lines.length }} {{ lines.length === 1 ? 'line' : 'lines' }}
      </span>
    </div>
    <SettingsNotice v-if="error" as="p" role="alert">{{ error }}</SettingsNotice>
    <div
      ref="container"
      class="min-h-48 flex-1 overflow-auto rounded-lg border border-border bg-card font-mono text-xs leading-5"
    >
      <p v-if="!lines.length" class="p-3 text-muted-foreground">no log lines match</p>
      <template v-for="row in rows" :key="row.key">
        <div v-if="row.separator" class="section-label px-3 pt-3 pb-1">{{ row.separator }}</div>
        <div
          v-if="row.parsed.level"
          class="grid grid-cols-[12ch_5ch_9ch_minmax(0,1fr)] gap-x-3 px-3"
          :class="rowTone[row.parsed.level]"
        >
          <span class="text-muted-foreground tabular-nums">{{ row.parsed.time }}</span>
          <span :class="levelColor[row.parsed.level]">{{ row.parsed.level.toUpperCase() }}</span>
          <span class="truncate text-muted-foreground">{{ row.parsed.component }}</span>
          <span
            class="break-words whitespace-pre-wrap"
            :class="dim.has(row.parsed.level) ? 'text-muted-foreground' : ''"
            v-text="row.parsed.message"
          />
        </div>
        <div
          v-else
          class="px-3 break-words whitespace-pre-wrap hover:bg-muted/40"
          v-text="row.parsed.raw"
        />
      </template>
    </div>
  </div>
</template>
