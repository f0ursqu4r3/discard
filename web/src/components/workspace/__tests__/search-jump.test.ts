import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { mount, flushPromises } from '@vue/test-utils'
import MessageList from '../../MessageList.vue'
const calls = vi.hoisted(() => ({ jump: vi.fn() }))
vi.mock('../../ui/message-scroller/useMessageScroller', async (original) => {
  const actual = await original<typeof import('../../ui/message-scroller/useMessageScroller')>()
  return {
    ...actual,
    useMessageScrollerContext: () => {
      const context = actual.useMessageScrollerContext()
      return {
        ...context,
        scrollToMessage: (...args: Parameters<typeof context.scrollToMessage>) => {
          calls.jump(...args)
          return context.scrollToMessage(...args)
        },
      }
    },
  }
})
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => ({
    items: [],
    scopes: [],
    proposals: [],
    responses: [],
    paused: false,
  })),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
let view: ReturnType<typeof mount>
// Wait on real animation frames. vi.waitFor would advance the fake timers and eat into the highlight time.
const nextFrame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
})
afterEach(() => {
  view?.unmount()
  calls.jump.mockClear()
  vi.useRealTimers()
})
test('a history result scrolls to and highlights its message, including a repeated selection', async () => {
  view = mount(MessageList, {
    props: {
      conversationId: 'chat',
      messages: [
        { id: 'older', role: 'user', blocks: [], continuesGroup: false },
        { id: 'newer', role: 'agent', blocks: [], continuesGroup: false },
      ],
      live: [],
      running: false,
      newAnchorId: null,
      jumpTo: { messageId: 'older', request: 1 },
    },
  })
  await flushPromises()
  await nextFrame()
  expect(calls.jump).toHaveBeenCalledWith('older', { align: 'center', behavior: 'auto' })
  expect(view.get('[data-search-match="true"]').attributes('data-message-id')).toBe('older')
  expect(view.get('[data-message-id="older"]').classes()).toContain('bg-accent')
  await vi.advanceTimersByTimeAsync(1499)
  expect(view.find('[data-search-match="true"]').exists()).toBe(true)
  await vi.advanceTimersByTimeAsync(1)
  expect(view.find('[data-search-match="true"]').exists()).toBe(false)
  expect(view.get('[data-message-id="older"]').classes()).not.toContain('bg-accent')
  calls.jump.mockClear()
  await view.setProps({ jumpTo: { messageId: 'older', request: 2 } })
  await nextFrame()
  expect(calls.jump).toHaveBeenCalledOnce()
  expect(view.get('[data-search-match="true"]').attributes('data-message-id')).toBe('older')
  await vi.advanceTimersByTimeAsync(1000)
  await view.setProps({ jumpTo: { messageId: 'older', request: 3 } })
  await vi.advanceTimersByTimeAsync(1000)
  expect(view.find('[data-search-match="true"]').exists()).toBe(true)
  await vi.advanceTimersByTimeAsync(500)
  expect(view.find('[data-search-match="true"]').exists()).toBe(false)
})
