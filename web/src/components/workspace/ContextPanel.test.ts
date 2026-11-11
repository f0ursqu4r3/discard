import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
import ContextPanel from './ContextPanel.vue'
import MemberList from '../MemberList.vue'
import fixture from '@/bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '@/bridge/types'
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
test('context values have lowercase keys and dotted leaders without row icons', () => {
  const snapshot: SnapshotDto = {
    ...(fixture as SnapshotDto),
    activeProjectId: null,
    branches: [
      {
        key: 'main',
        label: 'main',
        kind: 'branch',
        depth: 0,
        current: true,
        collapsed: false,
        hasChildren: false,
      },
    ],
  }
  const view = mount(ContextPanel, { props: { snapshot } })
  const row = view.findAll('dt').find((key) => key.text() === 'branch')
  expect(row).toBeDefined()
  expect(row!.element.parentElement!.querySelector('.kv-leader')).not.toBeNull()
  expect(row!.element.parentElement!.querySelector('svg')).toBeNull()
  view.unmount()
})

test('the working member identifies the active responder', () => {
  const snapshot: SnapshotDto = {
    ...(fixture as SnapshotDto),
    activeProjectId: null,
    turnRunning: true,
    responder: {
      agentId: 'mentioned',
      name: 'Reviewer',
      provider: 'claude',
      avatar: 'lucide:bot:ansi-5',
    },
  }
  const view = mount(ContextPanel, { props: { snapshot } })
  expect(view.getComponent(MemberList).props()).toMatchObject({
    agentId: 'mentioned',
    agent: 'Reviewer',
    avatar: 'lucide:bot:ansi-5',
    status: 'working',
  })
  view.unmount()
})

test('shows Autorun beside the agent status', () => {
  const snapshot: SnapshotDto = {
    ...(fixture as SnapshotDto),
    activeProjectId: null,
    autorun: { state: 'on', reason: null },
  }
  const view = mount(ContextPanel, { props: { snapshot } })
  expect(view.text()).toContain('Autorun')
  expect(view.text()).not.toContain('Autorun off')
  view.unmount()
})
test('shows why autorun is off', () => {
  const snapshot: SnapshotDto = {
    ...(fixture as SnapshotDto),
    activeProjectId: null,
    autorun: { state: 'unavailable', reason: 'r' },
  }
  const view = mount(ContextPanel, { props: { snapshot } })
  expect(view.text()).toContain('Autorun off: r')
  view.unmount()
})
test('shows no autorun text when autorun is not configured', () => {
  const snapshot: SnapshotDto = { ...(fixture as SnapshotDto), activeProjectId: null }
  const view = mount(ContextPanel, { props: { snapshot } })
  expect(view.text()).not.toContain('Autorun')
  view.unmount()
})
