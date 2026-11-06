<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
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
  <main class="flex h-screen overflow-hidden bg-background text-foreground">
    <aside
      class="w-60 shrink-0 space-y-2 overflow-auto border-r border-border bg-sidebar p-4 text-sidebar-foreground"
    >
      <Input v-model="query" aria-label="Search settings" placeholder="Search settings" />
      <nav class="flex flex-col gap-1">
        <Button
          v-for="entry in entries"
          :key="entry.title"
          :data-page="entry.title"
          :disabled="!entry.available"
          :variant="page === entry.title.toLowerCase() ? 'secondary' : 'ghost'"
          class="justify-start"
          @click="select(entry.title)"
        >
          {{ entry.title }}
          <span v-if="!entry.available" class="ml-auto text-muted-foreground">Soon</span>
        </Button>
      </nav>
    </aside>
    <section class="min-w-0 flex-1 overflow-auto p-6">
      <h1 class="mb-6 text-xl font-semibold">
        {{ entries.find((e) => e.title.toLowerCase() === page)?.title || page }}
      </h1>
      <General v-if="page === 'general'" />
      <Appearance v-else-if="page === 'appearance'" />
      <Providers v-else-if="page === 'providers'" />
      <Workspaces v-else-if="page === 'workspaces'" />
      <Memory v-else-if="page === 'memory'" />
      <Advanced v-else-if="page === 'advanced'" />
      <Plugins v-else-if="page === 'plugins'" />
      <Keybindings v-else-if="page === 'keybindings'" />
      <Logs v-else-if="page === 'logs'" />
    </section>
  </main>
</template>
