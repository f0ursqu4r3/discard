import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import WriteAccessApprovals from '../WriteAccessApprovals.vue'
import type { MoveDetails, WriteRequest } from '@/bridge/api'
const api = vi.hoisted(() => ({ getWriteRequests: vi.fn(), decideWriteRequest: vi.fn() }))
vi.mock('@/bridge/api', () => api)
afterEach(() => vi.clearAllMocks())
const pending = {
  id: 'r',
  channelId: 'c',
  agentId: 'a',
  kind: 'worktree_create',
  branch: 'feature',
  path: '/target',
  state: 'pending',
  result: null,
}
test('shows the branch and target folder', async () => {
  api.getWriteRequests.mockResolvedValue([pending])
  const view = mount(WriteAccessApprovals, { props: { channelId: 'c', running: false } })
  await flushPromises()
  expect(view.text()).toContain('feature')
  expect(view.text()).toContain('/target')
  expect(view.text()).toContain('Create a worktree for this branch?')
  view.unmount()
})
test('shows a home folder path with ~', async () => {
  api.getWriteRequests.mockResolvedValue([{ ...pending, path: '/Users/ada/.discard/worktrees/w1' }])
  const view = mount(WriteAccessApprovals, { props: { channelId: 'c', running: false } })
  await flushPromises()
  expect(view.text()).toContain('~/.discard/worktrees/w1')
  expect(view.text()).not.toContain('/Users/ada')
  view.unmount()
})
test('approve is disabled while the channel is running', async () => {
  api.getWriteRequests.mockResolvedValue([pending])
  const view = mount(WriteAccessApprovals, { props: { channelId: 'c', running: true } })
  await flushPromises()
  expect(
    view
      .findAll('button')
      .find((b) => b.text() === 'Create worktree')!
      .attributes('disabled'),
  ).toBeDefined()
  expect(view.text()).toContain('Wait for the current response to finish before approving.')
  view.unmount()
})
test('approve and deny call the command once', async () => {
  for (const approve of [true, false]) {
    api.getWriteRequests.mockResolvedValue([pending])
    api.decideWriteRequest.mockResolvedValue({
      ...pending,
      state: approve ? 'completed' : 'denied',
    })
    const view = mount(WriteAccessApprovals, { props: { channelId: 'c', running: false } })
    await flushPromises()
    const button = view
      .findAll('button')
      .find((b) => b.text() === (approve ? 'Create worktree' : 'Deny'))!
    await button.trigger('click')
    await flushPromises()
    expect(api.decideWriteRequest).toHaveBeenLastCalledWith('c', 'r', approve)
    expect(view.findAll('button')).toHaveLength(0)
    view.unmount()
  }
  expect(api.decideWriteRequest).toHaveBeenCalledTimes(2)
})
const moveDetails: MoveDetails = {
  target: { kind: 'channel', channelId: 'ch', name: 'feature' },
  handoff: { goal: 'Ship it', constraints: [], decisions: [], openQuestions: [] },
  sourceMessageId: 'm1',
}
const move: WriteRequest = {
  ...pending,
  kind: 'conversation_move',
  path: '',
  move: moveDetails,
}
const newBranchMove: WriteRequest = {
  ...move,
  move: {
    ...moveDetails,
    target: {
      kind: 'new_branch',
      repository: 'repo',
      repoPath: '/repo',
      branch: 'importer',
      base: 'main',
      baseCommit: '1a2b3c4d5e',
    },
  },
}
async function mountMove(request: WriteRequest, running = false) {
  api.getWriteRequests.mockResolvedValue([request])
  const view = mount(WriteAccessApprovals, { props: { channelId: 'c', running } })
  await flushPromises()
  return view
}
test('a move request shows the move card', async () => {
  const view = await mountMove(move)
  expect(view.text()).toContain('Move this conversation?')
  expect(view.findAll('button').map((b) => b.text())).toEqual(['Deny', 'Move'])
  expect(view.findAll('dt').map((d) => d.text())).not.toContain('Path')
  expect(view.get('[data-ui-id="write-access-approval"]').attributes('data-protected')).toBe('true')
  expect(view.text()).toContain('Approval applies once and expires after 10 minutes.')
  view.unmount()
})
test('a move to an existing channel explains what approval does', async () => {
  const view = await mountMove(move)
  expect(view.text()).toContain(
    'Approval posts this handoff and your latest message in #feature and starts a response there. This transcript is not copied. You stay in this channel.',
  )
  view.unmount()
})
test('a move to a new branch explains what approval creates', async () => {
  const view = await mountMove(newBranchMove)
  expect(view.text()).toContain(
    'Approval creates the branch, a worktree, and a channel. It posts this handoff and your latest message there and starts a response. This transcript is not copied. You stay in this channel.',
  )
  view.unmount()
})
test('move approval is disabled while the channel is running', async () => {
  const view = await mountMove(move, true)
  expect(
    view
      .findAll('button')
      .find((b) => b.text() === 'Move')!
      .attributes('disabled'),
  ).toBeDefined()
  expect(view.text()).toContain('Wait for the current response to finish before approving.')
  view.unmount()
})
test('move and deny call the command once', async () => {
  for (const approve of [true, false]) {
    api.decideWriteRequest.mockResolvedValue({ ...move, state: approve ? 'completed' : 'denied' })
    const view = await mountMove(move)
    await view
      .findAll('button')
      .find((b) => b.text() === (approve ? 'Move' : 'Deny'))!
      .trigger('click')
    await flushPromises()
    expect(api.decideWriteRequest).toHaveBeenLastCalledWith('c', 'r', approve)
    view.unmount()
  }
  expect(api.decideWriteRequest).toHaveBeenCalledTimes(2)
})
