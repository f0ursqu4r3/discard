<script setup lang="ts">
defineOptions({ name: 'SettingsPlugins' })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { runAction } from '@/bridge/api'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
type Review = {
  id: string
  name: string
  kind: string
  description: string
  capabilities: string[]
  changes: string[]
  diff: string
  fullAccess: boolean
  trustOffered: boolean
}
type Status = { name: string; state: string; modified: boolean; trusted: boolean }
type Data = {
  reviews: Review[]
  statuses: Status[]
  problems: string[]
  safeMode: boolean
  notice?: string | null
}
const data = ref<Data | null>(null),
  trusted = ref<Record<string, boolean>>({}),
  error = ref(''),
  diffs = ref<Record<string, string>>({})
let timer: ReturnType<typeof setInterval>
async function reload() {
  data.value = await invoke<Data>('settings_plugins')
}
async function run(command: string, args: Record<string, unknown>) {
  try {
    await invoke(command, args)
    error.value = ''
    await reload()
  } catch (e) {
    error.value = String(e)
    await reload()
  }
}
async function showDiff(name: string) {
  try {
    diffs.value[name] = await invoke<string>('settings_plugin_modified_diff', { name })
  } catch (e) {
    error.value = String(e)
  }
}
async function setTrust(id: string, value: boolean) {
  try {
    await invoke('settings_plugin_trust', { reviewId: id, trusted: value })
    trusted.value[id] = value
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
  <div v-if="data" class="space-y-4">
    <div
      v-if="data.safeMode"
      data-protected="true"
      class="flex items-center gap-2 border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"
    >
      <span>Safe mode: user plugins disabled</span>
      <Button @click="runAction('toggle_safe_mode')">Relaunch normally</Button>
    </div>
    <div
      v-if="data.problems.length || error"
      data-protected="true"
      class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"
    >
      <strong>Plugin problems</strong>
      <p v-for="problem in data.problems" :key="problem">{{ problem }}</p>
      <p v-if="error" role="alert">{{ error }}</p>
    </div>
    <div
      v-if="data.notice"
      data-protected="true"
      class="flex items-center gap-2 border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"
    >
      <span class="flex-1">{{ data.notice }}</span>
      <Button variant="outline" @click="run('settings_plugin_dismiss_notice', {})">Dismiss</Button>
    </div>
    <h2 class="font-semibold">Waiting for your review</h2>
    <p v-if="!data.reviews.length">Nothing to review.</p>
    <article
      v-for="review in data.reviews"
      :key="review.id"
      data-protected="true"
      class="space-y-2 rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-3 text-[var(--protected-fg)]"
    >
      <h3 class="font-bold">{{ review.kind }}: {{ review.name }}</h3>
      <p>{{ review.description }}</p>
      <strong>This plugin asks to:</strong>
      <p v-for="capability in review.capabilities" :key="capability">• {{ capability }}</p>
      <p v-if="!review.capabilities.length">• nothing beyond running in the sandbox</p>
      <strong v-if="review.changes.length">Changed since the approved version:</strong>
      <p v-for="change in review.changes" :key="change">{{ change }}</p>
      <p v-if="review.fullAccess" class="border border-[var(--protected-border)] p-2 font-bold">
        FULL LOCAL ACCESS: this plugin can run programs. A program can read your files, use the
        network, and start other programs.
      </p>
      <strong>Code changes:</strong>
      <pre
        class="max-h-72 overflow-auto whitespace-pre-wrap border border-[var(--protected-border)] p-2"
        >{{ review.diff }}</pre>
      <div class="flex items-center gap-2">
        <template v-if="review.trustOffered">
          <span>Trust future edits</span>
          <Switch
            :model-value="!!trusted[review.id]"
            aria-label="Trust future edits"
            @update:model-value="setTrust(review.id, $event)"
          />
        </template>
        <Button
          variant="destructive"
          @click="run('settings_plugin_reject', { reviewId: review.id })"
        >
          Reject
        </Button>
        <Button @click="run('settings_plugin_approve', { reviewId: review.id })">
          Approve and run
        </Button>
      </div>
    </article>
    <h2 class="font-semibold">Installed plugins</h2>
    <p v-if="!data.statuses.length">
      No user plugins are installed. The built-in interface is a bundled plugin and is not listed
      here.
    </p>
    <div v-for="status in data.statuses" :key="status.name" class="space-y-2">
      <div class="flex items-center gap-2">
        <strong>{{ status.name }}</strong>
        <span>{{ status.state }}</span>
        <span v-if="status.trusted">trusts future edits</span>
        <Button v-if="status.modified" variant="outline" @click="showDiff(status.name)">
          Show diff
        </Button>
        <Button variant="outline" @click="run('settings_plugin_rollback', { name: status.name })">
          Roll back
        </Button>
      </div>
      <div
        v-if="diffs[status.name] !== undefined"
        data-protected="true"
        class="border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"
      >
        <p>Changes against the repository’s current commit</p>
        <pre class="max-h-72 overflow-auto whitespace-pre-wrap">{{ diffs[status.name] }}</pre>
      </div>
    </div>
  </div>
</template>
