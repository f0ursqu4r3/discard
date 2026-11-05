<script setup lang="ts">
defineOptions({ name: 'SettingsAgents' })
import { onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { runAction } from '@/bridge/api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Switch } from '@/components/ui/switch'
type Agent = { id: string; name: string; provider: string; model: string; systemPrompt: string; policy: string; pluginWrite: boolean }
type Review = { changes: { field: string; before: string; after: string }[]; choices: { policy: string; enabled: boolean; reason: string | null }[]; blocked: string | null; needsConfirm: boolean }
const agents = ref<Agent[]>([]), selected = ref<Agent | null>(null), review = ref<Review | null>(null), error = ref(''), confirm = ref(false), saved = ref(false)
let generation = 0
async function reload() { agents.value = await invoke<Agent[]>('settings_agent_get') }
function select(agent: Agent) { selected.value = { ...agent }; confirm.value = false; saved.value = false; error.value = '' }
watch(selected, async agent => {
  if (!agent) return
  confirm.value = false; saved.value = false; error.value = ''
  const current = ++generation
  try { const result = await invoke<Review>('settings_agent_review', { id: agent.id, draft: agent }); if (current === generation) review.value = result }
  catch (e) { if (current === generation) error.value = String(e) }
}, { deep: true })
async function save(confirmed: boolean) {
  if (!selected.value) return
  try { await invoke('settings_agent_save', { id: selected.value.id, draft: selected.value, confirmed }); await reload(); review.value = null; confirm.value = false; saved.value = true; error.value = '' }
  catch (e) { if (String(e) === 'Confirmation is required') confirm.value = true; else error.value = String(e) }
}
onMounted(() => { void reload() })
</script>
<template>
  <div class="space-y-3">
    <div class="flex flex-wrap gap-2"><Button v-for="agent in agents" :key="agent.id" variant="outline" @click="select(agent)">{{ agent.name }} · {{ agent.provider }}</Button></div>
    <template v-if="selected">
      <p>Agent: {{ selected.name }} · {{ selected.provider }}</p>
      <label for="agent-model">Model</label><Input id="agent-model" v-model="selected.model" />
      <label for="agent-prompt">System prompt</label><Textarea id="agent-prompt" v-model="selected.systemPrompt" />
      <p>Approval policy</p><div class="flex gap-2"><div v-for="choice in review?.choices" :key="choice.policy"><Button :disabled="!choice.enabled" :variant="selected.policy === choice.policy ? 'default' : 'outline'" @click="selected.policy = choice.policy">{{ choice.policy === 'ask' ? 'Ask before tool calls' : 'Deny all' }}</Button><p v-if="choice.reason">{{ choice.reason }}</p></div></div>
      <div class="flex items-center gap-2"><span>plugin.write</span><Switch aria-label="plugin.write" v-model="selected.pluginWrite" /></div>
      <p v-for="change in review?.changes" :key="change.field" class="whitespace-pre-wrap">{{ change.field }}: {{ change.before }} → {{ change.after }}</p>
      <p v-if="review?.blocked">{{ review.blocked }}</p><p v-if="error" role="alert" data-protected="true" class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]">{{ error }}</p>
      <Button @click="save(false)">Save</Button>
      <div v-if="confirm" data-protected="true" class="space-y-2 border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"><p>This lets the agent do more without asking.</p><Button variant="destructive" @click="save(true)">Confirm and save</Button></div>
      <div v-if="saved"><p>Applies at next launch</p><Button @click="runAction('relaunch')">Relaunch</Button></div>
    </template>
  </div>
</template>
