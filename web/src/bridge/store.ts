import { onMounted, onUnmounted, reactive } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getHistory, getSnapshot } from './api'
import { applyTheme } from './theme'
import { watch } from 'vue'
import type { MessageDto, SnapshotDto } from './types'
export function useStore() {
  const state = reactive<{ snapshot: SnapshotDto | null; history: MessageDto[] }>({ snapshot: null, history: [] })
  watch(() => state.snapshot?.theme, theme => { if (theme) applyTheme(theme, document.documentElement) })
  const unlisten: UnlistenFn[] = []
  let historyChanged = false
  onMounted(async () => {
    unlisten.push(await listen<SnapshotDto>('discard://snapshot', event => { if (!state.snapshot || event.payload.rev >= state.snapshot.rev) state.snapshot = event.payload }))
    unlisten.push(await listen<MessageDto[]>('discard://history', event => { historyChanged = true; state.history = event.payload }))
    const snapshot = await getSnapshot()
    if (!state.snapshot || snapshot.rev >= state.snapshot.rev) state.snapshot = snapshot
    const history = await getHistory()
    if (!historyChanged) state.history = history
  })
  onUnmounted(() => unlisten.forEach(stop => stop()))
  return state
}
