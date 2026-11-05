import { expect, test, vi, beforeEach } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import { defineComponent, nextTick } from 'vue'
import snapshotFixture from './fixtures/snapshot.json'
import type { HistoryDto, SnapshotDto } from './types'
const invoke = vi.hoisted(() => vi.fn())
const listen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))
import { useStore } from './store'
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
