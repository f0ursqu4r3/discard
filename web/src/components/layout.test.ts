import { mount } from '@vue/test-utils'
import { describe, expect, test } from 'vitest'
import { serverInitials, serverPillHeight } from './layout'
import ChannelList from './ChannelList.vue'
import ServerRail from './ServerRail.vue'
import UserBar from './workspace/UserBar.vue'

describe('server rail', () => {
  test.each([
    [false, false, false, 0], [false, false, true, 2],
    [false, true, false, 6], [false, true, true, 6],
    [true, false, false, 12], [true, false, true, 12],
    [true, true, false, 12], [true, true, true, 12],
  ])('pill height for selected %s hovered %s unread %s', (selected, hovered, unread, height) => {
    expect(serverPillHeight(selected, hovered, unread)).toBe(height)
  })
  test.each([['discard.ui-juice', 'DU'], ['My Project', 'MP'], ['discard', 'DI'], ['x', 'X'], ['  ..  ', ''], ['', '']])('initials for %s', (name, expected) => {
    expect(serverInitials(name)).toBe(expected)
  })
  test('only shows the approval badge when approvals are pending', () => {
    const rail = mount(ServerRail, { props: { project: 'Discard', unread: false, pendingApprovals: 0, reviewCount: 0 } })
    expect(rail.find('[data-testid="approval-badge"]').exists()).toBe(false)
    return rail.setProps({ pendingApprovals: 2 }).then(() => expect(rail.get('[data-testid="approval-badge"]').text()).toBe('2'))
  })
})

test('branch rows have depth and emit the selected key', async () => {
  const list = mount(ChannelList, { props: { branches: [{ key: 'a', label: 'main', kind: 'main', depth: 2, current: true, collapsed: false }, { key: 'b', label: 'branch', kind: 'branch', depth: 3, current: false, collapsed: false }] } })
  const row = list.get('[data-depth="2"]')
  await row.trigger('click')
  expect(list.emitted('toggleBranch')?.[0]).toEqual(['a'])
})

test('rail and shared user bar expose project and review labels', () => {
  const rail = mount(ServerRail, { props: { project: 'My Project', unread: false, pendingApprovals: 0, reviewCount: 3 } })
  expect(rail.find('[aria-label="My Project"]').exists()).toBe(true)
  expect(mount(UserBar, { props: { reviewCount: 3 } }).find('[aria-label="Review plugins (3)"]').exists()).toBe(true)
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
  const rail = mount(ServerRail, { props: { project: 'Project', unread: false, pendingApprovals: 0, reviewCount: 0 } })
  const header = mount(ChannelHeader, { props: { channel: 'general', memberListVisible: true } })
  expect(rail.find('svg.lucide-messages-square').exists()).toBe(true)
  expect(mount(UserBar, { props: { reviewCount: 0 } }).find('svg.lucide-settings').exists()).toBe(true)
  expect(header.find('svg.lucide-panel-right').exists()).toBe(true)
})

test('saved widths exclude the project rail', async () => {
  const { panelWidths } = await import('./layout')
  expect(panelWidths([312, 600, 366])).toEqual([240, 366])
})

test('a branch row without children has no collapse control', async () => {
  const list = mount(ChannelList, { props: { branches: [{ key: 'a', label: 'main', kind: 'main', depth: 0, current: true, collapsed: false }] } })
  const row = list.get('[data-depth="0"]')
  expect(row.attributes('aria-expanded')).toBeUndefined()
  await row.trigger('click')
  expect(list.emitted('toggleBranch')).toBeUndefined()
})

test('member status shows its label and presence indicator', async () => {
  const { default: MemberList } = await import('./MemberList.vue')
  const member = mount(MemberList, { props: { agent: 'Agent', status: 'awaiting_approval', provider: 'claude' } })
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

test('DM and project rail entries emit separate navigation events', async () => {
  const rail = mount(ServerRail, { props: { project: 'Discard', unread: true, pendingApprovals: 0, directMessages: true } })
  await rail.get('[aria-label="Direct messages"]').trigger('click')
  expect(rail.emitted('directMessages')).toHaveLength(1)
  expect(rail.emitted('project')).toBeUndefined()
  await rail.get('[aria-label="Discard"]').trigger('click')
  expect(rail.emitted('project')).toHaveLength(1)
})
