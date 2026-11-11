import { expect, test, vi, beforeEach } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import { defineComponent, nextTick } from 'vue'
import snapshotFixture from '../fixtures/snapshot.json'
import threadFixture from '../fixtures/thread.json'
import subAgentFixture from '../fixtures/subagent.json'
import type { AgentRequest } from '../api'
import type { HistoryDto, OpenThreadDto, SnapshotDto, ThreadHistoryDto } from '../types'
const invoke = vi.hoisted(() => vi.fn())
const listen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))
import { useStore } from '../store'
const snapshot = snapshotFixture as SnapshotDto
beforeEach(() => {
  vi.clearAllMocks()
  listen.mockResolvedValue(() => {})
})
function setup() {
  let store!: ReturnType<typeof useStore>
  const wrapper = mount(
    defineComponent({
      setup() {
        store = useStore()
        return () => null
      },
    }),
  )
  const event = <T>(name: string, payload: T) =>
    listen.mock.calls.find(([type]) => type === `discard://${name}`)?.[1]({ payload })
  return { store, wrapper, event }
}
test('loads the snapshot and follows snapshot events', async () => {
  invoke.mockImplementation((name: string) =>
    Promise.resolve(
      name === 'get_snapshot' ? snapshot : { channelId: snapshot.channelId, rev: 0, messages: [] },
    ),
  )
  const { store, event, wrapper } = setup()
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('get_snapshot'))
  expect(invoke.mock.calls.filter(([name]) => name === 'get_snapshot')).toHaveLength(1)
  event('snapshot', { ...snapshot, project: 'Updated' })
  await nextTick()
  expect(store.snapshot?.project).toBe('Updated')
  wrapper.unmount()
})
test('late history and stale snapshots cannot replace another channel transcript', async () => {
  let resolveHistory!: (value: HistoryDto) => void
  invoke.mockImplementation((name: string) =>
    name === 'get_snapshot'
      ? Promise.resolve(snapshot)
      : new Promise<HistoryDto>((resolve) => {
          resolveHistory = resolve
        }),
  )
  const { store, event, wrapper } = setup()
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('get_history'))
  const late = resolveHistory
  const next: SnapshotDto = {
    ...snapshot,
    channelId: 'second',
    channel: 'tests',
    rev: 5,
    historyRev: 3,
  }
  event('snapshot', next)
  event('history', {
    channelId: 'second',
    rev: 3,
    messages: [{ id: 'new', role: 'user', blocks: [], continuesGroup: false }],
  })
  late({
    channelId: snapshot.channelId,
    rev: 1,
    messages: [{ id: 'old', role: 'user', blocks: [], continuesGroup: false }],
  })
  event('snapshot', { ...snapshot, rev: 2 })
  await nextTick()
  expect(store.snapshot?.channelId).toBe('second')
  expect(store.history.map((message) => message.id)).toEqual(['new'])
  event('history', { channelId: 'second', rev: 2, messages: [] })
  expect(store.history.map((message) => message.id)).toEqual(['new'])
  wrapper.unmount()
})
test('unmount during listener registration removes the listener', async () => {
  let resolve!: (value: () => void) => void
  const stop = vi.fn()
  listen.mockImplementation(
    () =>
      new Promise<() => void>((done) => {
        resolve = done
      }),
  )
  const { wrapper } = setup()
  wrapper.unmount()
  resolve(stop)
  await nextTick()
  expect(stop).toHaveBeenCalledOnce()
  expect(invoke).not.toHaveBeenCalled()
})

const withThread = threadFixture as unknown as {
  snapshot: SnapshotDto
  history: HistoryDto
  threadHistory: ThreadHistoryDto
}
const open = withThread.snapshot.openThread as OpenThreadDto
test('the open thread transcript follows its events and revisions', async () => {
  invoke.mockImplementation((name: string) =>
    Promise.resolve(
      name === 'get_snapshot'
        ? { ...snapshot, historyRev: 0 }
        : name === 'get_thread_history'
          ? withThread.threadHistory
          : { channelId: snapshot.channelId, rev: 0, messages: [] },
    ),
  )
  const { store, event, wrapper } = setup()
  await vi.waitFor(() => expect(store.snapshot).not.toBeNull())
  expect(store.threadHistory).toBeNull()
  expect(invoke).not.toHaveBeenCalledWith('get_thread_history')
  // The snapshot opens a thread. Its transcript arrives by fetch.
  event('snapshot', { ...snapshot, historyRev: 0, rev: 2, openThread: open })
  await vi.waitFor(() => expect(store.threadHistory?.channelId).toBe('thread'))
  expect(invoke.mock.calls.filter(([name]) => name === 'get_thread_history')).toHaveLength(1)
  expect(store.threadHistory?.messages).toHaveLength(2)
  // A newer snapshot revision keeps the old transcript until the event brings the new one.
  event('snapshot', { ...snapshot, historyRev: 0, rev: 3, openThread: { ...open, historyRev: 5 } })
  await nextTick()
  expect(store.threadHistory?.rev).toBe(4)
  event('thread-history', { ...withThread.threadHistory, rev: 5, messages: [] })
  expect(store.threadHistory?.rev).toBe(5)
  expect(store.threadHistory?.messages).toHaveLength(0)
  // A late older transcript never replaces a newer one.
  event('thread-history', withThread.threadHistory)
  expect(store.threadHistory?.rev).toBe(5)
  // A transcript of another thread is not shown.
  event('thread-history', { ...withThread.threadHistory, channelId: 'other', rev: 9 })
  expect(store.threadHistory?.channelId).toBe('thread')
  // Closing the thread drops the transcript.
  event('snapshot', { ...snapshot, historyRev: 0, rev: 4, openThread: null })
  await nextTick()
  expect(store.threadHistory).toBeNull()
  wrapper.unmount()
})

