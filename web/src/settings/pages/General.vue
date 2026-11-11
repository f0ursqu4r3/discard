<script setup lang="ts">
defineOptions({ name: 'SettingsGeneral' })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import SettingsNotice from '../ui/SettingsNotice.vue'
import SettingsRow from '../ui/SettingsRow.vue'
import SettingsCard from '../ui/SettingsCard.vue'
type General = {
  defaultProvider: string
  confirmWhileWorking: boolean
  providers: string[]
  issues: string[]
  themeProblems: string[]
}
const value = ref<General | null>(null)
const error = ref('')
let stop: UnlistenFn | undefined
async function reload() {
  value.value = await invoke<General>('settings_get', { page: 'general' })
}
async function patch(change: Record<string, unknown>) {
  try {
    await invoke('settings_set_general', { patch: change })
    error.value = ''
    await reload()
  } catch (e) {
    error.value = String(e)
  }
}
onMounted(async () => {
  stop = await listen('discard://settings', reload)
  await reload()
})
onUnmounted(() => stop?.())
</script>
<template>
  <div v-if="value" class="space-y-4">
    <SettingsNotice v-if="value.issues?.length">
      <strong>Problems in settings.toml</strong>
      <p v-for="issue in value.issues" :key="issue">{{ issue }}</p>
    </SettingsNotice>
    <SettingsNotice v-if="value.themeProblems?.length">
      <strong>Problems with the theme</strong>
      <p v-for="problem in value.themeProblems" :key="problem">{{ problem }}</p>
    </SettingsNotice>
    <SettingsNotice v-if="error" as="p" role="alert">{{ error }}</SettingsNotice>
    <SettingsCard title="Startup">
      <SettingsRow label="Default provider" help="Applies at next launch">
        <div class="flex gap-1.5">
          <Button
            v-for="provider in value.providers"
            :key="provider"
            size="sm"
            :variant="value.defaultProvider === provider ? 'default' : 'outline'"
            @click="patch({ defaultProvider: provider })"
          >
            {{ provider }}
          </Button>
        </div>
      </SettingsRow>
      <!-- TODO: Restore the last selected channel on startup. -->
      <SettingsRow label="Reopen last channel" help="Restore the last selected channel on startup">
        <Button size="sm" variant="outline" disabled aria-label="Reopen last channel · Soon">
          Soon
        </Button>
      </SettingsRow>
    </SettingsCard>
    <SettingsCard title="Safety">
      <SettingsRow
        label="Confirm while working"
        label-for="confirm-while-working"
        help="Confirm Quit and Safe Mode relaunch while a turn runs"
      >
        <Switch
          id="confirm-while-working"
          aria-label="Confirm while working"
          :model-value="value.confirmWhileWorking"
          @update:model-value="patch({ confirmWhileWorking: $event })"
        />
      </SettingsRow>
    </SettingsCard>
  </div>
</template>
