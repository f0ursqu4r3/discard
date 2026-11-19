import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
import ContextPanel from '../ContextPanel.vue'
import MemberList from '../../MemberList.vue'
import fixture from '@/bridge/fixtures/snapshot.json'
import type { ProjectChannelDto, SnapshotDto } from '@/bridge/types'
import ProjectChannelDetails from '../ProjectChannelDetails.vue'
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

test('shows when the response resumed the provider session', () => {
  const snapshot: SnapshotDto = {
    ...(fixture as SnapshotDto),
    activeProjectId: null,
    resumeState: { kind: 'resumed', agent: 'Reviewer' },
  }
  const view = mount(ContextPanel, { props: { snapshot } })
  expect(view.text()).toContain('Resumed session')
  expect(view.text()).not.toContain('Fresh replay')
  view.unmount()
})
test('shows why the response replayed the history', () => {
  const snapshot: SnapshotDto = {
    ...(fixture as SnapshotDto),
    activeProjectId: null,
    resumeState: { kind: 'replay', reason: 'the model changed.', agent: 'Reviewer' },
  }
  const view = mount(ContextPanel, { props: { snapshot } })
  expect(view.text()).toContain('Fresh replay: the model changed.')
  expect(view.text()).not.toContain('Resumed session')
  view.unmount()
})
test('shows no resume text when the response has no resume state', () => {
  const snapshot: SnapshotDto = { ...(fixture as SnapshotDto), activeProjectId: null }
  const view = mount(ContextPanel, { props: { snapshot } })
  expect(view.text()).not.toContain('Resumed session')
  expect(view.text()).not.toContain('Fresh replay')
  view.unmount()
})

test('a running text channel blocks a main agent change like a running general channel', () => {
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
  const snapshotWith = (running: string[]): SnapshotDto => ({
    ...(fixture as SnapshotDto),
    activeProjectId: 'project',
    channelId: 'general',
    channels: projectChannels.map((entry) => ({
      id: entry.id,
      projectId: 'project',
      agentId: 'agent',
      name: entry.name,
      agent: 'Agent',
      provider: 'codex',
      workspaceMode: 'main_checkout',
      running: running.includes(entry.id),
      unread: false,
      pendingApprovals: 0,
    })),
  })
  const generalRunning = (running: string[]) => {
    const view = mount(ContextPanel, {
      props: { snapshot: snapshotWith(running), projectChannels },
      global: { stubs: { ProjectChannelDetails: true } },
    })
    const value = view.findComponent(ProjectChannelDetails).props('generalRunning')
    view.unmount()
    return value
  }
  expect(generalRunning([])).toBe(false)
  expect(generalRunning(['general'])).toBe(true)
  expect(generalRunning(['plans'])).toBe(true)
  expect(generalRunning(['old', 'feature'])).toBe(false)
})
