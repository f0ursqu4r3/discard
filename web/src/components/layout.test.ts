import { mount } from '@vue/test-utils'
import { describe, expect, test } from 'vite-plus/test'
import { serverInitials, serverPillHeight } from './layout'
import ChannelList from './ChannelList.vue'
import ServerRail from './ServerRail.vue'
import UserBar from './workspace/UserBar.vue'

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
      projectPath: '/channel-test',
      channelId: 'first',
      branches: [],
      channels: [
        {
          id: 'first',
          name: 'general',
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

test('planned DM entry is disabled while project navigation remains available', async () => {
  const rail = mount(ServerRail, {
    props: { project: 'Discard', unread: true, pendingApprovals: 0, directMessages: true },
  })
  const dm = rail.get('[data-ui-id="direct-messages-button"]')
  expect(dm.attributes('disabled')).toBeDefined()
  await dm.trigger('click')
  expect(rail.emitted('directMessages')).toBeUndefined()
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
