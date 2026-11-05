import { expect, test, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, nextTick } from 'vue'
import snapshot from './fixtures/snapshot.json'
const invoke = vi.hoisted(() => vi.fn())
const listen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))
import { useStore } from './store'

test('loads the snapshot and follows snapshot events', async () => {
  invoke.mockImplementation((name: string) => Promise.resolve(name === 'get_snapshot' ? snapshot : []))
  listen.mockResolvedValue(() => {})
  let store: ReturnType<typeof useStore>
  mount(defineComponent({ setup() { store = useStore(); return () => null } }))
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('get_snapshot'))
  expect(invoke.mock.calls.filter(([name]) => name === 'get_snapshot')).toHaveLength(1)
  const callback = listen.mock.calls.find(([name]) => name === 'discard://snapshot')?.[1]
  callback({ payload: { ...snapshot, project: 'Updated' } })
  await nextTick()
  expect(store!.snapshot?.project).toBe('Updated')
})
