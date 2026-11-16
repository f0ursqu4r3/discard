import { expect, test, vi, beforeEach } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import snapshotFixture from '../fixtures/snapshot.json'
const invoke = vi.hoisted(() => vi.fn())
const listen = vi.hoisted(() => vi.fn())
const currentWindow = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: currentWindow }))
import { currentWindowLabel } from '../window'
import { useStore } from '../store'
beforeEach(() => {
  vi.clearAllMocks()
  listen.mockResolvedValue(() => {})
  invoke.mockImplementation((name: string) =>
    Promise.resolve(
      name === 'get_snapshot'
        ? snapshotFixture
        : { channelId: snapshotFixture.channelId, rev: 0, messages: [] },
    ),
  )
})
test('the label is the label of the current window', () => {
  currentWindow.mockReturnValue({ label: 'project-p1' })
  expect(currentWindowLabel()).toBe('project-p1')
})
test('a page outside a Tauri window is the main window', () => {
  currentWindow.mockImplementation(() => {
    throw new TypeError('no window')
  })
  expect(currentWindowLabel()).toBe('main')
})
test('a project window listens for the events of its own view', async () => {
  currentWindow.mockReturnValue({ label: 'project-p1' })
  const wrapper = mount(
    defineComponent({
      setup() {
        useStore()
        return () => null
      },
    }),
  )
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('get_snapshot'))
  const names = listen.mock.calls.map(([name]) => name)
  expect(names).toEqual(['discard://snapshot', 'discard://history', 'discard://thread-history'])
  for (const [, , options] of listen.mock.calls)
    expect(options).toEqual({ target: { kind: 'WebviewWindow', label: 'project-p1' } })
  wrapper.unmount()
})
