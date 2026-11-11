<script setup lang="ts">
defineOptions({ name: 'SettingsPlugins' })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { runAction } from '@/bridge/api'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import SettingsNotice from '../ui/SettingsNotice.vue'
import SettingsCard from '../ui/SettingsCard.vue'
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
    <SettingsNotice v-if="data.safeMode" class="flex items-center gap-2">
      <span class="flex-1">Safe mode: user plugins disabled</span>
      <Button size="sm" @click="runAction('toggle_safe_mode')">Relaunch normally</Button>
    </SettingsNotice>
    <SettingsNotice v-if="data.problems.length || error">
      <strong>Plugin problems</strong>
      <p v-for="problem in data.problems" :key="problem">{{ problem }}</p>
      <p v-if="error" role="alert">{{ error }}</p>
    </SettingsNotice>
    <SettingsNotice v-if="data.notice" class="flex items-center gap-2">
      <span class="flex-1">{{ data.notice }}</span>
      <Button size="sm" variant="outline" @click="run('settings_plugin_dismiss_notice', {})">
        Dismiss
      </Button>
    </SettingsNotice>
    <SettingsCard title="Waiting for your review">
      <template v-if="data.reviews.length" #actions>
        <span class="text-muted-foreground">{{ data.reviews.length }} pending</span>
      </template>
      <div class="flex flex-col gap-3">
        <p v-if="!data.reviews.length" class="text-sm text-muted-foreground">Nothing to review.</p>
        <SettingsNotice
          v-for="review in data.reviews"
          :key="review.id"
          as="article"
          class="space-y-2"
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
            class="max-h-72 overflow-auto border border-[var(--protected-border)] p-2 font-mono text-xs whitespace-pre-wrap"
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
              size="sm"
              variant="destructive"
              class="ml-auto"
              @click="run('settings_plugin_reject', { reviewId: review.id })"
            >
              Reject
            </Button>
            <Button size="sm" @click="run('settings_plugin_approve', { reviewId: review.id })">
              Approve and run
            </Button>
          </div>
        </SettingsNotice>
      </div>
    </SettingsCard>
    <SettingsCard title="Installed plugins" flush>
      <p v-if="!data.statuses.length" class="px-3 py-2 text-sm text-muted-foreground">
        No user plugins are installed. The built-in interface is a bundled plugin and is not listed
        here.
      </p>
      <div
        v-for="status in data.statuses"
        :key="status.name"
        class="border-b border-dashed border-border px-3 py-2 last:border-b-0 hover:bg-muted/40"
      >
        <div class="flex items-center gap-2">
          <strong class="font-mono text-sm font-medium">{{ status.name }}</strong>
          <span class="min-w-0 flex-1 truncate font-mono text-xs text-muted-foreground">
            {{ status.state }}
          </span>
          <span
            v-if="status.trusted"
            class="rounded-sm border border-border px-1.5 font-mono text-[10.5px] text-muted-foreground"
          >
            trusts future edits
          </span>
          <Button v-if="status.modified" size="sm" variant="outline" @click="showDiff(status.name)">
            Show diff
          </Button>
          <Button
            size="sm"
            variant="outline"
            @click="run('settings_plugin_rollback', { name: status.name })"
          >
            Roll back
          </Button>
        </div>
        <SettingsNotice v-if="diffs[status.name] !== undefined" class="mt-2">
          <p>Changes against the repository’s current commit</p>
          <pre class="max-h-72 overflow-auto font-mono text-xs whitespace-pre-wrap">{{
            diffs[status.name]
          }}</pre>
        </SettingsNotice>
      </div>
    </SettingsCard>
  </div>
</template>
