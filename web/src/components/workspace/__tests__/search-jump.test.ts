import { afterEach, expect, test, vi } from 'vite-plus/test'
import { mount, flushPromises } from '@vue/test-utils'
import MessageList from '../MessageList.vue'
const calls = vi.hoisted(() => ({ jump: vi.fn() }))
vi.mock('../ui/message-scroller/useMessageScroller', async (original) => {
  const actual = await original<typeof import('../ui/message-scroller/useMessageScroller')>()
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
afterEach(() => {
  view?.unmount()
  calls.jump.mockClear()
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
  await vi.waitFor(() =>
    expect(calls.jump).toHaveBeenCalledWith('older', { align: 'center', behavior: 'auto' }),
  )
  expect(view.get('[data-search-match="true"]').attributes('data-message-id')).toBe('older')
  calls.jump.mockClear()
  await view.setProps({ jumpTo: { messageId: 'older', request: 2 } })
  await vi.waitFor(() => expect(calls.jump).toHaveBeenCalledOnce())
})
