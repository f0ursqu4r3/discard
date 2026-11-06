import { mount, shallowMount } from '@vue/test-utils'
import { describe, expect, test } from 'vite-plus/test'
import { serverInitials, serverPillHeight } from './layout'
import ChannelList from './ChannelList.vue'
import ServerRail from './ServerRail.vue'
import UserBar from './workspace/UserBar.vue'
import type { WorkspacePage } from './workspace/types'
import archivedChannelsFixture from '../bridge/fixtures/project-channels.json'
import type { ProjectChannelDto } from '../bridge/types'

describe('server rail', () => {
  test.each([
    [false, false, false, 0],
    [false, false, true, 2],
    [false, true, false, 6],
    [false, true, true, 6],
    [true, false, false, 12],
    [true, false, true, 12],
    [true, true, false, 12],
    [true, true, true, 12],
  ])('pill height for selected %s hovered %s unread %s', (selected, hovered, unread, height) => {
    expect(serverPillHeight(selected, hovered, unread)).toBe(height)
  })
  test.each([
    ['discard.ui-juice', 'DU'],
    ['My Project', 'MP'],
    ['discard', 'DI'],
    ['x', 'X'],
    ['  ..  ', ''],
    ['', ''],
  ])('initials for %s', (name, expected) => {
    expect(serverInitials(name)).toBe(expected)
  })
  test('only shows the approval badge when approvals are pending', () => {
    const rail = mount(ServerRail, {
      props: { project: 'Discard', unread: false, pendingApprovals: 0, reviewCount: 0 },
    })
    expect(rail.find('[data-testid="approval-badge"]').exists()).toBe(false)
    return rail
      .setProps({ pendingApprovals: 2 })
      .then(() => expect(rail.get('[data-testid="approval-badge"]').text()).toBe('2'))
  })
})