test('a thread transcript that arrives before its snapshot is kept', async () => {
  invoke.mockImplementation((name: string) =>
    Promise.resolve(
      name === 'get_snapshot'
        ? { ...snapshot, historyRev: 0 }
        : { channelId: '', rev: 0, messages: [] },
    ),
  )
  const { store, event, wrapper } = setup()
  await vi.waitFor(() => expect(store.snapshot).not.toBeNull())
  event('thread-history', withThread.threadHistory)
  expect(store.threadHistory).toBeNull()
  event('snapshot', { ...snapshot, historyRev: 0, rev: 2, openThread: open })
  await nextTick()
  expect(store.threadHistory?.rev).toBe(4)
  expect(invoke).not.toHaveBeenCalledWith('get_thread_history')
  wrapper.unmount()
})

const subAgent = subAgentFixture as unknown as {
  snapshot: SnapshotDto
  history: HistoryDto
  threadHistory: ThreadHistoryDto
  agentRequests: AgentRequest[]
}
test('live sub-agent chips and thread approvals follow the snapshot', async () => {
  invoke.mockImplementation((name: string) =>
    Promise.resolve(
      name === 'get_snapshot'
        ? { ...snapshot, historyRev: 0 }
        : name === 'get_agent_requests'
          ? []
          : { channelId: snapshot.channelId, rev: 0, messages: [] },
    ),
  )
  const { store, event, wrapper } = setup()
  await vi.waitFor(() => expect(store.snapshot).not.toBeNull())
  expect(store.liveSubAgentThreads).toEqual([])
  expect(store.threadApprovals).toEqual([])
  event('snapshot', { ...subAgent.snapshot, historyRev: 0, rev: 2, channelId: snapshot.channelId })
  await nextTick()
  expect(store.liveSubAgentThreads.map((chip) => [chip.name, chip.state])).toEqual([
    ['Find callers', 'running'],
    ['Read the docs', 'done'],
  ])
  expect(store.threadApprovals.map((approval) => approval.threadId)).toEqual(['find-callers'])
  // The turn ends: the chips leave the snapshot and the store drops them.
  event('snapshot', {
    ...subAgent.snapshot,
    historyRev: 0,
    rev: 3,
    channelId: snapshot.channelId,
    liveSubAgentThreads: [],
    threadApprovals: [],
  })
  await nextTick()
  expect(store.liveSubAgentThreads).toEqual([])
  expect(store.threadApprovals).toEqual([])
  // A stale snapshot changes nothing.
  event('snapshot', { ...subAgent.snapshot, historyRev: 0, rev: 1, channelId: snapshot.channelId })
  await nextTick()
  expect(store.liveSubAgentThreads).toEqual([])
  wrapper.unmount()
})

test('agent requests load for the selected channel and after its turn ends, and decisions reload them', async () => {
  const pending = subAgent.agentRequests[0] as AgentRequest
  let rows: AgentRequest[] = []
  invoke.mockImplementation((name: string, args?: Record<string, unknown>) => {
    if (name === 'get_snapshot')
      return Promise.resolve({ ...snapshot, historyRev: 0, turnRunning: true })
    if (name === 'get_agent_requests') return Promise.resolve(rows)
    if (name === 'decide_agent_request') {
      rows = [{ ...pending, state: args?.decision === 'add' ? 'added' : 'denied' }]
      return Promise.resolve(rows[0])
    }
    return Promise.resolve({ channelId: snapshot.channelId, rev: 0, messages: [] })
  })
  const { store, event, wrapper } = setup()
  await vi.waitFor(() => expect(store.snapshot).not.toBeNull())
  await vi.waitFor(() =>
    expect(invoke).toHaveBeenCalledWith('get_agent_requests', { channelId: snapshot.channelId }),
  )
  // The agent asks during its turn. The list loads when the turn ends.
  rows = [pending]
  event('snapshot', { ...snapshot, historyRev: 0, rev: 2, turnRunning: false })
  await vi.waitFor(() => expect(store.agentRequests.map((row) => row.id)).toEqual(['request']))
  expect(invoke.mock.calls.filter(([name]) => name === 'get_agent_requests')).toHaveLength(2)
  // A decision goes to the selected channel and the list loads again.
  const decided = await store.decideAgentRequest('request', 'deny')
  expect(decided.state).toBe('denied')
  expect(invoke).toHaveBeenCalledWith('decide_agent_request', {
    channelId: snapshot.channelId,
    requestId: 'request',
    decision: 'deny',
  })
  expect(store.agentRequests.map((row) => row.state)).toEqual(['denied'])
  // Another channel starts with an empty list.
  rows = []
  event('snapshot', { ...snapshot, historyRev: 0, rev: 3, channelId: 'second' })
  expect(store.agentRequests).toEqual([])
  await vi.waitFor(() =>
    expect(invoke).toHaveBeenCalledWith('get_agent_requests', { channelId: 'second' }),
  )
  // A decision that fails still reloads the list and rejects.
  invoke.mockImplementation((name: string) =>
    name === 'decide_agent_request'
      ? Promise.reject(new Error('Wait for the current response to finish.'))
      : Promise.resolve(name === 'get_agent_requests' ? [pending] : snapshot),
  )
  await expect(store.decideAgentRequest('request', 'add')).rejects.toThrow('Wait for')
  expect(store.agentRequests).toHaveLength(1)
  wrapper.unmount()
})
