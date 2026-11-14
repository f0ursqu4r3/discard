import { afterEach, expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import type { PromptDto } from '../../bridge/types'
import ApprovalDock from '../ApprovalDock.vue'

vi.mock('../../bridge/api', () => ({ decideApproval: vi.fn() }))
const views: ReturnType<typeof mount>[] = []
afterEach(() => views.splice(0).forEach((view) => view.unmount()))
const prompt = (approvalId: string, view = 'pending', summary = approvalId): PromptDto => ({
  kind: 'provider_tool',
  alwaysAllow: 'unavailable',
  approvalId,
  tool: 'Bash',
  summary,
  view,
})
function dock(prompts: PromptDto[]) {
  const view = mount(ApprovalDock, { props: { channelId: 'channel', prompts } })
  views.push(view)
  return view
}

test('renders nothing when no prompt is pending', () => {
  expect(dock([]).find('[data-ui-id="approval-dock"]').exists()).toBe(false)
  expect(dock([prompt('a', 'approved'), prompt('b', 'stale')]).html()).toBe('<!--v-if-->')
})

test('shows one pending prompt with no count', () => {
  const view = dock([prompt('a', 'approved'), prompt('b')])
  expect(view.findAll('[data-ui-id="approval-card"]')).toHaveLength(1)
  expect(view.text()).toContain('b')
  expect(view.find('[data-ui-id="approval-dock-count"]').exists()).toBe(false)
})

test('shows the first pending prompt and a count when several wait', () => {
  const view = dock([
    prompt('done', 'denied', 'done'),
    prompt('a', 'pending', 'first'),
    prompt('b', 'pending', 'second'),
  ])
  expect(view.findAll('[data-ui-id="approval-card"]')).toHaveLength(1)
  expect(view.get('[data-ui-id="approval-card"]').text()).toContain('first')
  expect(view.text()).not.toContain('second')
  expect(view.get('[data-ui-id="approval-dock-count"]').text()).toBe('1 of 2')
})

test('moves to the next prompt when the first is answered', async () => {
  const view = dock([prompt('a', 'pending', 'first'), prompt('b', 'pending', 'second')])
  await view.setProps({
    prompts: [prompt('a', 'approved', 'first'), prompt('b', 'pending', 'second')],
  })
  expect(view.get('[data-ui-id="approval-card"]').text()).toContain('second')
  expect(view.find('[data-ui-id="approval-dock-count"]').exists()).toBe(false)
})
