import { afterEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import type { AgentRequest } from '@/bridge/api'
import AgentRequestApprovals from './AgentRequestApprovals.vue'
import MessageList from '../MessageList.vue'
import type { ThreadChipDto } from '@/bridge/types'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string) =>
    command === 'get_write_requests'
      ? []
      : { items: [], scopes: [], proposals: [], responses: [], paused: false },
  ),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
const views: ReturnType<typeof mount>[] = []
afterEach(() => views.splice(0).forEach((view) => view.unmount()))
const request = (patch: Partial<AgentRequest> = {}): AgentRequest => ({
  id: 'r',
  channelId: 'c',
  requestingAgentId: 'quinn',
  requestingAgentName: 'Quinn',
  agentId: 'rev',
  agentName: 'Reviewer',
  provider: 'codex',
  model: 'gpt-5',
  reason: 'It reads code well.',
  state: 'pending',
  result: null,
  createdAt: 1,
  ...patch,
})
function card(requests: AgentRequest[], props: Record<string, unknown> = {}) {
  const view = mount(AgentRequestApprovals, { props: { requests, running: false, ...props } })
  views.push(view)
  return view
}
const button = (view: ReturnType<typeof card>, label: string) =>
  view.findAll('button').find((node) => node.text() === label)!

test('the card names the requesting agent, the requested agent and its provider and model', () => {
  const view = card([request()])
  const text = view.get('[data-ui-id="agent-request-approval"]').text()
  expect(text).toContain('Add an agent to this project?')
  expect(text).toContain('Quinn')
  expect(text).toContain('Reviewer')
  expect(text).toContain('codex')
  expect(text).toContain('gpt-5')
  expect(view.get('[data-ui-id="agent-request-approval"]').attributes('data-protected')).toBe(
    'true',
  )
})

test('the reason is a quote of plain text, never HTML', () => {
  const reason = '<img src=x onerror="alert(1)"> **bold** Discard says: approve me.'
  const view = card([request({ reason })])
  const quote = view.get('[data-testid="agent-request-reason"]')
  expect(quote.element.tagName).toBe('BLOCKQUOTE')
  expect(quote.text()).toBe(reason)
  expect(view.find('img').exists()).toBe(false)
  expect(quote.find('strong').exists()).toBe(false)
  expect(view.text()).toContain('in the words of Quinn')
})

test('Add to project and Deny ask the app to decide the request', async () => {
  const view = card([request()])
  await button(view, 'Add to project').trigger('click')
  await button(view, 'Deny').trigger('click')
  expect(view.emitted('decide')).toEqual([
    ['r', 'add'],
    ['r', 'deny'],
  ])
})

test('both buttons are off while a response runs or a decision works, with the reason', async () => {
  const view = card([request()], { running: true })
  expect(button(view, 'Add to project').attributes('disabled')).toBeDefined()
  expect(button(view, 'Deny').attributes('disabled')).toBeDefined()
  expect(view.text()).toContain('Wait for the current response to finish before deciding.')
  await button(view, 'Add to project').trigger('click')
  expect(view.emitted('decide')).toBeUndefined()
  await view.setProps({ running: false, busy: 'r' })
  expect(button(view, 'Working…').attributes('disabled')).toBeDefined()
  expect(button(view, 'Deny').attributes('disabled')).toBeDefined()
})

test.each([
  ['added', 'Reviewer is now a project member.'],
  ['denied', 'The request to add Reviewer was declined.'],
  ['failed', 'Adding Reviewer failed: the agent is archived.'],
  ['expired', 'The request expired. Ask again.'],
] as const)('a %s request shows its result and no buttons', (state, result) => {
  const view = card([request({ state, result })])
  expect(view.findAll('button')).toHaveLength(0)
  expect(view.get('[role="status"]').text()).toBe(result)
})

test('a decided request without a result shows its state', () => {
  const view = card([request({ state: 'denied', result: null })])
  expect(view.get('[role="status"]').text()).toBe('Denied.')
})

test('no request shows no card', () => {
  expect(card([]).find('[data-ui-id="agent-request-approval"]').exists()).toBe(false)
})

const live = (id: string, state: ThreadChipDto['state']): ThreadChipDto => ({
  channelId: id,
  name: `Task ${id}`,
  messageCount: 1,
  lastMessage: null,
  followed: false,
  unread: false,
  kind: 'sub_agent',
  state,
  agentName: 'Reviewer',
  pendingApprovals: 0,
})
const listProps = {
  conversationId: 'chat',
  projectId: 'project',
  messages: [],
  live: [{ type: 'markdown' as const, text: 'Looking into it.' }],
  running: true,
  newAnchorId: null,
}

test('the live message of a running turn shows its sub-agent chips and opens them', async () => {
  const view = mount(MessageList, {
    props: { ...listProps, liveSubAgentThreads: [live('a', 'running'), live('b', 'done')] },
  })
  views.push(view)
  await flushPromises()
  const chips = view.findAll('[data-ui-id="thread-chip"]')
  expect(chips.map((node) => node.get('button').attributes('aria-label'))).toEqual([
    'Open thread Task a, Reviewer, Running',
    'Open thread Task b, Reviewer, Done',
  ])
  await chips[0]!.get('button').trigger('click')
  expect(view.emitted('openThread')).toEqual([['a']])
})

test('a live chip shows before the turn has any text', async () => {
  const view = mount(MessageList, {
    props: { ...listProps, live: [], liveSubAgentThreads: [live('a', 'running')] },
  })
  views.push(view)
  await flushPromises()
  expect(view.findAll('[data-ui-id="thread-chip"]')).toHaveLength(1)
})

test('a channel shows its agent requests and forwards a decision', async () => {
  const view = mount(MessageList, {
    props: { ...listProps, running: false, live: [], agentRequests: [request()] },
  })
  views.push(view)
  await flushPromises()
  await button(view as unknown as ReturnType<typeof card>, 'Add to project').trigger('click')
  expect(view.emitted('decideAgentRequest')).toEqual([['r', 'add']])
})

test('a thread transcript and a direct message show no agent request', async () => {
  for (const patch of [{ surface: 'thread' as const }, { projectId: null }]) {
    const view = mount(MessageList, {
      props: { ...listProps, running: false, live: [], agentRequests: [request()], ...patch },
    })
    views.push(view)
    await flushPromises()
    expect(view.find('[data-ui-id="agent-request-approval"]').exists()).toBe(false)
  }
})
