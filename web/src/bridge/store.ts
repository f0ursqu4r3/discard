import { onMounted, onUnmounted, reactive, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getHistory, getSnapshot } from './api'
import { applyTheme } from './theme'
import type { HistoryDto, MessageDto, SnapshotDto } from './types'
export function useStore() {
  const state = reactive<{
    snapshot: SnapshotDto | null
    history: MessageDto[]
    error: string | null
  }>({ snapshot: null, history: [], error: null })
  watch(
    () => state.snapshot?.theme,
    (theme) => {
      if (theme) applyTheme(theme, document.documentElement)
    },
  )
  const unlisten: UnlistenFn[] = []
  const histories = new Map<string, HistoryDto>()
  let disposed = false
  function acceptHistory(history: HistoryDto) {
    if (disposed || (histories.get(history.channelId)?.rev ?? -1) > history.rev) return
    histories.set(history.channelId, history)
    if (state.snapshot?.channelId === history.channelId && history.rev >= state.snapshot.historyRev)
      state.history = history.messages
  }
  function acceptSnapshot(snapshot: SnapshotDto) {
    if (disposed || (state.snapshot && snapshot.rev < state.snapshot.rev)) return
    const changed = state.snapshot?.channelId !== snapshot.channelId
    state.snapshot = snapshot
    const cached = histories.get(snapshot.channelId)
    if (cached && cached.rev >= snapshot.historyRev) state.history = cached.messages
    else if (changed) state.history = []
    if (!cached || cached.rev < snapshot.historyRev)
      void getHistory()
        .then(acceptHistory)
        .catch((error) => {
          state.error = String(error)
        })
  }
  onMounted(async () => {
    try {
      for (const [name, callback] of [
        ['discard://snapshot', (payload: SnapshotDto) => acceptSnapshot(payload)],
        ['discard://history', (payload: HistoryDto) => acceptHistory(payload)],
      ] as const) {
        const stop = await listen(name, (event) =>
          callback(event.payload as SnapshotDto & HistoryDto),
        )
        if (disposed) {
          stop()
          return
        }
        unlisten.push(stop)
      }
      acceptSnapshot(await getSnapshot())
    } catch (error) {
      if (!disposed) state.error = String(error)
    }
  })
  onUnmounted(() => {
    disposed = true
    unlisten.forEach((stop) => stop())
  })
  return Object.assign(state, { refresh: async () => acceptSnapshot(await getSnapshot()) })
}
