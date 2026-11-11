import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import ChannelList from './ChannelList.vue'
import NavigationContextMenu from './NavigationContextMenu.vue'
import { ContextMenuItem } from './ui/context-menu'
import { clearAction, pendingAction } from './thread-actions'
import type { BranchRowDto, ChannelDto, ProjectChannelDto, ThreadLinkDto } from '../bridge/types'

vi.mock('@/bridge/api', () => ({ createTextChannel: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

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
const thread = (id: string): ThreadLinkDto =>
  ({
    id,
    parentChannelId: 'plans',
    name: `Thread ${id}`,
    kind: 'question',
    state: null,
    running: false,
    unread: false,
    pendingApprovals: 0,
  }) as unknown as ThreadLinkDto
const projectChannels = [
  channel('general', { kind: 'general', name: 'general' }),
  channel('plans', { name: 'plans' }),
  channel('notes', { name: 'notes' }),
  channel('old-text', { name: 'old-text', archived: true, hasMessages: true }),
]
const props = {
  projectId: 'project',
  activeProjectId: 'project',
  projectPath: '/repo/text',
  channelId: 'general',
  repositories: [{ id: '', label: 'Repo', path: '/repo' }],
  branches: [] as BranchRowDto[],
  projectChannels,
}
const views: ReturnType<typeof mount>[] = []
const track = (view: ReturnType<typeof mount>) => (views.push(view), view)
// The section state is saved for each project path.
beforeEach(() => localStorage.clear())
afterEach(() => {
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
  clearAction()
})

test('the Text channels section lists general first, then text channels in creation order', () => {
  const view = track(mount(ChannelList, { props }))
  const section = view.get('[data-ui-id="text-channels-list"]')
  expect(section.get('[data-ui-id="text-channels-toggle"]').text()).toBe('Text channels 3')
  const rows = section
    .findAll('button[data-ui-id]')
    .filter(
      (row) =>
        /channel-/.test(row.attributes('data-ui-id')!) ||
        row.attributes('data-ui-id') === 'project-general-channel',
    )
  const labels = rows.map((row) => row.text()).filter(Boolean)
  expect(labels).toEqual(['general', 'plans', 'notes'])
  expect(view.find('[data-ui-id="text-channel-plans"]').exists()).toBe(true)
  expect(view.find('[data-ui-id="text-channel-old-text"]').exists()).toBe(false)
  expect(view.text()).not.toContain('Branch channels Text')
})

test('a text channel row opens its channel and marks the open channel', async () => {
  const view = track(mount(ChannelList, { props: { ...props, channelId: 'plans' } }))
  const row = view.get('[data-ui-id="text-channel-plans"]')
  expect(row.attributes('aria-current')).toBe('page')
  expect(
    view.get('[data-ui-id="project-general-channel"]').attributes('aria-current'),
  ).toBeUndefined()
  await view.get('[data-ui-id="text-channel-notes"]').trigger('click')
  await view.get('[data-ui-id="project-general-channel"]').trigger('click')
  expect(view.emitted('selectChannel')).toEqual([['notes'], ['general']])
})

test('a text channel row shows its threads, spinner, and unread mark', () => {
  const view = track(
    mount(ChannelList, {
      props: {
        ...props,
        projectChannels: [
          projectChannels[0]!,
          channel('plans', { threads: [thread('t1'), thread('t2')] }),
          channel('notes'),
        ],
        channels: [live('plans', { unread: true }), live('notes', { running: true })],
      },
    }),
  )
  const plans = view.get('[data-ui-id="text-channel-plans"]')
  expect(plans.find('[data-ui-id="channel-unread-pip"]').exists()).toBe(true)
  expect(view.findAll('[data-ui-id="thread-row"]')).toHaveLength(2)
  expect(
    view
      .get('[data-ui-id="text-channel-notes"]')
      .find('[data-ui-id="channel-unread-pip"]')
      .exists(),
  ).toBe(false)
  expect(
    view
      .get('[data-ui-id="text-channel-notes"]')
      .findComponent({ name: 'BrailleSpinner' })
      .exists(),
  ).toBe(true)
})

test('the section stays when there are no text channels, and it can collapse', async () => {
  const view = track(
    mount(ChannelList, { props: { ...props, projectChannels: [projectChannels[0]!] } }),
  )
  expect(view.get('[data-ui-id="text-channels-toggle"]').text()).toBe('Text channels 1')
  expect(view.find('[data-ui-id="project-general-channel"]').exists()).toBe(true)
  await view.get('[data-ui-id="text-channels-toggle"]').trigger('click')
  expect(view.find('[data-ui-id="project-general-channel"]').exists()).toBe(false)
})

test('general without a loaded channel still opens the project', async () => {
  const view = track(mount(ChannelList, { props: { ...props, projectChannels: [] } }))
  await view.get('[data-ui-id="project-general-channel"]').trigger('click')
  expect(view.emitted('action')).toEqual([['select_project:project']])
})

test('the plus button opens the new text channel dialog', async () => {
  const view = track(mount(ChannelList, { attachTo: document.body, props }))
  expect(document.querySelector('[data-ui-id="create-text-channel-dialog"]')).toBeNull()
  const button = view.get('[data-ui-id="text-channels-create"]')
  expect(button.attributes('aria-label')).toBe('New text channel')
  expect(button.attributes('disabled')).toBeUndefined()
  await button.trigger('click')
  await flushPromises()
  expect(document.querySelector('[data-ui-id="create-text-channel-dialog"]')).not.toBeNull()
  expect(document.body.textContent).toContain(
    'Use a text channel for a project conversation that has no branch.',
  )
})

test('the plus button is disabled while the app is busy', () => {
  const view = track(mount(ChannelList, { props: { ...props, busy: true } }))
  expect(view.get('[data-ui-id="text-channels-create"]').attributes('disabled')).toBeDefined()
})

test('a text channel menu has Rename as a working item; Mute and Archive stay disabled Soon items', async () => {
  const view = track(mount(ChannelList, { attachTo: document.body, props }))
  const menus = view.findAllComponents(NavigationContextMenu)
  const channelMenus = menus.filter((menu) => menu.props('kind') === 'channel')
  expect(
    channelMenus.map((menu) => [
      menu.props('id'),
      menu.props('name'),
      menu.props('renamable') ?? false,
    ]),
  ).toEqual([
    ['general', 'general', false],
    ['plans', 'plans', true],
    ['notes', 'notes', true],
  ])
  await view
    .get('[data-ui-id="text-channel-plans"]')
    .trigger('contextmenu', { button: 2, clientX: 10, clientY: 10 })
  await flushPromises()
  const items = view.findAllComponents(ContextMenuItem)
  const labels = items.map((item) => item.text().replace(/\s+/g, ' '))
  expect(labels.filter((label) => /Mute|Rename Channel|Archive/.test(label))).toEqual([
    'Mute Channel Soon',
    'Rename Channel…',
    'Archive Channel… Soon',
  ])
  const rename = items.find((item) => item.text() === 'Rename Channel…')!
  expect(rename.attributes('aria-disabled')).toBeUndefined()
  for (const label of ['Mute Channel Soon', 'Archive Channel… Soon'])
    expect(
      items.find((item) => item.text().replace(/\s+/g, ' ') === label)!.attributes('aria-disabled'),
    ).toBe('true')
  rename.vm.$emit('select')
  expect(pendingAction.value).toEqual({
    kind: 'rename-channel',
    projectId: 'project',
    channelId: 'plans',
    name: 'plans',
  })
})

test('the general row menu has no Rename item', async () => {
  const view = track(mount(ChannelList, { attachTo: document.body, props }))
  await view
    .get('[data-ui-id="project-general-channel"]')
    .trigger('contextmenu', { button: 2, clientX: 10, clientY: 10 })
  await flushPromises()
  const items = view
    .findAllComponents(ContextMenuItem)
    .map((item) => item.text().replace(/\s+/g, ' '))
  expect(items).toContain('Mute Channel Soon')
  expect(items).toContain('Archive Channel… Soon')
  expect(items.some((item) => item.startsWith('Rename'))).toBe(false)
})

test('Archived stays hidden unless the project setting is on', async () => {
  const view = track(mount(ChannelList, { props }))
  expect(view.find('[data-ui-id="archived-channel-navigation"]').exists()).toBe(false)
  expect(view.text()).not.toContain('Archived')
  await view.setProps({ showArchived: true })
  const section = view.get('[data-ui-id="archived-channel-navigation"]')
  expect(section.get('[data-ui-id="archived-channel-toggle"]').text()).toBe('Archived')
  expect(view.find('[data-ui-id="archived-channel-old-text"]').exists()).toBe(false)
  await view.get('[data-ui-id="archived-channel-toggle"]').trigger('click')
  const row = view.get('[data-ui-id="archived-channel-old-text"]')
  expect(row.text()).toContain('old-text')
  expect(row.text()).toContain('History')
  await row.trigger('click')
  expect(view.emitted('selectChannel')).toEqual([['old-text']])
})

test('Archived stays hidden when its list is empty, even with the setting on', () => {
  const view = track(
    mount(ChannelList, {
      props: { ...props, showArchived: true, projectChannels: projectChannels.slice(0, 3) },
    }),
  )
  expect(view.find('[data-ui-id="archived-channel-navigation"]').exists()).toBe(false)
})

test('Archived stays hidden while the setting is off, even when the open channel is in it', () => {
  const view = track(mount(ChannelList, { props: { ...props, channelId: 'old-text' } }))
  expect(view.find('[data-ui-id="archived-channel-navigation"]').exists()).toBe(false)
})

test('a deleted branch channel shows in Archived only with messages', async () => {
  const branch = (id: string, hasMessages: boolean) =>
    channel(id, { kind: 'branch', branch: id, name: id, hasMessages })
  const view = track(
    mount(ChannelList, {
      props: {
        ...props,
        showArchived: true,
        projectChannels: [
          projectChannels[0]!,
          branch('with-messages', true),
          branch('empty', false),
        ],
      },
    }),
  )
  await view.get('[data-ui-id="archived-channel-toggle"]').trigger('click')
  expect(view.find('[data-ui-id="archived-channel-with-messages"]').exists()).toBe(true)
  expect(view.find('[data-ui-id="archived-channel-empty"]').exists()).toBe(false)
})
