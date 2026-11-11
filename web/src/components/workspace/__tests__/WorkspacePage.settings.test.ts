import { mount, flushPromises } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vite-plus/test'
import WorkspacePage from '../WorkspacePage.vue'
import ProjectChannelListSettings from '../ProjectChannelListSettings.vue'
import fixture from '@/bridge/fixtures/snapshot.json'
import ProjectIdentitySettings from '../ProjectIdentitySettings.vue'
import type { ProjectChannelDto, SnapshotDto } from '@/bridge/types'

vi.mock('@/bridge/api', () => ({
  getConversations: vi.fn(async () => []),
  getAgents: vi.fn(async () => []),
  getAlwaysAllowRules: vi.fn(async () => []),
  setProjectShowArchived: vi.fn(async () => undefined),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
const project = { id: 'project', name: 'Discard', path: '/project', showArchived: true }
const props = {
  page: 'project-settings' as const,
  snapshot: { ...fixture, projectId: 'project' } as SnapshotDto,
  messages: [],
  project,
}
beforeEach(() => vi.clearAllMocks())

test('the Project settings page has a Channel list card with the Archived switch', async () => {
  const view = mount(WorkspacePage, { props, global: { stubs: { ProjectAlwaysAllowRules: true } } })
  await flushPromises()
  const card = view.getComponent(ProjectChannelListSettings)
  expect(card.props('project')).toEqual(project)
  expect(card.text()).toContain('Channel list')
  expect(card.get('#project-show-archived').attributes('aria-checked')).toBe('true')
  await card.get('#project-show-archived').trigger('click')
  await flushPromises()
  expect(view.emitted('projectChanged')).toHaveLength(1)
  view.unmount()
})

test('a running text channel disables the primary agent selector like a running general channel', async () => {
  const channel = (id: string, kind: 'general' | 'text' | 'branch', archived = false) =>
    ({
      id,
      projectId: 'project',
      name: id,
      agentId: 'agent',
      workspaceMode: 'main_checkout',
      kind,
      branch: null,
      archived,
      archivedInto: null,
      hasMessages: false,
    }) as ProjectChannelDto
  const projectChannels = [
    channel('general', 'general'),
    channel('plans', 'text'),
    channel('old', 'text', true),
    channel('feature', 'branch'),
  ]
  const running = async (ids: string[]) => {
    const snapshot = {
      ...props.snapshot,
      channels: projectChannels.map((entry) => ({
        id: entry.id,
        projectId: 'project',
        agentId: 'agent',
        name: entry.name,
        agent: 'Agent',
        provider: 'codex',
        workspaceMode: 'main_checkout',
        running: ids.includes(entry.id),
        unread: false,
        pendingApprovals: 0,
      })),
    } as SnapshotDto
    const view = mount(WorkspacePage, {
      props: { ...props, snapshot, projectChannels },
      global: { stubs: { ProjectAlwaysAllowRules: true } },
    })
    await flushPromises()
    const value = view.getComponent(ProjectIdentitySettings).props('generalRunning')
    view.unmount()
    return value
  }
  expect(await running([])).toBe(false)
  expect(await running(['general'])).toBe(true)
  expect(await running(['plans'])).toBe(true)
  expect(await running(['old', 'feature'])).toBe(false)
})
