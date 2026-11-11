import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import type { ChannelDto, ProjectChannelDto, ThreadLinkDto } from '../bridge/types'
import ChannelList from './ChannelList.vue'
import ThreadRow from './ThreadRow.vue'
import AgentsNavigation from './workspace/AgentsNavigation.vue'
import { ContextMenuItem } from './ui/context-menu'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
const views: ReturnType<typeof mount>[] = []
afterEach(() => {
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
  localStorage.clear()
})
const link = (id: string, patch: Partial<ThreadLinkDto> = {}): ThreadLinkDto => ({
  id,
  parentChannelId: 'general',
  name: `Thread ${id}`,
  unread: false,
  running: false,
  pendingApprovals: 0,
  ...patch,
})
function track<T extends ReturnType<typeof mount>>(view: T) {
  views.push(view)
  return view
}

test('a thread row shows unread, running and pending approvals and selects on click', async () => {
  const view = track(
    mount(ThreadRow, {
      props: { link: link('t', { unread: true, running: true, pendingApprovals: 2 }) },
    }),
  )
  expect(view.text()).toContain('Thread t')
  expect(view.find('[aria-label="Unread messages"]').exists()).toBe(true)
  expect(view.find('[aria-label="Working"]').exists()).toBe(true)
  expect(view.get('[aria-label="2 pending approvals"]').text()).toBe('2')
  await view.get('button').trigger('click')
  expect(view.emitted('select')).toEqual([['t']])
})

test('a quiet thread row has no state marks and the open thread is highlighted', async () => {
  const view = track(mount(ThreadRow, { props: { link: link('t') } }))
  expect(view.find('[aria-label="Unread messages"]').exists()).toBe(false)
  expect(view.find('[aria-label="Working"]').exists()).toBe(false)
  expect(view.get('button').attributes('aria-current')).toBeUndefined()
  await view.setProps({ active: true })
  expect(view.get('button').attributes('aria-current')).toBe('page')
  expect(view.get('button').classes()).toContain('nav-active-edge')
})

test('the context menu leaves the thread and lists Rename and Delete as planned items', async () => {
  const view = track(mount(ThreadRow, { attachTo: document.body, props: { link: link('t') } }))
  await view.get('button').trigger('contextmenu', { button: 2, clientX: 20, clientY: 20 })
  await flushPromises()
  const items = view.findAllComponents(ContextMenuItem)
  expect(items.map((item) => item.text().replace(/\s+/g, ' '))).toEqual([
    'Open thread',
    'Leave thread',
    'Rename thread Soon',
    'Delete thread Soon',
  ])
  for (const item of items.slice(2)) {
    expect(item.attributes('aria-disabled')).toBe('true')
    item.vm.$emit('select')
  }
  expect(view.emitted('leave')).toBeUndefined()
  items[1]!.vm.$emit('select')
  expect(view.emitted('leave')).toEqual([['t']])
  items[0]!.vm.$emit('select')
  expect(view.emitted('select')).toEqual([['t']])
})

test('the keyboard opens the thread row menu', async () => {
  const view = track(mount(ThreadRow, { attachTo: document.body, props: { link: link('t') } }))
  await view.get('button').trigger('keydown', { key: 'F10', shiftKey: true })
  await flushPromises()
  expect(document.querySelector('[data-ui-id="thread-context-menu"]')).not.toBeNull()
})

const projectChannel = (id: string, threads?: ThreadLinkDto[]): ProjectChannelDto => ({
  id,
  projectId: 'project',
  name: id,
  agentId: 'agent',
  workspaceMode: 'main_checkout',
  kind: id === 'general' ? 'general' : 'legacy',
  branch: null,
  archived: false,
  archivedInto: null,
  threads,
})
const live = (id: string, threads?: ThreadLinkDto[]): ChannelDto => ({
  id,
  projectId: 'project',
  agentId: 'agent',
  name: id,
  agent: 'Agent',
  provider: 'p',
  workspaceMode: 'main_checkout',
  running: false,
  unread: false,
  pendingApprovals: 0,
  threads,
})
const listProps = {
  projectId: 'project',
  projectPath: '/repo',
  channelId: 'general',
  repositories: [{ id: 'repo', label: 'Repo', path: '/repo' }],
  branches: [],
}

