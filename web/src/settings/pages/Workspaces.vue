<script setup lang="ts">
defineOptions({ name: 'SettingsWorkspaces' })
import { onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
type Worktree = { path: string; branch: string; exists: boolean }
type Project = { name: string; path: string; exists: boolean; channels: { name: string; mode: string; archived: boolean; current: boolean; worktree: Worktree | null }[] }
const rows = ref<Project[]>([]), error = ref('')
async function reveal(path: string) { try { await invoke('settings_reveal_workspace', { path }) } catch (e) { error.value = String(e) } }
onMounted(async () => { try { rows.value = await invoke<Project[]>('settings_workspaces') } catch (e) { error.value = String(e) } })
</script>
<template>
  <div class="space-y-4"><p v-if="error" role="alert" data-protected="true" class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]">{{ error }}</p><section v-for="project in rows" :key="project.path" class="space-y-1"><div class="flex items-center gap-2"><span>{{ project.name }} · {{ project.path }}{{ project.exists ? '' : ' · missing' }}</span><Button variant="outline" @click="reveal(project.path)">Reveal</Button></div><div v-for="channel in project.channels" :key="channel.name"><p>{{ channel.name }} · {{ channel.mode }}{{ channel.archived ? ' · Archived' : '' }}{{ channel.current ? ' · Current' : '' }}</p><div v-if="channel.worktree" class="flex items-center gap-2"><span>{{ channel.worktree.branch }} · {{ channel.worktree.path }}{{ channel.worktree.exists ? '' : ' · missing' }}</span><Button variant="outline" @click="reveal(channel.worktree.path)">Reveal</Button></div></div></section></div>
</template>
