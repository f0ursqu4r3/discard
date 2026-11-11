import { mount } from '@vue/test-utils'
import { afterEach, expect, test } from 'vite-plus/test'
import StatusLine from '../StatusLine.vue'

const views: ReturnType<typeof mount>[] = []
afterEach(() => views.splice(0).forEach((view) => view.unmount()))
function status(overrides = {}) {
  const view = mount(StatusLine, {
    props: {
      project: 'discard',
      branch: 'feat/x',
      workingAgents: [{ id: 'builder', name: 'Builder', avatar: 'icon:bot:ansi-4' }],
      pendingApprovals: 2,
      ...overrides,
    },
  })
  views.push(view)
  return view
}
test('shows project, branch, working agents, and approvals', () => {
  const view = status()
  expect(view.text()).toContain('discard')
  expect(view.text()).toContain('⎇ feat/x')
  expect(view.text()).toContain('builder')
  expect(view.find('[aria-label="Working"]').exists()).toBe(true)
  expect(view.text()).toContain('▲ 2 approvals')
})
test('hides the branch when null and approvals at zero', () => {
  const view = status({ branch: null, pendingApprovals: 0, workingAgents: [] })
  expect(view.text()).not.toMatch(/[⎇▲]/)
  expect(view.find('[aria-label="Working"]').exists()).toBe(false)
})
test('uses the singular for one approval', () => {
  expect(status({ pendingApprovals: 1 }).text()).toContain('▲ 1 approval')
  expect(status({ pendingApprovals: 1 }).text()).not.toContain('approvals')
})
test('has no buttons or links', () => {
  expect(status().findAll('button,a')).toHaveLength(0)
})
