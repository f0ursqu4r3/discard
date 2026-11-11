import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
import WorkspacePage from '../WorkspacePage.vue'
import ContextPanel from '../ContextPanel.vue'
import UserBar from '../UserBar.vue'
import NavigationPanel from '../NavigationPanel.vue'
import DirectMessageSidebar from '../DirectMessageSidebar.vue'
import AgentsNavigation from '../AgentsNavigation.vue'
import ServerRail from '../../ServerRail.vue'
import snapshotFixture from '@/bridge/fixtures/snapshot.json'
import type { MessageDto, SnapshotDto } from '@/bridge/types'
vi.mock('@/bridge/api', () => ({
  runAction: vi.fn(),
  contextMenu: vi.fn(() => []),
  getConversations: vi.fn(async () => []),
  getAgents: vi.fn(async () => []),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
const snapshot = snapshotFixture as SnapshotDto
test.each([null, 'another-project'])(
  'project navigation shows branches with active project %s',
  (activeProjectId) => {
    const selected: SnapshotDto = {
      ...snapshot,
      activeProjectId,
      channelId: '',
      projectPath: '/Users/example/git/discard',
      projectDisplayPath: '~/git/discard',
      repositories: [{ id: '', label: 'discard', path: '/Users/example/git/discard' }],
      branches: [
        {
          key: 'main',
          label: 'main',
          kind: 'main',
          depth: 0,
          current: false,
          collapsed: false,
          hasChildren: true,
        },
        {
          key: 'feature',
          label: 'feature',
          kind: 'branch',
          depth: 1,
          current: false,
          collapsed: false,
          hasChildren: false,
        },
      ],
    }
    const view = mount(NavigationPanel, { props: { page: 'agents', snapshot: selected } })
    expect(view.get('[data-ui-id="project-header"]').text()).toContain('~/git/discard')
    expect(view.get('[data-ui-id="project-header"]').text()).not.toContain('/Users/example')
    expect(view.find('[title="/Users/example/git/discard"]').exists()).toBe(true)
    expect(view.findAll('[data-ui-id^="branches-row-"]')).toHaveLength(2)
    expect(view.findAll('[data-ui-id^="worktrees-row-"]')).toHaveLength(1)
    expect(view.get('[data-ui-id="branches-row-main"]').attributes('disabled')).toBeUndefined()

    const overview = mount(WorkspacePage, {
      props: { page: 'overview', snapshot: selected, messages: [] },
    })
    const branchCard = overview
      .findAll('[data-slot="card"]')
      .find((card) => card.text().includes('Branches'))
    expect(branchCard?.text()).toContain('2')
    expect(overview.text()).toContain('~/git/discard')
  },
)

test.each([null, 'project'])(
  'DM navigation excludes selected-project data with active project %s',
  async (activeProjectId) => {
    const view = mount(NavigationPanel, {
      props: { page: 'direct-messages', snapshot: { ...snapshot, activeProjectId } },
    })
    expect(view.findComponent(DirectMessageSidebar).exists()).toBe(true)
    expect(view.find('[data-ui-id="channel-panel"]').exists()).toBe(false)
    for (const id of [
      'project-header',
      'project-general-channel',
      'project-navigation',
      'branches-list',
      'worktrees-list',
      'integrations-navigation',
    ]) {
      expect(view.find(`[data-ui-id="${id}"]`).exists()).toBe(false)
    }
    expect(view.text()).toContain('Direct Messages')
    const agents = view.getComponent(AgentsNavigation)
    expect(agents.props('projectId')).toBeNull()
    expect(agents.props('channelId')).toBe(snapshot.channelId)
    agents.vm.$emit('select', 'dm')
    expect(view.emitted('selectChannel')).toEqual([['dm']])
    await agents.get('button').trigger('click')
    expect(view.emitted('action')).toEqual([['show_friends']])

    await view.setProps({ busy: true })
    expect(agents.props('busy')).toBe(true)
    await view.setProps({ page: 'overview' })
    expect(view.findComponent(DirectMessageSidebar).exists()).toBe(false)
    expect(view.find('[data-ui-id="project-navigation"]').exists()).toBe(true)
    view.unmount()
  },
)

test.each([false, true])(
  'background replies show a pill on their own project in compact layout %s',
  async (compact) => {
    const channel = {
      id: 'background',
      name: 'general',
      agentId: 'agent',
      agent: 'Quinn',
      provider: 'codex',
      workspaceMode: 'main_checkout',
      projectId: 'away',
      running: false,
      unread: true,
      pendingApprovals: 1,
    }
    const selected = { ...snapshot, projectId: 'current', channels: [channel] }
    const view = mount(NavigationPanel, {
      props: {
        page: 'overview',
        compact,
        snapshot: selected,
        projects: [
          { id: 'away', name: 'Away', path: '/away' },
          { id: 'current', name: 'Current', path: '/current' },
          { id: 'quiet', name: 'Quiet', path: '/quiet' },
        ],
      },
    })
    const rail = view.getComponent(ServerRail)
    expect(rail.get('[data-ui-id="project-selection-pill-away"]').attributes('style')).toContain(
      '* 2)',
    )
    expect(rail.get('[data-ui-id="project-selection-pill-quiet"]').attributes('style')).toContain(
      '* 0)',
    )
    expect(rail.get('[aria-label="Away"]').attributes('aria-description')).toBe('Unread messages')
    expect(rail.get('[aria-label="Away"]').get('[data-testid="approval-badge"]').text()).toBe('1')
    expect(rail.get('[aria-label="Current"]').find('[data-testid="approval-badge"]').exists()).toBe(
      false,
    )

    await view.setProps({
      snapshot: { ...selected, channels: [{ ...channel, unread: false, pendingApprovals: 0 }] },
    })
    expect(rail.get('[data-ui-id="project-selection-pill-away"]').attributes('style')).toContain(
      '* 0)',
    )
    expect(rail.get('[aria-label="Away"]').attributes('aria-description')).toBeUndefined()

    await view.setProps({
      snapshot: { ...selected, channels: [{ ...channel, projectId: null, pendingApprovals: 0 }] },
    })
    expect(rail.get('[data-ui-id="dm-selection-pill"]').attributes('style')).toContain('* 2)')
    expect(rail.get('[data-ui-id="project-selection-pill-away"]').attributes('style')).toContain(
      '* 0)',
    )
    expect(rail.get('[aria-label="Direct messages"]').attributes('aria-description')).toBe(
      'Unread messages',
    )
    view.unmount()
  },
)

const messages: MessageDto[] = [
  {
    id: 'project-message',
    role: 'user',
    blocks: [{ type: 'markdown', text: 'Private project conversation' }],
    continuesGroup: false,
  },
]

test('DM view cannot display or send into the project conversation', async () => {
  const view = mount(WorkspacePage, { props: { page: 'direct-messages', snapshot, messages } })
  expect(view.text()).not.toContain('Private project conversation')
  expect(view.find('textarea').exists()).toBe(false)
  expect(view.text()).toContain('Agents')
  await view.get('button:not([role="tab"])').trigger('click')
  expect(view.emitted('action')).toEqual([['add_agent']])
})

test('project context frames the plugin member list as a card', () => {
  const withSlot: SnapshotDto = {
    ...snapshot,
    slots: {
      ...snapshot.slots,
      member_list: {
        plugin: 'default-ui',
        element: { tag: 'text', props: { variant: 'label' }, children: [{ Text: 'Agents' }] },
      },
    },
  }
  const view = mount(ContextPanel, { props: { snapshot: withSlot } })
  const slot = view.get('[data-ui-id="context-plugin-slot"]')
  expect(slot.attributes('data-slot')).toBe('card')
  expect(slot.find('[data-slot="card-content"]').text()).toBe('Agents')
  expect(view.find('[data-ui-id="project-members"]').exists()).toBe(false)
})

test('DM profile hides project-specific branch and plugin content', () => {
  const view = mount(ContextPanel, { props: { snapshot, directMessage: true } })
  expect(view.find('[data-ui-id="branch-context"]').exists()).toBe(false)
  expect(view.find('[data-ui-id="context-plugin-slot"]').exists()).toBe(false)
  expect(view.text()).toContain(snapshot.agent)
  expect(view.text()).toContain('Profile')
  const planned = view.findAll('[role="tab"]').find((tab) => tab.text().includes('Projects'))!
  expect(planned.attributes('disabled')).toBeDefined()
  expect(planned.text()).toContain('Soon')
})

test('shared user bar opens application settings when reviews are pending', async () => {
  const view = mount(UserBar, { props: { reviewCount: 3 } })
  await view.get('[aria-label="Application settings"]').trigger('click')
  expect(view.emitted('settings')).toHaveLength(1)
})
