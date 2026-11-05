<script setup lang="ts">
defineOptions({ name: 'SettingsLogs' })
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
const level = ref('INFO'), component = ref('All'), id = ref(''), follow = ref(true), lines = ref<string[]>([]), error = ref(''), container = ref<HTMLElement | null>(null)
let timer: ReturnType<typeof setInterval>, generation = 0
async function reload() { const current = ++generation; try { const result = await invoke<string[]>('settings_logs', { level: level.value, component: component.value, id: id.value }); if (current !== generation) return; const changed = result.join('\n') !== lines.value.join('\n'); lines.value = result; if (follow.value && changed) { await nextTick(); container.value?.scrollTo(0, container.value.scrollHeight) } } catch (e) { error.value = String(e) } }
watch([level, component, id], () => { void reload() })
onMounted(() => { void reload(); timer = setInterval(reload, 700) })
onUnmounted(() => clearInterval(timer))
</script>
<template>
  <div class="space-y-2"><p>Level</p><div class="flex flex-wrap gap-1"><Button v-for="item in ['TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR']" :key="item" :variant="item === level ? 'default' : 'outline'" @click="level = item">{{ item }}</Button></div><p>Component</p><div class="flex flex-wrap gap-1"><Button v-for="item in ['All', 'Provider', 'Plugin', 'Approval', 'Storage', 'MCP', 'Workspace', 'Other']" :key="item" :variant="item === component ? 'default' : 'outline'" @click="component = item">{{ item }}</Button></div><Input v-model="id" placeholder="Session, turn, or plugin ID" /><div class="flex items-center gap-2"><span>Follow</span><Switch v-model="follow" aria-label="Follow" /></div><p v-if="error" role="alert" data-protected="true" class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]">{{ error }}</p><div ref="container" class="h-100 overflow-auto"><p v-for="(line, index) in lines" :key="index" class="whitespace-pre-wrap">{{ line }}</p></div></div>
</template>
