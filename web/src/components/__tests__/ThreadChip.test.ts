import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import type { ThreadChipDto, ThreadState } from '../../bridge/types'
import ThreadChip from '../ThreadChip.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
const views: ReturnType<typeof mount>[] = []
beforeEach(() => vi.mocked(invoke).mockReset().mockResolvedValue(null))
afterEach(() => views.splice(0).forEach((view) => view.unmount()))
const chip = (patch: Partial<ThreadChipDto> = {}): ThreadChipDto => ({
  channelId: 'sub',
  name: 'Find callers',
  messageCount: 2,
  lastMessage: { role: 'agent', authorName: 'Reviewer', preview: 'Callers: a, b.' },
  followed: true,
  unread: false,
  kind: 'sub_agent',
  state: 'done',
  agentName: 'Reviewer',
  pendingApprovals: 0,
  ...patch,
})
function mountChip(thread: ThreadChipDto) {
  const view = mount(ThreadChip, { props: { thread } })
  views.push(view)
  return view
}

const labels: Record<ThreadState, string> = {
  running: 'Running',
  done: 'Done',
  failed: 'Failed',
  cancelled: 'Cancelled',
  timed_out: 'Timed out',
}
test('a sub-agent chip shows the title, the agent name and the state', () => {
  const view = mountChip(chip())
  const element = view.get('[data-ui-id="thread-chip"]')
  expect(element.attributes('data-kind')).toBe('sub_agent')
  expect(element.text()).toContain('Find callers')
  expect(element.get('[data-ui-id="thread-agent"]').text()).toBe('Reviewer')
  expect(element.get('[data-ui-id="thread-state"]').text()).toBe('Done')
  expect(element.text()).not.toContain('messages')
})

test.each(Object.keys(labels) as ThreadState[])(
  'the %s state has its own label and style',
  (state) => {
    const view = mountChip(chip({ state }))
    const mark = view.get('[data-ui-id="thread-state"]')
    expect(mark.text()).toContain(labels[state])
    expect(mark.attributes('data-state')).toBe(state)
    expect(view.get('button').attributes('aria-label')).toBe(
      `Open thread Find callers, Reviewer, ${labels[state]}`,
    )
  },
)

test('every state class differs from the others and uses a theme token', () => {
  const classes = (Object.keys(labels) as ThreadState[]).map((state) => {
    const view = mountChip(chip({ state }))
    return view.get('[data-ui-id="thread-state"]').classes().join(' ')
  })
  // Running and cancelled share the muted tone. Their marks differ by label and icon.
  expect(new Set(classes).size).toBe(4)
  expect(classes.join(' ')).toMatch(/text-success/)
  expect(classes.join(' ')).toMatch(/text-destructive/)
  expect(classes.join(' ')).toMatch(/text-warning/)
})

test('a running chip shows the app spinner and a finished one shows an icon', () => {
  const running = mountChip(chip({ state: 'running' }))
  expect(running.get('[data-ui-id="thread-state"] [role="img"]').attributes('aria-hidden')).toBe(
    'true',
  )
  const failed = mountChip(chip({ state: 'failed' }))
  expect(failed.find('[data-ui-id="thread-state"] svg').exists()).toBe(true)
  expect(failed.find('[data-ui-id="thread-state"] [role="img"]').exists()).toBe(false)
})

test('selecting a running sub-agent chip opens its thread', async () => {
  const view = mountChip(chip({ state: 'running' }))
  await view.get('button').trigger('click')
  expect(view.emitted('open')).toEqual([['sub']])
})

test('a pending approval and unread messages show on the chip', () => {
  const view = mountChip(chip({ pendingApprovals: 2, unread: true }))
  expect(view.get('[aria-label="2 pending approvals"]').text()).toBe('2')
  expect(view.find('[aria-label="Unread messages"]').exists()).toBe(true)
  expect(mountChip(chip()).find('[data-ui-id="thread-pending-approvals"]').exists()).toBe(false)
})

test('a sub-agent chip that is not followed offers Follow', async () => {
  const view = mountChip(chip({ followed: false }))
  await view.get('[aria-label="Follow thread Find callers"]').trigger('click')
  expect(invoke).toHaveBeenCalledWith('follow_thread', { threadId: 'sub' })
  expect(view.emitted('open')).toBeUndefined()
})

test('a question chip keeps its message count and has no state', () => {
  const view = mountChip(
    chip({ kind: 'question', state: null, agentName: 'Quinn', name: 'Why a thread?' }),
  )
  expect(view.text()).toContain('2 messages')
  expect(view.find('[data-ui-id="thread-state"]').exists()).toBe(false)
  expect(view.find('[data-ui-id="thread-agent"]').exists()).toBe(false)
  expect(view.get('button').attributes('aria-label')).toBe('Open thread Why a thread?')
})
