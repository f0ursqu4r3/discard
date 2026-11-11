import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { toast } from 'vue-sonner'
import type { ThreadChipDto } from '../bridge/types'
import ThreadsPopover from './ThreadsPopover.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
const views: ReturnType<typeof mount>[] = []
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
beforeEach(() => {
  vi.stubGlobal('ResizeObserver', ResizeObserverStub)
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockResolvedValue(null)
  vi.mocked(toast.error).mockReset()
})
afterEach(() => {
  vi.unstubAllGlobals()
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
})
const chip = (id: string, patch: Partial<ThreadChipDto> = {}): ThreadChipDto => ({
  channelId: id,
  name: `Thread ${id}`,
  messageCount: 2,
  lastMessage: null,
  followed: true,
  unread: false,
  kind: 'question',
  state: null,
  agentName: 'Quinn',
  pendingApprovals: 0,
  ...patch,
})
async function open(threads: ThreadChipDto[]) {
  const view = mount(ThreadsPopover, { attachTo: document.body, props: { threads } })
  views.push(view)
  await view.get('button[aria-label="Threads"]').trigger('click')
  await flushPromises()
  return view
}
const rows = () => [...document.querySelectorAll<HTMLElement>('[data-ui-id="threads-list-row"]')]
const clean = (node: Element | null) => node?.textContent?.replace(/\s+/g, ' ').trim()

test('an empty channel says that there is no thread yet', async () => {
  await open([])
  expect(rows()).toHaveLength(0)
  expect(clean(document.querySelector('[data-ui-id="threads-list"]'))).toContain('No threads yet.')
})

test('every thread shows, followed or not, in the order the app gives', async () => {
  await open([chip('newest'), chip('quiet', { followed: false, messageCount: 1 }), chip('older')])
  expect(rows().map((row) => row.dataset.threadId)).toEqual(['newest', 'quiet', 'older'])
  expect(clean(rows()[1]!)).toContain('Thread quiet')
  expect(clean(rows()[1]!)).toContain('1 message')
  expect(clean(rows()[0]!)).toContain('2 messages')
})

test('a row shows the kind, the sub-agent state and the unread mark', async () => {
  await open([
    chip('q', { unread: true }),
    chip('s', { kind: 'sub_agent', state: 'timed_out', messageCount: 3 }),
  ])
  const [question, sub] = rows()
  expect(question!.querySelector('[data-ui-id="threads-list-kind"]')!.textContent!.trim()).toBe(
    'Question',
  )
  expect(question!.querySelector('[aria-label="Unread messages"]')).not.toBeNull()
  expect(question!.querySelector('[data-ui-id="thread-state"]')).toBeNull()
  expect(sub!.querySelector('[data-ui-id="threads-list-kind"]')!.textContent!.trim()).toBe(
    'Sub-agent',
  )
  expect(sub!.querySelector('[data-ui-id="thread-state"]')!.getAttribute('data-state')).toBe(
    'timed_out',
  )
  expect(clean(sub!)).toContain('Timed out')
  expect(clean(sub!)).toContain('3 messages')
  expect(sub!.querySelector('[aria-label="Unread messages"]')).toBeNull()
})

test('a click on a row opens the thread and closes the list', async () => {
  const view = await open([chip('a'), chip('b')])
  document.querySelector<HTMLElement>('button[aria-label="Open thread Thread b"]')!.click()
  await flushPromises()
  expect(view.emitted('open')).toEqual([['b']])
  expect(document.querySelector('[data-ui-id="threads-list"]')).toBeNull()
})

test('Follow and Leave change the follow state without opening the thread', async () => {
  const view = await open([chip('on'), chip('off', { followed: false })])
  document.querySelector<HTMLElement>('button[aria-label="Leave thread Thread on"]')!.click()
  document.querySelector<HTMLElement>('button[aria-label="Follow thread Thread off"]')!.click()
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('leave_thread', { threadId: 'on' })
  expect(invoke).toHaveBeenCalledWith('follow_thread', { threadId: 'off' })
  expect(view.emitted('open')).toBeUndefined()
  expect(document.querySelector('[data-ui-id="threads-list"]')).not.toBeNull()
})

test('a failed follow shows a toast', async () => {
  await open([chip('off', { followed: false })])
  vi.mocked(invoke).mockRejectedValueOnce('The thread is gone.')
  document.querySelector<HTMLElement>('button[aria-label="Follow thread Thread off"]')!.click()
  await flushPromises()
  expect(toast.error).toHaveBeenCalledWith('Could not follow thread', {
    description: 'The thread is gone.',
  })
})

test('Escape closes the list', async () => {
  await open([chip('a')])
  document
    .querySelector('[data-ui-id="threads-list"]')!
    .dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
  await flushPromises()
  expect(document.querySelector('[data-ui-id="threads-list"]')).toBeNull()
})
