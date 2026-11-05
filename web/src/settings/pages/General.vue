<script setup lang="ts">
defineOptions({ name: "SettingsGeneral" })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
type General = { defaultProvider: string; confirmWhileWorking: boolean; providers: string[]; issues: string[]; themeProblems: string[] }
const value = ref<General | null>(null)
const error = ref('')
let stop: UnlistenFn | undefined
async function reload() { value.value = await invoke<General>('settings_get', { page: 'general' }) }
async function patch(change: Record<string, unknown>) {
  try { await invoke('settings_set_general', { patch: change }); error.value = ''; await reload() }
  catch (e) { error.value = String(e) }
}
onMounted(async () => { stop = await listen('discard://settings', reload); await reload() })
onUnmounted(() => stop?.())
</script>
<template>
  <div v-if="value" class="space-y-5">
    <div v-if="value.issues?.length" data-protected="true" class="rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"><strong>Problems in settings.toml</strong><p v-for="issue in value.issues" :key="issue">{{ issue }}</p></div>
    <div v-if="value.themeProblems?.length" data-protected="true" class="rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"><strong>Problems with the theme</strong><p v-for="problem in value.themeProblems" :key="problem">{{ problem }}</p></div>
    <p v-if="error" role="alert" data-protected="true" class="rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]">{{ error }}</p>
    <section class="space-y-2"><h2 class="font-semibold">Default provider</h2><div class="flex gap-2"><Button v-for="provider in value.providers" :key="provider" :variant="value.defaultProvider === provider ? 'default' : 'outline'" @click="patch({ defaultProvider: provider })">{{ provider }}</Button></div><p class="text-sm text-muted-foreground">Applies at next launch</p></section>
    <section class="space-y-2"><div class="flex items-center gap-3"><span class="font-semibold">Confirm while working</span><Switch aria-label="Confirm while working" :model-value="value.confirmWhileWorking" @update:model-value="patch({ confirmWhileWorking: $event })" /></div><p class="text-sm text-muted-foreground">Confirm Quit and Safe Mode relaunch while a turn runs</p></section>
    <p class="text-muted-foreground">Reopen last channel · Soon</p>
  </div>
</template>
