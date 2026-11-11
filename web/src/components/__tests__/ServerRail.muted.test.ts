import { mount } from '@vue/test-utils'
import { expect, test } from 'vite-plus/test'
import ServerRail from '../ServerRail.vue'
import type { ChannelDto } from '../../bridge/types'

const live = (id: string, projectId: string, extra: Partial<ChannelDto> = {}): ChannelDto => ({
  id,
  projectId,
  agentId: 'agent',
  name: id,
  agent: 'Agent',
  provider: 'codex',
  workspaceMode: 'main_checkout',
  running: false,
  unread: false,
  pendingApprovals: 0,
  ...extra,
})
const projects = [
  { id: 'a', name: 'Alpha', path: '/a' },
  { id: 'b', name: 'Beta', path: '/b' },
]
const mountRail = (channels: ChannelDto[]) =>
  mount(ServerRail, {
    props: {
      projects,
      projectId: 'a',
      channels,
      project: 'Alpha',
      unread: false,
      pendingApprovals: 0,
    },
  })
const unread = (rail: ReturnType<typeof mountRail>, name: string) =>
  rail.get(`[aria-label="${name}"]`).attributes('aria-description') === 'Unread messages'

test('a muted channel does not mark its project on the rail, and an unmuted one does', () => {
  const rail = mountRail([
    live('muted', 'a', { unread: false, muted: true }),
    live('plain', 'b', { unread: true }),
  ])
  expect(unread(rail, 'Alpha')).toBe(false)
  expect(unread(rail, 'Beta')).toBe(true)
})

test('a muted channel is ignored even when a stale payload still says unread', () => {
  const rail = mountRail([live('muted', 'a', { unread: true, muted: true })])
  expect(unread(rail, 'Alpha')).toBe(false)
})

test('another unread channel still marks the project that has a muted channel', () => {
  const rail = mountRail([
    live('muted', 'a', { unread: false, muted: true }),
    live('other', 'a', { unread: true }),
  ])
  expect(unread(rail, 'Alpha')).toBe(true)
})

test('pending approvals of a muted channel still show on the rail', () => {
  const rail = mountRail([live('muted', 'a', { unread: false, muted: true, pendingApprovals: 2 })])
  expect(rail.get('[aria-label="Alpha"]').get('[data-testid="approval-badge"]').text()).toBe('2')
  expect(unread(rail, 'Alpha')).toBe(false)
})
