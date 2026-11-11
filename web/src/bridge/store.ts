import { onMounted, onUnmounted, reactive, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getHistory, getSnapshot, getThreadHistory } from './api'
import { applyTheme } from './theme'
import type { HistoryDto, MessageDto, SnapshotDto, ThreadHistoryDto } from './types'
export function useStore() {
  const state = reactive<{
    snapshot: SnapshotDto | null
    history: MessageDto[]
    /** The transcript of `snapshot.openThread`, at a revision that is at least `openThread.historyRev`. Null when no thread is open or it has not loaded. */
    threadHistory: ThreadHistoryDto | null
    error: string | null
  }>({ snapshot: null, history: [], threadHistory: null, error: null })
  watch(
    () => state.snapshot?.theme,
    (theme) => {
      if (theme) applyTheme(theme, document.documentElement)
    },
  )
  const unlisten: UnlistenFn[] = []
  const histories = new Map<string, HistoryDto>()
  // Only the open thread's transcript is kept. A closed or replaced thread drops its entry.
  const threadHistories = new Map<string, ThreadHistoryDto>()
  let threadFetch = ''
  let disposed = false
  function acceptThreadHistory(history: ThreadHistoryDto) {
    if (disposed || (threadHistories.get(history.channelId)?.rev ?? -1) > history.rev) return
    threadHistories.set(history.channelId, history)
    const open = state.snapshot?.openThread
    if (open?.channelId === history.channelId && history.rev >= open.historyRev)
      state.threadHistory = history
  }
  function syncThread(snapshot: SnapshotDto) {
    const open = snapshot.openThread
    if (!open) {
      threadHistories.clear()
      state.threadHistory = null
      return
    }
    for (const id of threadHistories.keys()) if (id !== open.channelId) threadHistories.delete(id)
    const cached = threadHistories.get(open.channelId)
    if (cached && cached.rev >= open.historyRev) state.threadHistory = cached
    else if (state.threadHistory?.channelId !== open.channelId) state.threadHistory = null
    const key = `${open.channelId}:${open.historyRev}`
    if ((!cached || cached.rev < open.historyRev) && threadFetch !== key) {
      threadFetch = key
      void getThreadHistory()
        .then((history) => {
          if (history) acceptThreadHistory(history)
        })
        .catch((error) => {
          state.error = String(error)
        })
        .finally(() => {
          if (threadFetch === key) threadFetch = ''
        })
    }
  }
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
    syncThread(snapshot)
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
        ['discard://thread-history', (payload: ThreadHistoryDto) => acceptThreadHistory(payload)],
      ] as const) {
        const stop = await listen(name, (event) =>
          callback(event.payload as SnapshotDto & HistoryDto & ThreadHistoryDto),
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
