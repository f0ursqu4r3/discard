<script setup lang="ts">
defineOptions({ name: 'SettingsAdvanced' })
import { onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/button'
import SettingsNotice from '../ui/SettingsNotice.vue'
import SettingsCard from '../ui/SettingsCard.vue'
type Action = { id: string; title: string; lines: string[] }
const actions = ref<Action[]>([]),
  pending = ref<string | null>(null),
  result = ref<Record<string, string>>({}),
  error = ref('')
async function run(id: string) {
  try {
    result.value[id] = await invoke<string>('settings_advanced_run', {
      actionId: id,
      confirmed: true,
    })
    pending.value = null
    error.value = ''
  } catch (e) {
    error.value = String(e)
  }
}
onMounted(async () => {
  actions.value = await invoke<Action[]>('settings_advanced_actions')
})
</script>
<template>
  <div class="space-y-4">
    <SettingsNotice v-if="error" as="p" role="alert">{{ error }}</SettingsNotice>
    <SettingsCard v-for="action in actions" :key="action.id" :title="action.title">
      <div class="flex items-start justify-between gap-4">
        <div class="min-w-0 space-y-1 text-sm text-muted-foreground">
          <p v-for="line in action.lines" :key="line">{{ line }}</p>
        </div>
        <Button v-if="pending !== action.id" size="sm" @click="pending = action.id">
          {{ action.title }}
        </Button>
      </div>
      <SettingsNotice v-if="pending === action.id" class="mt-2 flex gap-2">
        <Button size="sm" @click="run(action.id)">Confirm</Button>
        <Button size="sm" variant="outline" @click="pending = null">Cancel</Button>
      </SettingsNotice>
      <p v-if="result[action.id]" class="mt-2 text-sm">{{ result[action.id] }}</p>
    </SettingsCard>
  </div>
</template>
