<script setup lang="ts">
defineOptions({ name: 'SettingsAdvanced' })
import { onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
type Action = { id: string; title: string; lines: string[] }
const actions = ref<Action[]>([]), pending = ref<string | null>(null), result = ref<Record<string, string>>({}), error = ref('')
async function run(id: string) { try { result.value[id] = await invoke<string>('settings_advanced_run', { actionId: id, confirmed: true }); pending.value = null; error.value = '' } catch (e) { error.value = String(e) } }
onMounted(async () => { actions.value = await invoke<Action[]>('settings_advanced_actions') })
</script>
<template>
  <div class="space-y-4"><p v-if="error" role="alert" data-protected="true" class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]">{{ error }}</p><section v-for="action in actions" :key="action.id" class="space-y-2"><h2 class="font-semibold">{{ action.title }}</h2><p v-for="line in action.lines" :key="line">{{ line }}</p><div v-if="pending === action.id" data-protected="true" class="flex gap-2 border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"><Button @click="run(action.id)">Confirm</Button><Button variant="outline" @click="pending = null">Cancel</Button></div><Button v-else @click="pending = action.id">{{ action.title }}</Button><p v-if="result[action.id]">{{ result[action.id] }}</p></section></div>
</template>
