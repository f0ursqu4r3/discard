import { onMounted, onUnmounted, reactive, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  decideAgentRequest as decideAgentRequestApi,
  getAgentRequests,
  getHistory,
  getSnapshot,
  getThreadHistory,
  type AgentRequest,
} from './api'
import { applyTheme } from './theme'
import { currentWindowLabel } from './window'
import type {
  HistoryDto,
  MessageDto,
  SnapshotDto,
  ThreadApprovalDto,
  ThreadChipDto,
  ThreadHistoryDto,
} from './types'
export function useStore() {
  const state = reactive<{
    snapshot: SnapshotDto | null
    history: MessageDto[]
    /** The transcript of `snapshot.openThread`, at a revision that is at least `openThread.historyRev`. Null when no thread is open or it has not loaded. */
    threadHistory: ThreadHistoryDto | null
    /**
     * The chips of the sub-agent threads of the running turn of the selected channel, in task order. They have no
     * anchor yet. When the turn ends they appear on `MessageDto.subAgentThreads` of the saved response.
     */
    liveSubAgentThreads: ThreadChipDto[]
    /** Threads with an approval that waits, followed or not. The Approvals page lists these next to the channels'. */
    threadApprovals: ThreadApprovalDto[]
    /**
     * The agent add requests of the selected channel, oldest first, decided or not. They load when the channel
     * changes and when its turn ends (an agent asks during a turn and the turn ends), and after a decision.
     */
    agentRequests: AgentRequest[]
    error: string | null
  }>({
    snapshot: null,
    history: [],
    threadHistory: null,
    liveSubAgentThreads: [],
    threadApprovals: [],
    agentRequests: [],
    error: null,
  })
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
  let requestsFetch = 0
  let disposed = false
  async function loadAgentRequests(channelId: string) {
    const current = ++requestsFetch
    try {
      const rows = await getAgentRequests(channelId)
      if (disposed || current !== requestsFetch || state.snapshot?.channelId !== channelId) return
      state.agentRequests = Array.isArray(rows) ? rows : []
    } catch (error) {
      if (!disposed && current === requestsFetch) state.error = String(error)
    }
  }
  /** Decide a request of the selected channel. The list loads again either way: a decision can save and its follow-up still fail. */
  async function decideAgentRequest(requestId: string, decision: 'add' | 'deny') {
    const channelId = state.snapshot?.channelId
    if (!channelId) throw new Error('No channel is selected.')
    try {
      return await decideAgentRequestApi(channelId, requestId, decision)
    } finally {
      await loadAgentRequests(channelId)
    }
  }
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
    const turnEnded = !!state.snapshot?.turnRunning && !snapshot.turnRunning
    state.snapshot = snapshot
    state.liveSubAgentThreads = snapshot.liveSubAgentThreads ?? []
    state.threadApprovals = snapshot.threadApprovals ?? []
    if (changed) state.agentRequests = []
    if (snapshot.channelId && (changed || turnEnded)) void loadAgentRequests(snapshot.channelId)
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
      // A listener with the default target receives the events that the core sends to another window. These three
      // events belong to the view of one window, so the store listens for its own window only.
      const target = { kind: 'WebviewWindow', label: currentWindowLabel() } as const
      for (const [name, callback] of [
        ['discard://snapshot', (payload: SnapshotDto) => acceptSnapshot(payload)],
        ['discard://history', (payload: HistoryDto) => acceptHistory(payload)],
        ['discard://thread-history', (payload: ThreadHistoryDto) => acceptThreadHistory(payload)],
      ] as const) {
        const stop = await listen(
          name,
          (event) => callback(event.payload as SnapshotDto & HistoryDto & ThreadHistoryDto),
          { target },
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
  return Object.assign(state, {
    refresh: async () => acceptSnapshot(await getSnapshot()),
    decideAgentRequest,
    refreshAgentRequests: async () => {
      const channelId = state.snapshot?.channelId
      if (channelId) await loadAgentRequests(channelId)
    },
  })
}
