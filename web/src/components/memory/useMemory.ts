import { onMounted, onUnmounted, ref, toValue, watch, type MaybeRefOrGetter } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { MemoryView } from './types'

export function useMemory(
  channel: MaybeRefOrGetter<string | null>,
  enabled: MaybeRefOrGetter<boolean> = true,
  project: MaybeRefOrGetter<string | null> = null,
) {
  const data = ref<MemoryView>({
    items: [],
    scopes: [],
    proposals: [],
    responses: [],
    paused: false,
  })
  const error = ref('')
  const loading = ref(false)
  let request = 0
  let disposed = false
  let stop: UnlistenFn | undefined
  async function reload() {
    const ticket = ++request
    data.value = toValue(enabled)
      ? data.value
      : { items: [], scopes: [], proposals: [], responses: [], paused: false }
    if (!toValue(enabled)) {
      loading.value = false
      return
    }
    loading.value = true
    try {
      const next = await invoke<MemoryView>('memory_get', {
        channelId: toValue(channel),
        ...(toValue(project) !== null ? { projectId: toValue(project) } : {}),
      })
      if (!disposed && ticket === request && next) {
        data.value = next
        error.value = ''
      }
    } catch (cause) {
      if (!disposed && ticket === request) error.value = String(cause)
    } finally {
      if (ticket === request) loading.value = false
    }
  }
  watch([() => toValue(channel), () => toValue(enabled), () => toValue(project)], () => {
    error.value = ''
    data.value = { items: [], scopes: [], proposals: [], responses: [], paused: false }
    void reload()
  })
  onMounted(async () => {
    try {
      const unlisten = await listen('discard://memory', () => {
        void reload()
      })
      if (disposed) unlisten()
      else stop = unlisten
    } catch (cause) {
      if (!disposed) error.value = String(cause)
    }
    if (!disposed) await reload()
  })
  onUnmounted(() => {
    disposed = true
    ++request
    stop?.()
  })
  return { data, error, loading, reload }
}