test('followed threads show under their channel, live state first, stored state as the fallback', () => {
  const stored = link('stored', { name: 'From storage' })
  const view = track(
    mount(ChannelList, {
      props: { ...listProps, projectChannels: [projectChannel('general', [stored])] },
    }),
  )
  expect(view.findAll('[data-ui-id="thread-row"]').map((row) => row.get('button').text())).toEqual([
    'From storage',
  ])
  return view
    .setProps({
      channels: [live('general', [link('live', { name: 'Live thread', unread: true })])],
    })
    .then(() => {
      const rows = view.findAll('[data-ui-id="thread-row"]')
      expect(rows.map((row) => row.get('button').text())).toEqual(['Live thread'])
      expect(rows[0]!.find('[aria-label="Unread messages"]').exists()).toBe(true)
    })
})

test('a channel with no followed threads shows none', () => {
  const view = track(
    mount(ChannelList, { props: { ...listProps, projectChannels: [projectChannel('general')] } }),
  )
  expect(view.find('[data-ui-id="thread-row"]').exists()).toBe(false)
})

test('selecting or leaving a thread row in the channel bar emits the thread id and highlights the open one', async () => {
  const view = track(
    mount(ChannelList, {
      attachTo: document.body,
      props: {
        ...listProps,
        openThreadId: 'b',
        projectChannels: [projectChannel('general', [link('a'), link('b')])],
      },
    }),
  )
  const [first, second] = view.findAll('[data-ui-id="thread-row"]')
  expect(first!.get('button').attributes('aria-current')).toBeUndefined()
  expect(second!.get('button').attributes('aria-current')).toBe('page')
  await first!.get('button').trigger('click')
  expect(view.emitted('selectThread')).toEqual([['a']])
  await second!.get('button').trigger('contextmenu', { button: 2, clientX: 20, clientY: 20 })
  await flushPromises()
  view
    .findAllComponents(ContextMenuItem)
    .find((item) => item.text() === 'Leave thread')!
    .vm.$emit('select')
  expect(view.emitted('leaveThread')).toEqual([['b']])
})

test('threads show under saved channels too', () => {
  const view = track(
    mount(ChannelList, {
      props: {
        ...listProps,
        projectChannels: [projectChannel('general'), projectChannel('saved', [link('s')])],
      },
    }),
  )
  expect(view.findAll('[data-ui-id="thread-row"]')).toHaveLength(0)
  return view
    .get('[data-ui-id="saved-channel-toggle"]')
    .trigger('click')
    .then(() => {
      expect(view.findAll('[data-ui-id="thread-row"]')).toHaveLength(1)
    })
})

const agents = [
  { id: 'a', name: 'Robin', avatar: null, provider: 'codex', model: 'm', archived: false },
]
const conversation = (patch: Record<string, unknown>) => ({
  inDmList: true,
  isDefault: true,
  name: 'General',
  agentId: 'a',
  agent: 'Robin',
  projectId: null,
  deleted: false,
  agentArchived: false,
  archived: false,
  ...patch,
})
beforeEach(() => {
  vi.mocked(invoke)
    .mockReset()
    .mockImplementation(async (name) => {
      if (name === 'settings_agent_get') return agents
      if (name === 'get_conversations')
        return [
          conversation({ id: 'dm', threads: [link('dm-thread', { parentChannelId: 'dm' })] }),
          conversation({
            id: 'project-one',
            projectId: 'project',
            name: 'Design',
            threads: [link('p-thread', { parentChannelId: 'project-one' })],
          }),
        ]
    })
})

test('followed threads show under the direct message contact and select by thread id', async () => {
  const view = track(mount(AgentsNavigation, { props: { projectId: null } }))
  await flushPromises()
  const rows = view.findAll('[data-ui-id="thread-row"]')
  expect(rows.map((row) => row.get('button').text())).toEqual(['Thread dm-thread'])
  await rows[0]!.get('button').trigger('click')
  expect(view.emitted('selectThread')).toEqual([['dm-thread']])
  await view.setProps({ channels: [live('dm', [link('live-thread')])] })
  expect(view.findAll('[data-ui-id="thread-row"]').map((row) => row.get('button').text())).toEqual([
    'Thread live-thread',
  ])
})

test('followed threads show under project conversations', async () => {
  const view = track(
    mount(AgentsNavigation, { props: { projectId: 'project', openThreadId: 'p-thread' } }),
  )
  await flushPromises()
  const rows = view.findAll('[data-ui-id="thread-row"]')
  expect(rows.map((row) => row.get('button').text())).toEqual(['Thread p-thread'])
  expect(rows[0]!.get('button').attributes('aria-current')).toBe('page')
})
