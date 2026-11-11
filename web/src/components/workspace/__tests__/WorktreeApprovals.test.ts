import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import WorktreeApprovals from './WorktreeApprovals.vue'
const api = vi.hoisted(() => ({ getWorktreeRequests: vi.fn(), decideWorktreeRemoval: vi.fn() }))
vi.mock('@/bridge/api', () => api)
afterEach(() => {
  vi.clearAllMocks()
})
const pending = {
  id: 'request',
  channelId: 'channel',
  agentId: 'agent',
  state: 'pending',
  result: null,
  inspection: {
    path: '/repo/worktree',
    head: 'abc123',
    recoveryBranch: 'discard/retained-worktrees/abc123',
  },
}
test('displays the exact target and preservation plan; only a click approves removal', async () => {
  api.getWorktreeRequests.mockResolvedValue([pending])
  api.decideWorktreeRemoval.mockResolvedValue({
    ...pending,
    state: 'completed',
    result: 'Removed worktree /repo/worktree.',
  })
  const view = mount(WorktreeApprovals, { props: { channelId: 'channel', running: false } })
  await flushPromises()
  expect(view.text()).toContain('/repo/worktree')
  expect(view.text()).toContain('discard/retained-worktrees/abc123')
  expect(api.decideWorktreeRemoval).not.toHaveBeenCalled()
  await view
    .findAll('button')
    .find((button) => button.text() === 'Approve removal')!
    .trigger('click')
  await flushPromises()
  expect(api.decideWorktreeRemoval).toHaveBeenCalledWith('channel', 'request', true)
  expect(view.text()).toContain('Removed worktree /repo/worktree.')
  expect(view.findAll('button')).toHaveLength(0)
  view.unmount()
})
test('denial uses the scoped request and a failed action remains visible', async () => {
  api.getWorktreeRequests.mockResolvedValue([pending])
  api.decideWorktreeRemoval.mockRejectedValue(new Error('Worktree changed'))
  const view = mount(WorktreeApprovals, { props: { channelId: 'channel', running: false } })
  await flushPromises()
  await view
    .findAll('button')
    .find((button) => button.text() === 'Deny')!
    .trigger('click')
  await flushPromises()
  expect(api.decideWorktreeRemoval).toHaveBeenCalledWith('channel', 'request', false)
  expect(view.get('[role="alert"]').text()).toContain('Worktree changed')
  view.unmount()
})
