import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { getAgents, searchAgentHistory } from '@/bridge/api'
import AgentSummary from '../AgentSummary.vue'
import AgentHistorySearch from '../AgentHistorySearch.vue'
import ChannelHeader from '../../ChannelHeader.vue'
vi.mock('@/bridge/api', () => ({ getAgents: vi.fn(), searchAgentHistory: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
let view: ReturnType<typeof mount>
beforeEach(() => vi.clearAllMocks())
afterEach(() => view?.unmount())
test('A toggles the short profile; B submits a history query for the current agent', async () => {
  view = mount(ChannelHeader, {
    props: {
      channel: 'Robin',
      agentName: 'Robin',
      agentId: 'a',
      directMessage: true,
      memberListVisible: false,
    },
  })
  await view.get('[aria-label="Show short profile"]').trigger('click')
  expect(view.emitted('toggleMembers')).toHaveLength(1)
  expect(view.text()).not.toContain('View profile')
  await view.get('input').setValue('  release notes  ')
  await view.get('form').trigger('submit')
  expect(view.emitted('search')).toEqual([['release notes']])
  expect(
    view.get('[data-slot="input-group"] [data-align="inline-end"] button').attributes('type'),
  ).toBe('submit')
  expect(view.get('[data-slot="input-group"]').classes()).toContain('h-8')
  await view.setProps({ agentId: 'b' })
  expect((view.get('input').element as HTMLInputElement).value).toBe('')
})
test('short profile shows identity and personality with an explicit full-profile action', async () => {
  vi.mocked(getAgents).mockResolvedValue([
    {
      id: 'a',
      name: 'Robin',
      avatar: 'avatar.png',
      provider: 'codex',
      model: 'exact',
      systemPrompt: 'Keep answers clear.',
      archived: false,
      policy: 'deny',
      pluginWrite: false,
      workspaceWrite: false,
      sandboxAutorun: false,
      sandboxAutorunUnavailable: null,
    },
  ])
  view = mount(AgentSummary, { props: { agentId: 'a', name: 'Robin', status: 'idle' } })
  await flushPromises()
  expect(view.text()).toContain('Keep answers clear.')
  expect(view.text()).toContain('exact')
  expect(view.text()).toContain('Idle')
  expect(view.find('form').exists()).toBe(false)
  await view
    .findAll('button')
    .find((b) => b.text() === 'View full profile')!
    .trigger('click')
  expect(view.emitted('profile')).toEqual([['a']])
})
test('search renders saved matches and selects the exact message', async () => {
  const match = {
    channelId: 'old-dm',
    channelName: 'Earlier chat',
    messageId: 'm1',
    role: 'user',
    preview: 'Release notes',
  }
  vi.mocked(searchAgentHistory).mockResolvedValue({ results: [match], hasMore: true })
  view = mount(AgentHistorySearch, {
    props: { agentId: 'a', agentName: 'Robin', query: 'release', revision: 1 },
  })
  await flushPromises()
  expect(searchAgentHistory).toHaveBeenCalledWith('a', 'release')
  expect(view.text()).toContain('newest 50')
  expect(view.text()).toContain('Earlier chat')
  await view.get('button').trigger('click')
  expect(view.emitted('select')).toEqual([[match]])
})
test('an older search response cannot replace results for a new query', async () => {
  let resolveOld!: (result: { results: []; hasMore: false }) => void
  vi.mocked(searchAgentHistory)
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveOld = resolve
        }),
    )
    .mockResolvedValueOnce({
      results: [
        {
          channelId: 'new',
          channelName: 'New',
          messageId: 'new',
          role: 'agent',
          preview: 'Current result',
        },
      ],
      hasMore: false,
    })
  view = mount(AgentHistorySearch, {
    props: { agentId: 'a', agentName: 'Robin', query: 'old', revision: 1 },
  })
  await view.setProps({ query: 'current', revision: 2 })
  await flushPromises()
  resolveOld({ results: [], hasMore: false })
  await flushPromises()
  expect(view.text()).toContain('Current result')
})
test('failed history searches show a retry action', async () => {
  vi.mocked(searchAgentHistory)
    .mockRejectedValueOnce('Read failed')
    .mockResolvedValueOnce({ results: [], hasMore: false })
  view = mount(AgentHistorySearch, {
    props: { agentId: 'a', agentName: 'Robin', query: 'query', revision: 1 },
  })
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toContain('Read failed')
  await view.get('button').trigger('click')
  await flushPromises()
  expect(view.text()).toContain('No saved messages match')
})
