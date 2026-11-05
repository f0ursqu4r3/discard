import { mount } from '@vue/test-utils'
import { describe, expect, test } from 'vitest'
import { serverInitials, serverPillHeight } from './layout'
import ChannelList from './ChannelList.vue'
import ServerRail from './ServerRail.vue'

describe('server rail', () => {
  test.each([
    [false, false, false, 0], [false, false, true, 2],
    [false, true, false, 4], [false, true, true, 4],
    [true, false, false, 8], [true, false, true, 8],
    [true, true, false, 8], [true, true, true, 8],
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

test('rail exposes project and review tooltip labels', () => {
  const rail = mount(ServerRail, { props: { project: 'My Project', unread: false, pendingApprovals: 0, reviewCount: 3 } })
  expect(rail.find('[aria-label="My Project"]').exists()).toBe(true)
  expect(rail.find('[aria-label="Review plugins (3)"]').exists()).toBe(true)
})

test('panel widths save only for the visible three panel layout', async () => {
  const { savePanelWidths } = await import('./layout')
  expect(savePanelWidths(false, 2)).toBe(false)
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
  expect(rail.find('svg.lucide-settings').exists()).toBe(true)
  expect(header.find('svg.lucide-panel-right').exists()).toBe(true)
})

test('saved widths use the resizable panels pixel values', async () => {
  const { panelWidths } = await import('./layout')
  expect(panelWidths([240, 60, 366])).toEqual([240, 366])
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
