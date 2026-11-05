<script setup lang="ts">
defineOptions({ name: 'SettingsProviders' })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
type Row = { id: string; setup: string; command: string | null; capabilities: { label: string; supported: boolean }[]; hooks: { label: string; mode: string }[]; blocked: string | null }
const rows = ref<Row[]>([])
const error = ref('')
let timer: ReturnType<typeof setInterval>
async function reload() { rows.value = await invoke<Row[]>('settings_providers') }
async function recheck(id: string) { try { await invoke('settings_recheck_provider', { id }); await reload() } catch (e) { error.value = String(e) } }
onMounted(() => { void reload(); timer = setInterval(reload, 700) })
onUnmounted(() => clearInterval(timer))
</script>
<template>
  <div class="space-y-4">
    <p v-if="error" role="alert" data-protected="true" class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]">{{ error }}</p>
    <Card v-for="row in rows" :key="row.id"><CardHeader class="flex flex-row items-center justify-between"><CardTitle>{{ row.id }}</CardTitle><Button @click="recheck(row.id)">Re-check</Button></CardHeader><CardContent class="space-y-2"><p>Setup status: {{ row.setup }}</p><p v-if="row.command" class="font-mono">{{ row.command }}</p><h2 class="font-semibold">Capabilities</h2><p v-for="item in row.capabilities" :key="item.label">{{ item.label }}: {{ item.supported ? 'Yes' : 'No' }}</p><h2 class="font-semibold">Hook enforcement</h2><p v-for="item in row.hooks" :key="item.label">{{ item.label }}: {{ item.mode }}</p><p v-if="row.blocked">{{ row.blocked }}</p></CardContent></Card>
  </div>
</template>
