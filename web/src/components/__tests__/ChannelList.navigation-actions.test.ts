import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { toast } from 'vue-sonner'
import ChannelList from '../ChannelList.vue'
import { ContextMenuItem } from '../ui/context-menu'
import { clearAction, pendingAction } from '../thread-actions'
import { setChannelMuted } from '@/bridge/api'
import type { BranchRowDto, ChannelDto, ProjectChannelDto } from '../../bridge/types'

vi.mock('@/bridge/api', () => ({ createTextChannel: vi.fn(), setChannelMuted: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))

const channel = (id: string, extra: Partial<ProjectChannelDto> = {}): ProjectChannelDto => ({
  id,
  projectId: 'project',
  name: id,
  agentId: 'agent',
  workspaceMode: 'main_checkout',
  kind: 'text',
  branch: null,
  archived: false,
  archivedInto: null,
  hasMessages: false,
  ...extra,
})
const live = (id: string, extra: Partial<ChannelDto> = {}): ChannelDto => ({
  id,
  projectId: 'project',
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
const base = { repository: '', depth: 0, current: false, collapsed: false, hasChildren: false }
const rows: BranchRowDto[] = [
  { ...base, key: 'main', label: 'main', kind: 'main', worktree: '/repo' },
  { ...base, key: 'free', label: 'free', kind: 'branch', depth: 1 },
  { ...base, key: 'linked', label: 'linked', kind: 'worktree', worktree: '/linked' },
]
const props = {
  projectId: 'project',
  activeProjectId: 'project',
  projectPath: '/repo',
  channelId: 'general',
  repositories: [{ id: '', label: 'Repo', path: '/repo' }],
  branches: rows,
  projectChannels: [
    channel('general', { kind: 'general' }),
    channel('plans'),
    channel('old', { archived: true, hasMessages: true }),
  ],
  showArchived: true,
}
const views: ReturnType<typeof mount>[] = []
const track = (view: ReturnType<typeof mount>) => (views.push(view), view)
beforeEach(() => {
  localStorage.clear()
  vi.mocked(setChannelMuted).mockReset().mockResolvedValue(undefined)
  vi.mocked(toast.error).mockReset()
})
afterEach(() => {
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
  clearAction()
})

/** Open the menu of a row and return its items by label. */
async function menu(view: ReturnType<typeof mount>, row: string) {
  await view
    .get(`[data-ui-id="${row}"]`)
    .trigger('contextmenu', { button: 2, clientX: 5, clientY: 5 })
  await flushPromises()
  const items = view.findAllComponents(ContextMenuItem)
  const find = (label: string) => items.find((item) => item.text() === label)!
  const blocked = (label: string) => find(label).attributes('aria-disabled') === 'true'
  return { find, blocked, labels: items.map((item) => item.text()) }
}
const mountList = (extra: Record<string, unknown> = {}) =>
  track(mount(ChannelList, { attachTo: document.body, props: { ...props, ...extra } }))

test('#general cannot be archived but can be muted', async () => {
  const view = mountList()
  const items = await menu(view, 'project-general-channel')
  expect(items.blocked('Archive Channel…')).toBe(true)
  expect(items.blocked('Mute Channel')).toBe(false)
  items.find('Archive Channel…').vm.$emit('select')
  expect(pendingAction.value).toBeNull()
  items.find('Mute Channel').vm.$emit('select')
  expect(setChannelMuted).toHaveBeenCalledWith('project', 'general', true)
})

test('Archive Channel… on a text row asks for the dialog with the channel name', async () => {
  const view = mountList()
  const items = await menu(view, 'text-channel-plans')
  expect(items.blocked('Archive Channel…')).toBe(false)
  items.find('Archive Channel…').vm.$emit('select')
  expect(pendingAction.value).toEqual({
    kind: 'archive-channel',
    projectId: 'project',
    channelId: 'plans',
    name: 'plans',
  })
})

test('Archive Channel… is disabled, with no Soon label, for a busy channel', async () => {
  for (const extra of [
    { running: true },
    { pendingApprovals: 1 },
    { threadPendingApprovals: 2 },
    {
      threads: [
        {
          id: 't',
          parentChannelId: 'plans',
          name: 'T',
          unread: false,
          running: false,
          pendingApprovals: 0,
          kind: 'sub_agent',
          state: 'running',
        },
      ] as ChannelDto['threads'],
    },
  ]) {
    const view = mountList({ channels: [live('plans', extra)] })
    const items = await menu(view, 'text-channel-plans')
    expect(items.blocked('Archive Channel…'), JSON.stringify(extra)).toBe(true)
    expect(items.find('Archive Channel…').text()).not.toContain('Soon')
    view.unmount()
    document.body.innerHTML = ''
  }
})

test('the archive of the open channel blocks its own Archive Channel… item', async () => {
  const view = mountList({ channelId: 'plans', activeChannelBusy: true })
  expect((await menu(view, 'text-channel-plans')).blocked('Archive Channel…')).toBe(true)
})

test('an archived channel cannot be archived again, but it can be muted', async () => {
  const view = mountList()
  await view.get('[data-ui-id="archived-channel-toggle"]').trigger('click')
  const items = await menu(view, 'archived-channel-old')
  expect(items.blocked('Archive Channel…')).toBe(true)
  expect(items.blocked('Mute Channel')).toBe(false)
})

test('Mute Channel and Unmute Channel call the app with the opposite of the saved flag', async () => {
  const view = mountList()
  await (await menu(view, 'text-channel-plans')).find('Mute Channel').vm.$emit('select')
  expect(setChannelMuted).toHaveBeenLastCalledWith('project', 'plans', true)
  await view.setProps({
    projectChannels: [channel('general', { kind: 'general' }), channel('plans', { muted: true })],
  })
  const items = await menu(view, 'text-channel-plans')
  expect(items.labels).toContain('Unmute Channel')
  expect(items.labels).not.toContain('Mute Channel')
  items.find('Unmute Channel').vm.$emit('select')
  expect(setChannelMuted).toHaveBeenLastCalledWith('project', 'plans', false)
})

test('a failed mute shows an error toast', async () => {
  vi.mocked(setChannelMuted).mockRejectedValue('no')
  const view = mountList()
  ;(await menu(view, 'text-channel-plans')).find('Mute Channel').vm.$emit('select')
  await flushPromises()
  expect(toast.error).toHaveBeenCalledWith('Could not mute channel', { description: 'no' })
})

test('a muted row is dimmed, shows a Muted bell and has no unread marker', async () => {
  const view = mountList({
    projectChannels: [
      channel('general', { kind: 'general' }),
      channel('plans', { muted: true }),
      channel('notes'),
    ],
    channels: [live('plans', { unread: true, muted: true }), live('notes', { unread: true })],
  })
  const plans = view.get('[data-ui-id="text-channel-plans"]')
  expect(plans.get('[aria-label="Muted"]').attributes('role')).toBe('img')
  expect(plans.classes()).toContain('opacity-60')
  expect(plans.find('[data-ui-id="channel-unread-pip"]').exists()).toBe(false)
  const notes = view.get('[data-ui-id="text-channel-notes"]')
  expect(notes.find('[aria-label="Muted"]').exists()).toBe(false)
  expect(notes.classes()).not.toContain('opacity-60')
  expect(notes.get('[data-ui-id="channel-unread-pip"]').attributes('aria-label')).toBe(
    'Unread messages',
  )
})

test('unmuting shows the unread marker again', async () => {
  const view = mountList({
    projectChannels: [channel('general', { kind: 'general' }), channel('plans', { muted: true })],
    channels: [live('plans', { unread: false, muted: true })],
  })
  expect(view.find('[data-ui-id="channel-unread-pip"]').exists()).toBe(false)
  await view.setProps({
    projectChannels: [channel('general', { kind: 'general' }), channel('plans')],
    channels: [live('plans', { unread: true })],
  })
  expect(view.find('[aria-label="Muted"]').exists()).toBe(false)
  expect(view.find('[data-ui-id="channel-unread-pip"]').exists()).toBe(true)
})

test('an unread thread of a muted channel still shows in the thread row', () => {
  const link = {
    id: 't',
    parentChannelId: 'plans',
    name: 'Why',
    unread: true,
    running: false,
    pendingApprovals: 0,
    kind: 'question',
    state: null,
  }
  const view = mountList({
    projectChannels: [
      channel('general', { kind: 'general' }),
      channel('plans', { muted: true, threads: [link] } as Partial<ProjectChannelDto>),
    ],
    channels: [live('plans', { muted: true, threads: [link] as ChannelDto['threads'] })],
  })
  expect(
    view
      .get('[data-ui-id="text-channel-plans"]')
      .find('[data-ui-id="channel-unread-pip"]')
      .exists(),
  ).toBe(false)
  expect(view.findAll('[data-ui-id="channel-unread-pip"]')).toHaveLength(1)
})

test('a branch that no working tree holds can get a worktree, be renamed and be deleted', async () => {
  const view = mountList()
  const items = await menu(view, 'branches-row-free')
  for (const label of ['Create Worktree…', 'Rename Branch…', 'Delete Branch…'])
    expect(items.blocked(label), label).toBe(false)
  expect(items.labels.join()).not.toContain('Soon')
  items.find('Create Worktree…').vm.$emit('select')
  expect(pendingAction.value).toEqual({
    kind: 'create-worktree',
    projectId: 'project',
    repository: '',
    branch: 'free',
  })
  items.find('Rename Branch…').vm.$emit('select')
  expect(pendingAction.value).toMatchObject({ kind: 'rename-branch', branch: 'free' })
  items.find('Delete Branch…').vm.$emit('select')
  expect(pendingAction.value).toMatchObject({ kind: 'delete-branch', branch: 'free' })
})

test('the default branch and a checked-out branch cannot get a worktree or be deleted', async () => {
  for (const [row, label] of [
    ['branches-row-main', 'main'],
    ['branches-row-linked', 'linked'],
  ] as const) {
    const view = mountList()
    const items = await menu(view, row)
    expect(items.blocked('Create Worktree…'), label).toBe(true)
    expect(items.blocked('Delete Branch…'), label).toBe(true)
    expect(items.blocked('Rename Branch…'), label).toBe(false)
    items.find('Delete Branch…').vm.$emit('select')
    expect(pendingAction.value).toBeNull()
    view.unmount()
    document.body.innerHTML = ''
  }
})

test('a branch whose channel already has a worktree cannot get another one', async () => {
  const view = mountList({
    projectChannels: [
      ...props.projectChannels,
      channel('free-channel', {
        kind: 'branch',
        branch: 'free',
        repository: '',
        worktree: { path: '/elsewhere', branch: 'free' },
      }),
    ],
  })
  const items = await menu(view, 'branches-row-free')
  expect(items.blocked('Create Worktree…')).toBe(true)
  expect(items.blocked('Rename Branch…')).toBe(false)
  expect(items.blocked('Delete Branch…')).toBe(false)
})

test('a busy branch channel blocks Rename Branch… and Delete Branch…', async () => {
  const view = mountList({
    projectChannels: [
      ...props.projectChannels,
      channel('free-channel', { kind: 'branch', branch: 'free', repository: '' }),
    ],
    channels: [live('free-channel', { running: true })],
  })
  const items = await menu(view, 'branches-row-free')
  expect(items.blocked('Rename Branch…')).toBe(true)
  expect(items.blocked('Delete Branch…')).toBe(true)
  expect(items.blocked('Create Worktree…')).toBe(false)
  expect(items.find('Rename Branch…').text()).not.toContain('Soon')
})

test('the main checkout cannot be removed, and a linked worktree can', async () => {
  const view = mountList()
  const main = await menu(view, 'worktrees-row-main')
  expect(main.blocked('Remove Worktree…')).toBe(true)
  expect(main.blocked('Create Branch…')).toBe(false)
  view.unmount()
  document.body.innerHTML = ''
  const second = mountList()
  const linked = await menu(second, 'worktrees-row-linked')
  expect(linked.blocked('Remove Worktree…')).toBe(false)
  expect(linked.blocked('Create Branch…')).toBe(false)
  expect(linked.labels.join()).not.toContain('Soon')
  linked.find('Create Branch…').vm.$emit('select')
  expect(pendingAction.value).toEqual({
    kind: 'create-branch',
    projectId: 'project',
    repository: '',
    worktree: '/linked',
    name: 'linked',
  })
  linked.find('Remove Worktree…').vm.$emit('select')
  expect(pendingAction.value).toEqual({
    kind: 'remove-worktree',
    projectId: 'project',
    repository: '',
    worktree: '/linked',
  })
})

test('a busy channel that uses a working tree blocks Create Branch… and Remove Worktree…', async () => {
  const bound = channel('bound', {
    kind: 'legacy',
    worktree: { path: '/linked', branch: 'linked' },
  })
  const view = mountList({
    projectChannels: [...props.projectChannels, bound],
    channels: [live('bound', { pendingApprovals: 1 })],
  })
  const items = await menu(view, 'worktrees-row-linked')
  expect(items.blocked('Create Branch…')).toBe(true)
  expect(items.blocked('Remove Worktree…')).toBe(true)
})

test('the branch channel of the branch that a working tree holds counts as a user of the tree', async () => {
  const view = mountList({
    projectChannels: [
      ...props.projectChannels,
      channel('linked-channel', { kind: 'branch', branch: 'linked', repository: '' }),
    ],
    channels: [live('linked-channel', { running: true })],
  })
  const items = await menu(view, 'worktrees-row-linked')
  expect(items.blocked('Create Branch…')).toBe(true)
  expect(items.blocked('Remove Worktree…')).toBe(true)
})

test('an archived channel that held the worktree does not block its removal', async () => {
  const view = mountList({
    projectChannels: [
      ...props.projectChannels,
      channel('gone', {
        archived: true,
        kind: 'legacy',
        worktree: { path: '/linked', branch: 'linked' },
      }),
    ],
    channels: [live('gone', { running: true })],
  })
  const items = await menu(view, 'worktrees-row-linked')
  expect(items.blocked('Remove Worktree…')).toBe(false)
})
