<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import {
  Brain,
  Cable,
  CircleDashed,
  FolderTree,
  Keyboard,
  Palette,
  Puzzle,
  ScrollText,
  SlidersHorizontal,
  Wrench,
} from '@lucide/vue'
import General from './pages/General.vue'
import Appearance from './pages/Appearance.vue'
import Providers from './pages/Providers.vue'
import Workspaces from './pages/Workspaces.vue'
import Memory from './pages/Memory.vue'
import Advanced from './pages/Advanced.vue'
import Plugins from './pages/Plugins.vue'
import Keybindings from './pages/Keybindings.vue'
import Logs from './pages/Logs.vue'
import { getSnapshot } from '@/bridge/api'
import { applyTheme } from '@/bridge/theme'
import type { SnapshotDto } from '@/bridge/types'

type Entry = { title: string; available: boolean }
// Unknown page titles fall back to a generic icon.
const icons: Record<string, unknown> = {
  general: SlidersHorizontal,
  appearance: Palette,
  providers: Cable,
  plugins: Puzzle,
  memory: Brain,
  workspaces: FolderTree,
  keybindings: Keyboard,
  logs: ScrollText,
  advanced: Wrench,
}
function iconFor(title: string) {
  return icons[title.toLowerCase()] ?? CircleDashed
}
const query = ref('')
const entries = ref<Entry[]>([])
const page = ref(settingsPage(location.hash))
function settingsPage(route: string) {
  const name = route.split('/')[2] || 'general'
  return name === 'agents' ? 'general' : name
}
const stops: UnlistenFn[] = []
let searchId = 0
async function search() {
  const id = ++searchId
  const result = await invoke<Entry[]>('settings_search', { query: query.value })
  if (id === searchId) entries.value = result
}
function select(title: string) {
  page.value = title.toLowerCase()
  location.hash = `/settings/${page.value}`
}
// The log pane fills the page height. Other pages scroll as a whole.
const fill = computed(() => page.value === 'logs')
watch(query, search)
onMounted(async () => {
  await search()
  stops.push(
    await listen<SnapshotDto>('discard://snapshot', (event) =>
      applyTheme(event.payload.theme, document.documentElement),
    ),
  )
  const snapshot = await getSnapshot()
  if (snapshot?.theme) applyTheme(snapshot.theme, document.documentElement)
  function openRoute(route: string) {
    page.value = settingsPage(route)
    location.hash = `/settings/${page.value}`
  }
  stops.push(await listen<string>('discard://settings-page', (event) => openRoute(event.payload)))
  const route = await invoke<string | null>('settings_current_page')
  if (typeof route === 'string') openRoute(route)
})
onUnmounted(() => stops.forEach((stop) => stop()))
</script>
<template>
  <!-- The Settings window has a native titlebar, so there is no drag toolbar here. -->
  <main class="flex h-screen gap-1.5 overflow-hidden bg-canvas p-1.5 text-sm text-foreground">
    <aside
      class="flex w-60 shrink-0 flex-col overflow-hidden rounded-xl border border-border/60 bg-sidebar text-sidebar-foreground"
    >
      <div class="p-2.5 pb-1.5">
        <Input v-model="query" aria-label="Search settings" placeholder="Search settings" />
      </div>
      <h2 class="section-label px-2.5 pt-2 pb-1.5">Settings</h2>
      <nav class="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-2.5 pt-0">
        <Button
          v-for="entry in entries"
          :key="entry.title"
          :data-page="entry.title"
          :disabled="!entry.available"
          :aria-current="page === entry.title.toLowerCase() ? 'page' : undefined"
          size="sm"
          :variant="page === entry.title.toLowerCase() ? 'secondary' : 'ghost'"
          :class="{ 'nav-active-edge': page === entry.title.toLowerCase() }"
          class="h-8 justify-start gap-2 px-2 font-normal"
          @click="select(entry.title)"
        >
          <component :is="iconFor(entry.title)" data-icon="inline-start" />
          {{ entry.title }}
          <span
            v-if="!entry.available"
            class="ml-auto font-mono text-[10.5px] text-muted-foreground"
          >
            Soon
          </span>
        </Button>
      </nav>
    </aside>
    <section
      class="flex min-w-0 flex-1 flex-col overflow-hidden rounded-xl border border-border/60 bg-background"
    >
      <header class="flex h-11 shrink-0 items-baseline gap-3 border-b border-border/60 px-5 py-2.5">
        <h1 class="text-base font-semibold">
          {{ entries.find((e) => e.title.toLowerCase() === page)?.title || page }}
        </h1>
        <span class="font-mono text-xs text-muted-foreground">settings/{{ page }}</span>
      </header>
      <div class="flex min-h-0 flex-1 flex-col" :class="fill ? 'overflow-hidden' : 'overflow-auto'">
        <div
          class="mx-auto flex w-full max-w-3xl flex-1 flex-col p-5"
          :class="fill ? 'min-h-0' : ''"
        >
          <General v-if="page === 'general'" />
          <Appearance v-else-if="page === 'appearance'" />
          <Providers v-else-if="page === 'providers'" />
          <Workspaces v-else-if="page === 'workspaces'" />
          <Memory v-else-if="page === 'memory'" />
          <Advanced v-else-if="page === 'advanced'" />
          <Plugins v-else-if="page === 'plugins'" />
          <Keybindings v-else-if="page === 'keybindings'" />
          <Logs v-else-if="page === 'logs'" />
        </div>
      </div>
    </section>
  </main>
</template>
