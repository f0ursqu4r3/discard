<script setup lang="ts">
defineOptions({ name: 'SettingsMemory' })
import { onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
type Row = { scope: string; title: string; files: string[] }
const rows = ref<Row[]>([]), text = ref<string | null>(null), error = ref('')
async function reload() { rows.value = await invoke<Row[]>('settings_memory_scopes') }
async function read(scope: string, path: string) { try { await reload(); text.value = await invoke<string>('settings_memory_read', { scope, path }) } catch (e) { error.value = String(e) } }
async function reveal(scope: string) { try { await invoke('settings_reveal_memory', { scope }) } catch (e) { error.value = String(e) } }
onMounted(() => { void reload() })
</script>
<template>
  <div class="space-y-3"><p v-if="error" role="alert" data-protected="true" class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]">{{ error }}</p><section v-for="row in rows" :key="row.scope" class="space-y-1"><div class="flex items-center gap-2"><span>{{ row.title }} ({{ row.files.length }})</span><Button variant="outline" @click="reveal(row.scope)">Reveal</Button></div><div v-for="file in row.files" :key="file"><Button variant="ghost" @click="read(row.scope, file)">{{ file }}</Button></div></section><pre v-if="text !== null" class="whitespace-pre-wrap">{{ text }}</pre></div>
</template>
