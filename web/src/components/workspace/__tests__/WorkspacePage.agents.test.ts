import { mount, flushPromises } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { getConversations, type ConversationEntry } from '@/bridge/api'
import { listen } from '@tauri-apps/api/event'
import WorkspacePage from '../WorkspacePage.vue'
import AgentAvatar from '../../agents/AgentAvatar.vue'
import fixture from '@/bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '@/bridge/types'

vi.mock('@/bridge/api', () => ({ getConversations: vi.fn(), getAgents: vi.fn(async () => []) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
const conversation: ConversationEntry = {
  id: 'chat',
  name: 'general',
  agentId: 'quinn',
  agent: 'Quinn',
  avatar: 'lucide:bot:#a78bfa',
  provider: 'codex',
  model: 'model',
  projectId: 'project',
  archived: false,
  agentArchived: false,
  deleted: false,
  inDmList: false,
  isDefault: false,
  workspaceMode: 'main_checkout',
}
const props = {
  page: 'agents' as const,
  snapshot: { ...fixture, projectId: 'project' } as SnapshotDto,
  messages: [],
}
beforeEach(() => vi.clearAllMocks())

test('project agent cards show each saved avatar once and keep profile navigation', async () => {
  vi.mocked(getConversations).mockResolvedValue([
    conversation,
    { ...conversation, id: 'another-chat' },
    { ...conversation, id: 'other-project', projectId: 'other', agentId: 'outside' },
    { ...conversation, id: 'jordan-chat', agentId: 'jordan', agent: 'Jordan', avatar: null },
  ])
  const view = mount(WorkspacePage, { props })
  try {
    await flushPromises()
    expect(view.findAllComponents(AgentAvatar)).toHaveLength(2)
    expect(view.get('[data-agent-icon="bot"]').attributes('data-agent-color')).toBe('#a78bfa')
    expect(view.findAllComponents(AgentAvatar)[1]!.text()).toBe('JO')
    const profile = view.findAll('button').find((button) => button.text() === 'Quinn')!
    await profile.trigger('click')
    expect(view.emitted('action')).toEqual([['view_agent_profile:quinn']])
  } finally {
    view.unmount()
  }
})

test('saved avatar changes refresh in the open project agents view', async () => {
  vi.mocked(getConversations).mockResolvedValue([conversation])
  const view = mount(WorkspacePage, { props })
  try {
    await flushPromises()
    vi.mocked(getConversations).mockResolvedValue([
      { ...conversation, avatar: 'lucide:rocket:#ff0000' },
    ])
    const handler = vi.mocked(listen).mock.calls.find(([name]) => name === 'discard://agents')![1]
    handler({ event: 'discard://agents', id: 1, payload: null })
    await flushPromises()
    expect(view.find('[data-agent-icon="bot"]').exists()).toBe(false)
    expect(view.get('[data-agent-icon="rocket"]').attributes('data-agent-color')).toBe('#ff0000')
  } finally {
    view.unmount()
  }
})
