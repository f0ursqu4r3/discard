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
import KeyValue from '../ui/KeyValue.vue'
import SettingsNotice from '../ui/SettingsNotice.vue'
import SettingsRow from '../ui/SettingsRow.vue'
import SettingsCard from '../ui/SettingsCard.vue'
import PathText from '../ui/PathText.vue'
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
  <div v-if="value" class="space-y-4">
    <SettingsNotice v-if="value.problems?.length">
      <strong>Problems with the theme</strong>
      <p v-for="problem in value.problems" :key="problem">{{ problem }}</p>
    </SettingsNotice>
    <SettingsNotice v-if="error" as="p" role="alert">{{ error }}</SettingsNotice>
    <SettingsCard title="Theme">
      <div>
        <SettingsRow
          label="Source"
          :help="
            value.themeSource === 'default'
              ? 'Catppuccin, light or dark with your system'
              : value.themeSource === 'ghostty'
                ? 'Use the theme in your Ghostty config'
                : 'Pick the themes below'
          "
        >
          <div class="flex gap-1.5">
            <Button
              v-for="source in ['default', 'ghostty', 'custom']"
              :key="source"
              size="sm"
              :variant="value.themeSource === source ? 'default' : 'outline'"
              class="capitalize"
              @click="patch({ themeSource: source })"
            >
              {{ source }}
            </Button>
          </div>
        </SettingsRow>
        <template v-if="value.themeSource === 'custom'">
          <SettingsRow
            label="Use one theme for both"
            label-for="use-one-theme"
            :help="
              value.single
                ? 'The same theme in light and dark'
                : 'Discard switches with your system\'s light and dark setting'
            "
          >
            <Switch
              id="use-one-theme"
              aria-label="Use one theme for both"
              :model-value="value.single"
              @update:model-value="choose($event ? 'single' : 'pair')"
            />
          </SettingsRow>
          <div class="grid gap-3 pt-3" :class="value.single ? 'grid-cols-1' : 'grid-cols-2'">
            <div v-for="kind in value.single ? ['light'] : ['light', 'dark']" :key="kind">
              <label :for="`theme-${kind}`" class="mb-1.5 block font-medium">
                {{ value.single ? 'Theme' : kind === 'light' ? 'Light theme' : 'Dark theme' }}
              </label>
              <Select
                @update:open="themeSearch = ''"
                :model-value="kind === 'light' ? value.light : value.dark"
                @update:model-value="choose(kind, String($event))"
              >
                <SelectTrigger :id="`theme-${kind}`" class="w-full"><SelectValue /></SelectTrigger>
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
        </template>
      </div>
      <div class="mt-3 space-y-3 border-t border-dashed border-border pt-3">
        <div class="flex items-center gap-2">
          <span class="font-semibold">{{ value.activeName }}</span>
          <span
            v-if="value.origin"
            class="rounded-sm border border-border px-1.5 font-mono text-[10.5px] text-muted-foreground"
          >
            {{ value.origin }}
          </span>
          <Button
            size="sm"
            variant="outline"
            class="ml-auto"
            @click="run('settings_copy_theme', { name: value.activeName })"
          >
            Edit a copy
          </Button>
        </div>
        <div class="space-y-2">
          <div
            class="grid grid-cols-8 overflow-hidden rounded-md border border-border"
            role="group"
            aria-label="Palette"
          >
            <span
              v-for="swatch in (value.swatches ?? []).slice(0, 16)"
              :key="swatch[0]"
              class="h-6 bg-[var(--swatch-color)]"
              :style="{ '--swatch-color': swatch[1] }"
              :title="swatch[0]"
            />
          </div>
          <div v-if="(value.swatches ?? []).length > 16" class="flex flex-wrap gap-x-3 gap-y-1.5">
            <span
              v-for="swatch in (value.swatches ?? []).slice(16)"
              :key="swatch[0]"
              class="inline-flex items-center gap-1.5 font-mono text-xs text-muted-foreground"
              :title="swatch[0]"
            >
              <span
                class="size-3 rounded-sm border border-border bg-[var(--swatch-color)]"
                :style="{ '--swatch-color': swatch[1] }"
              />
              {{ swatch[0] }}
            </span>
          </div>
        </div>
        <dl class="flex flex-col gap-1.5">
          <KeyValue label="theme file"><PathText :path="value.file" /></KeyValue>
          <KeyValue label="ghostty config"><PathText :path="value.config" /></KeyValue>
        </dl>
      </div>
    </SettingsCard>
    <SettingsCard title="Layout and messages" aria-label="Layout and messages" flush>
      <AppearancePreview :preferences="value" />
      <div class="px-3 pt-2 pb-3">
        <div>
          <SettingsRow
            label="Mini project rail"
            label-for="mini-project-rail"
            help-id="mini-project-rail-help"
            help="Use a 48 px rail with smaller icons at the default scale."
          >
            <Switch
              id="mini-project-rail"
              aria-describedby="mini-project-rail-help"
              :model-value="value.miniProjectRail"
              :disabled="saving"
              @update:model-value="patch({ miniProjectRail: $event })"
            />
          </SettingsRow>
          <SettingsRow
            label="Message density"
            label-for="message-density"
            help-id="message-density-help"
            help="Compact reduces spacing and keeps the same text size."
          >
            <Select
              :model-value="value.messageDensity"
              :disabled="saving"
              @update:model-value="patch({ messageDensity: String($event) })"
            >
              <SelectTrigger
                id="message-density"
                aria-describedby="message-density-help"
                class="w-40"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="comfortable">Comfortable</SelectItem>
                <SelectItem value="compact">Compact</SelectItem>
              </SelectContent>
            </Select>
          </SettingsRow>
          <SettingsRow
            label="Show message avatars"
            label-for="show-message-avatars"
            help-id="show-message-avatars-help"
            help="Hide avatars to give messages more room. Author names stay visible."
          >
            <Switch
              id="show-message-avatars"
              aria-describedby="show-message-avatars-help"
              :model-value="value.showMessageAvatars"
              :disabled="saving"
              @update:model-value="patch({ showMessageAvatars: $event })"
            />
          </SettingsRow>
        </div>
        <p class="pt-2 text-sm text-muted-foreground">
          Changes apply to all conversations and are saved automatically.
        </p>
      </div>
    </SettingsCard>
    <SettingsCard title="Plugin themes">
      <SettingsRow
        label="Ignore plugin themes"
        label-for="ignore-plugin-themes"
        help="Keep the theme colors above, whatever plugins ask for"
      >
        <Switch
          id="ignore-plugin-themes"
          aria-label="Ignore plugin themes"
          :model-value="value.ignorePluginThemes"
          @update:model-value="patch({ ignorePluginThemes: $event })"
        />
      </SettingsRow>
      <div class="pt-3">
        <p v-if="value.ignorePluginThemes" class="text-sm text-muted-foreground">
          Plugin themes are off
        </p>
        <p v-else-if="!value.overrides?.length" class="text-sm text-muted-foreground">
          No plugin changes the theme
        </p>
        <dl v-else class="flex flex-col gap-1.5">
          <KeyValue v-for="item in value.overrides" :key="item[0]" :label="item[0]">
            {{ item[1] }}
          </KeyValue>
        </dl>
      </div>
    </SettingsCard>
  </div>
</template>