test('Git branches and worktrees populate separate lists and open their branch', async () => {
  const list = mount(ChannelList, {
    props: {
      projectId: 'project',
      activeProjectId: 'project',
      branches: [
        {
          key: 'main',
          label: 'main',
          kind: 'main',
          depth: 0,
          current: true,
          collapsed: true,
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
    },
  })
  expect(list.findAll('[data-ui-id^="branches-row-"]')).toHaveLength(2)
  expect(list.findAll('[data-ui-id^="worktrees-row-"]')).toHaveLength(1)
  await list.get('[data-ui-id="branches-row-feature"]').trigger('click')
  expect(list.emitted('openBranch')?.[0]).toEqual(['feature'])
  await list.get('[data-ui-id="worktrees-row-main"]').trigger('click')
  expect(list.emitted('openBranch')?.[1]).toEqual(['main'])
})

test('saved conversations stay behind Git navigation while approvals remain accessible', async () => {
  const list = mount(ChannelList, {
    props: {
      projectId: 'project',
      activeProjectId: 'project',
      projectPath: '/channel-test',
      channelId: 'first',
      branches: [],
      channels: [
        {
          id: 'first',
          name: 'general',
          agentId: 'a',
          projectId: 'project',
          agent: 'Builder',
          provider: 'codex',
          workspaceMode: 'main_checkout',
          running: false,
          unread: false,
          pendingApprovals: 0,
        },
        {
          id: 'second',
          name: 'tests',
          agentId: 'a',
          projectId: 'project',
          agent: 'Builder',
          provider: 'codex',
          workspaceMode: 'worktree',
          running: true,
          unread: true,
          pendingApprovals: 2,
        },
      ],
    },
  })
  expect(list.find('[data-ui-id="channel-list"]').exists()).toBe(false)
  expect(list.find('[data-ui-id="branches-toggle"]').exists()).toBe(true)
  expect(list.find('[data-ui-id="worktrees-toggle"]').exists()).toBe(true)
  expect(list.get('[data-ui-id="nav-approvals"]').text()).toContain('2')
  await list.get('[data-ui-id="nav-approvals"]').trigger('click')
  expect(list.emitted('approvals')).toHaveLength(1)
})

test('rail and shared user bar expose project and review labels', () => {
  const rail = mount(ServerRail, {
    props: { project: 'My Project', unread: false, pendingApprovals: 0, reviewCount: 3 },
  })
  expect(rail.find('[aria-label="My Project"]').exists()).toBe(true)
  expect(
    mount(UserBar, { props: { reviewCount: 3 } })
      .find('[aria-label="Application settings"]')
      .exists(),
  ).toBe(true)
})

test('panel widths save only for a complete two or three panel layout', async () => {
  const { savePanelWidths } = await import('./layout')
  expect(savePanelWidths(false, 2)).toBe(true)
  expect(savePanelWidths(false, 3)).toBe(false)
  expect(savePanelWidths(true, 2)).toBe(false)
  expect(savePanelWidths(true, 3)).toBe(true)
})
test('settings targets review when reviews are pending', async () => {
  const { settingsTarget } = await import('./layout')
  expect(settingsTarget(0)).toEqual(['open_settings', 'Settings'])
  expect(settingsTarget(3)).toEqual(['review_plugins', 'Review plugins (3)'])
})

test('bundled chrome icons render for settings and the member list', async () => {
  const { default: ChannelHeader } = await import('./ChannelHeader.vue')
  const rail = mount(ServerRail, {
    props: { project: 'Project', unread: false, pendingApprovals: 0, reviewCount: 0 },
  })
  const header = mount(ChannelHeader, { props: { channel: 'general', memberListVisible: true } })
  expect(rail.find('svg.lucide-messages-square').exists()).toBe(true)
  expect(
    mount(UserBar, { props: { reviewCount: 0 } })
      .find('svg.lucide-settings')
      .exists(),
  ).toBe(true)
  expect(header.find('svg.lucide-panel-right').exists()).toBe(true)
})

test('saved widths exclude the project rail', async () => {
  const { panelWidths } = await import('./layout')
  expect(panelWidths([312, 600, 366])).toEqual([240, 366])
})

test('a branch row without children has no collapse control', async () => {
  const list = mount(ChannelList, {
    props: {
      projectId: 'project',
      activeProjectId: 'project',
      branches: [
        {
          key: 'a',
          label: 'main',
          kind: 'main',
          depth: 0,
          current: true,
          collapsed: false,
          hasChildren: true,
        },
      ],
    },
  })
  const row = list.get('[data-ui-id="branches-row-a"]')
  expect(row.attributes('aria-expanded')).toBeUndefined()
  await row.trigger('click')
  expect(list.emitted('toggleBranch')).toBeUndefined()
})

test('member status shows its label and presence indicator', async () => {
  const { default: MemberList } = await import('./MemberList.vue')
  const member = mount(MemberList, {
    props: { agent: 'Agent', status: 'awaiting_approval', provider: 'claude' },
  })
  expect(member.text()).toContain('Waiting for approval · claude')
  expect(member.find('[data-testid="presence-dot"]').classes()).toContain('bg-warning')
})

test('navigation size conversion survives reload and scaled rail widths', async () => {
  const { navigationWidth, panelWidths } = await import('./layout')
  for (const rail of [72, 90, 108]) {
    for (const width of [180, 240, 320, 400]) {
      expect(panelWidths([navigationWidth(width, rail), 600], rail, 366)).toEqual([width, 366])
    }
  }
})

test('DM entry opens contacts while project navigation remains available', async () => {
  const rail = mount(ServerRail, {
    props: { project: 'Discard', unread: true, pendingApprovals: 0, directMessages: true },
  })
  const dm = rail.get('[data-ui-id="direct-messages-button"]')
  expect(rail.get('[data-ui-id="dm-selection-pill"]').attributes('style')).toContain('* 12)')
  expect(rail.find('[data-ui-id="project-selection-pill"]').exists()).toBe(false)
  await rail.setProps({ directMessages: false })
  expect(rail.get('[data-ui-id="dm-selection-pill"]').attributes('style')).toContain('* 0)')
  await rail.setProps({ directMessages: true })

  expect(dm.attributes('disabled')).toBeUndefined()
  await dm.trigger('click')
  expect(rail.emitted('directMessages')).toHaveLength(1)
  expect(rail.emitted('project')).toBeUndefined()
  await rail.get('[aria-label="Discard"]').trigger('click')
  expect(rail.emitted('project')).toHaveLength(1)
})

test('project rail keeps separate entries and selects by project ID', async () => {
  const rail = mount(ServerRail, {
    props: {
      project: 'First',
      projectId: 'first',
      unread: false,
      pendingApprovals: 0,
      projects: [
        { id: 'first', name: 'First', path: '/first' },
        { id: 'second', name: 'Second', path: '/second' },
      ],
    },
  })
  expect(rail.get('[aria-label="First"]').attributes('aria-current')).toBe('page')
  await rail.get('[aria-label="Second"]').trigger('click')
  expect(rail.emitted('project')).toEqual([['second']])
  await rail.get('[aria-label="Import project"]').trigger('click')
  expect(rail.emitted('addProject')).toHaveLength(1)
  expect(rail.findAll('[data-ui-id="project-selection-pill"]')).toHaveLength(1)
})

test('project general channel stays available when the active conversation has no project', async () => {
  const list = mount(ChannelList, {
    props: {
      page: 'overview',
      projectId: 'selected-project',
      activeProjectId: null,
      branches: [],
      projectChannels: [
        {
          id: 'general-id',
          projectId: 'selected-project',
          name: 'general',
          agentId: 'agent',
          workspaceMode: 'main_checkout',
          kind: 'general',
          branch: null,
          archived: false,
          archivedInto: null,
        },
      ],
    },
  })
  const general = list.get('[data-ui-id="project-general-channel"]')
  expect(general.attributes('disabled')).toBeUndefined()
  await general.trigger('click')
  expect(list.emitted('selectChannel')).toEqual([['general-id']])
})

test('branch rows can open selected-project channels when the active conversation has no project', async () => {
  const list = mount(ChannelList, {
    props: {
      page: 'overview',
      projectId: 'selected-project',
      activeProjectId: null,
      branches: [
        {
          key: 'feature/x',
          label: 'feature/x',
          kind: 'branch',
          depth: 0,
          current: false,
          collapsed: false,
          hasChildren: false,
        },
      ],
    },
  })
  await list.get('[data-ui-id="branches-row-feature/x"]').trigger('click')
  expect(list.emitted('openBranch')).toEqual([['feature/x']])
})

test('saved channel history opens by its durable channel id', async () => {
  const list = mount(ChannelList, {
    props: {
      projectId: 'project',
      activeProjectId: 'project',
      branches: [],
      projectChannels: archivedChannelsFixture as ProjectChannelDto[],
    },
  })
  expect(list.text()).toContain('Saved channels')
  await list.get('[data-ui-id="saved-channel-toggle"]').trigger('click')
  await list.get('[data-ui-id="saved-channel-legacy-id"]').trigger('click')
  expect(list.emitted('selectChannel')).toEqual([['legacy-id']])
})

test('saved branch opens by id when live Git rows omit it', async () => {
  const list = mount(ChannelList, {
    props: {
      projectId: 'project',
      activeProjectId: 'project',
      projectPath: '/test/saved-branch-missing-git',
      branches: [],
      projectChannels: [
        {
          id: 'saved-branch-id',
          projectId: 'project',
          name: 'Feature x',
          agentId: 'agent',
          workspaceMode: 'main_checkout',
          kind: 'branch',
          branch: 'feature/x',
          archived: false,
          archivedInto: null,
        },
      ],
    },
  })
  await list.get('[data-ui-id="saved-channel-toggle"]').trigger('click')
  await list.get('[data-ui-id="saved-channel-saved-branch-id"]').trigger('click')
  expect(list.emitted('selectChannel')).toEqual([['saved-branch-id']])
  expect(list.emitted('openBranch')).toBeUndefined()
})

test('archived branch history stays reachable when Git still lists the branch', async () => {
  const list = mount(ChannelList, {
    props: {
      projectId: 'project',
      activeProjectId: 'project',
      projectPath: '/test/saved-branch-still-listed',
      branches: [
        {
          key: 'feature/x',
          label: 'feature/x',
          kind: 'branch',
          depth: 0,
          current: false,
          collapsed: false,
          hasChildren: false,
        },
      ],
      projectChannels: [
        {
          id: 'archived-branch-id',
          projectId: 'project',
          name: 'Feature x',
          agentId: 'agent',
          workspaceMode: 'main_checkout',
          kind: 'branch',
          branch: 'feature/x',
          archived: true,
          archivedInto: null,
        },
      ],
    },
  })
  await list.get('[data-ui-id="saved-channel-toggle"]').trigger('click')
  await list.get('[data-ui-id="saved-channel-archived-branch-id"]').trigger('click')
  expect(list.emitted('selectChannel')).toEqual([['archived-branch-id']])
})

test('project conversation creation stays visible, disabled, and marked Soon', async () => {
  const list = mount(ChannelList, {
    attachTo: document.body,
    props: { projectId: 'project', activeProjectId: 'project', branches: [] },
  })
  await list.get('[data-ui-id="project-header"]').trigger('click')
  const item = [...document.body.querySelectorAll('[role="menuitem"]')].find((entry) =>
    entry.textContent?.includes('New conversation'),
  )
  expect(item?.textContent).toContain('New conversation')
  expect(item?.textContent).toContain('Soon')
  expect(item?.getAttribute('aria-disabled')).toBe('true')
  expect(list.emitted('createChannel')).toBeUndefined()
  list.unmount()
})

test.each<WorkspacePage>([
  'conversation',
  'overview',
  'agents',
  'memory',
  'artifacts',
  'project-settings',
])('project %s navigation omits the agent/conversation list', (page) => {
  const nav = shallowMount(ChannelList, {
    props: { page, projectId: 'project', activeProjectId: 'project', branches: [] },
    global: { renderStubDefaultSlot: true },
  })
  expect(nav.findComponent({ name: 'AgentsNavigation' }).exists()).toBe(false)
  expect(nav.find('[data-ui-id="project-header"]').exists()).toBe(true)
  expect(nav.find('[data-ui-id="project-navigation"]').exists()).toBe(true)
  nav.unmount()
})
