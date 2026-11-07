<script setup lang="ts">
defineOptions({ name: 'SettingsAppearance' })
import { onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Button } from '@/components/ui/button'
import AppearancePreview from '../AppearancePreview.vue'
import { defaultAppearance, type AppearancePreferences } from '@/bridge/appearance'
import { Switch } from '@/components/ui/switch'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

type Appearance = AppearancePreferences & {
  themeSource: string
  theme: string
  ignorePluginThemes: boolean
  single: boolean
  light: string
  dark: string
  activeName: string
  origin: string | null
  file: string
  config: string
  themes: [string, string][]
  swatches: [string, string][]
  problems: string[]
  overrides: [string, string][]
}
const value = ref<Appearance | null>(null)
const error = ref('')
const saving = ref(false)
let reloadId = 0
const themeSearch = ref('')
let stop: UnlistenFn | undefined
async function reload() {
  const id = ++reloadId
  const next = await invoke<Appearance>('settings_get', { page: 'appearance' })
  if (id === reloadId) value.value = { ...defaultAppearance, ...next }
}
async function run(command: string, args: Record<string, unknown>) {
  saving.value = true
  try {
    await invoke(command, args)
    error.value = ''
    await reload()
  } catch (e) {
    error.value = String(e)
  } finally {
    saving.value = false
  }
}
function patch(change: Record<string, unknown>) {
  return run('settings_set_appearance', { patch: change })
}
function choose(choice: string, name?: string) {
  return run('settings_choose_theme', { choice, name: name ?? null })
}
onMounted(async () => {
  stop = await listen('discard://settings', reload)
  await reload()
})
onUnmounted(() => stop?.())
</script>
<template>
  <div v-if="value" class="space-y-5">
    <div
      v-if="value.problems?.length"
      data-protected="true"
      class="rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"
    >
      <strong>Problems with the theme</strong>
      <p v-for="problem in value.problems" :key="problem">{{ problem }}</p>
    </div>
    <p
      v-if="error"
      role="alert"
      data-protected="true"
      class="rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-2 text-[var(--protected-fg)]"
    >
      {{ error }}
    </p>
    <section class="space-y-4" aria-label="Layout and messages">
      <h2 class="font-semibold">Layout and messages</h2>
      <div class="flex items-center justify-between gap-4">
        <div>
          <label for="mini-project-rail" class="font-medium">Mini project rail</label>
          <p id="mini-project-rail-help" class="text-sm text-muted-foreground">
            Use a 48 px rail with smaller icons at the default scale.
          </p>
        </div>
        <Switch
          id="mini-project-rail"
          aria-describedby="mini-project-rail-help"
          :model-value="value.miniProjectRail"
          :disabled="saving"
          @update:model-value="patch({ miniProjectRail: $event })"
        />
      </div>
      <div class="flex items-center justify-between gap-4">
        <div>
          <label for="message-density" class="font-medium">Message density</label>
          <p id="message-density-help" class="text-sm text-muted-foreground">
            Compact reduces spacing and keeps the same text size.
          </p>
        </div>
        <Select
          :model-value="value.messageDensity"
          :disabled="saving"
          @update:model-value="patch({ messageDensity: String($event) })"
        >
          <SelectTrigger id="message-density" aria-describedby="message-density-help" class="w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="comfortable">Comfortable</SelectItem>
            <SelectItem value="compact">Compact</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div class="flex items-center justify-between gap-4">
        <div>
          <label for="show-message-avatars" class="font-medium">Show message avatars</label>
          <p id="show-message-avatars-help" class="text-sm text-muted-foreground">
            Hide avatars to give messages more room. Author names stay visible.
          </p>
        </div>
        <Switch
          id="show-message-avatars"
          aria-describedby="show-message-avatars-help"
          :model-value="value.showMessageAvatars"
          :disabled="saving"
          @update:model-value="patch({ showMessageAvatars: $event })"
        />
      </div>
      <AppearancePreview :preferences="value" />
      <p class="text-sm text-muted-foreground">
        Changes apply to all conversations and are saved automatically.
      </p>
    </section>
    <section class="space-y-2">
      <h2 class="font-semibold">Theme source</h2>
      <div class="flex gap-2">
        <Button
          v-for="source in ['default', 'ghostty', 'custom']"
          :key="source"
          :variant="value.themeSource === source ? 'default' : 'outline'"
          class="capitalize"
          @click="patch({ themeSource: source })"
        >
          {{ source }}
        </Button>
      </div>
      <p class="text-sm text-muted-foreground">
        {{
          value.themeSource === 'default'
            ? 'Catppuccin, light or dark with your system'
            : value.themeSource === 'ghostty'
              ? 'Use the theme in your Ghostty config'
              : 'Pick the themes below'
        }}
      </p>
    </section>
    <section v-if="value.themeSource === 'custom'" class="space-y-2">
      <div class="flex items-center gap-3">
        <span class="font-semibold">Use one theme for both</span>
        <Switch
          aria-label="Use one theme for both"
          :model-value="value.single"
          @update:model-value="choose($event ? 'single' : 'pair')"
        />
      </div>
      <div class="flex gap-3">
        <div
          v-for="kind in value.single ? ['light'] : ['light', 'dark']"
          :key="kind"
          class="space-y-1"
        >
          <p>{{ value.single ? 'Theme' : kind === 'light' ? 'Light theme' : 'Dark theme' }}</p>
          <Select
            @update:open="themeSearch = ''"
            :model-value="kind === 'light' ? value.light : value.dark"
            @update:model-value="choose(kind, String($event))"
          >
            <SelectTrigger class="w-60"><SelectValue /></SelectTrigger>
            <SelectContent>
              <Input
                v-model="themeSearch"
                aria-label="Search themes"
                placeholder="Search themes"
                @keydown.stop
              />
              <SelectGroup v-for="origin in ['Yours', 'Ghostty', 'Bundled']" :key="origin">
                <SelectLabel>{{ origin }}</SelectLabel>
                <SelectItem
                  v-for="item in value.themes?.filter(
                    (t) =>
                      t[1] === origin && t[0].toLowerCase().includes(themeSearch.toLowerCase()),
                  )"
                  :key="item[0]"
                  :value="item[0]"
                >
                  {{ item[0] }}
                </SelectItem>
              </SelectGroup>
            </SelectContent>
          </Select>
        </div>
      </div>
      <p class="text-sm text-muted-foreground">
        {{
          value.single
            ? 'The same theme in light and dark'
            : "Discard switches with your system's light and dark setting"
        }}
      </p>
    </section>
    <section class="space-y-2">
      <div class="flex items-center gap-3">
        <strong>{{ value.activeName }}</strong>
        <span class="text-sm text-muted-foreground">{{ value.origin }}</span>
        <Button variant="outline" @click="run('settings_copy_theme', { name: value.activeName })">
          Edit a copy
        </Button>
      </div>
      <p class="text-sm text-muted-foreground">Active theme file: {{ value.file }}</p>
      <p class="text-sm text-muted-foreground">Ghostty config: {{ value.config }}</p>
      <strong>Swatches</strong>
      <div class="flex flex-wrap gap-2">
        <div
          v-for="(swatch, index) in value.swatches"
          :key="swatch[0]"
          class="flex items-center gap-1"
        >
          <span
            class="size-6 rounded-sm border border-border bg-[var(--swatch-color)]"
            :style="{ '--swatch-color': swatch[1] }"
            :title="swatch[0]"
          />
          <span v-if="index >= 16" class="text-sm text-muted-foreground">{{ swatch[0] }}</span>
        </div>
      </div>
    </section>
    <section class="space-y-1">
      <div class="flex items-center gap-3">
        <span class="font-semibold">Ignore plugin themes</span>
        <Switch
          aria-label="Ignore plugin themes"
          :model-value="value.ignorePluginThemes"
          @update:model-value="patch({ ignorePluginThemes: $event })"
        />
      </div>
      <p class="text-sm text-muted-foreground">
        Keep the theme colors above, whatever plugins ask for
      </p>
    </section>
    <section>
      <strong>Plugin overrides</strong>
      <p v-if="value.ignorePluginThemes">Plugin themes are off</p>
      <p v-else-if="!value.overrides?.length">No plugin changes the theme</p>
      <p v-for="item in value.ignorePluginThemes ? [] : value.overrides" :key="item[0]">
        {{ item[0] }} from {{ item[1] }}
      </p>
    </section>
  </div>
</template>
