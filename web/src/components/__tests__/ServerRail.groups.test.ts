import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { nextTick } from 'vue'
import type { RailLayoutDto } from '../../bridge/api'
import type { ChannelDto } from '../../bridge/types'
import GroupContextMenu from '../GroupContextMenu.vue'
import ProjectContextMenu from '../ProjectContextMenu.vue'
import ServerRail from '../ServerRail.vue'
import type { RailChange } from '../rail-layout'

vi.mock('@/bridge/api', () => ({ contextMenu: vi.fn(async () => []), pluginMenuAction: vi.fn() }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))

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
  { id: 'c', name: 'Gamma', path: '/c' },
  { id: 'd', name: 'Delta', path: '/d' },
  { id: 'e', name: 'Epsilon', path: '/e' },
  { id: 'f', name: 'Zeta', path: '/f' },
]
const group = (id: string, projectIds: string[], extra: object = {}) => ({
  kind: 'group' as const,
  id,
  name: '',
  color: null,
  collapsed: true,
  projectIds,
  ...extra,
})
type Rail = VueWrapper<InstanceType<typeof ServerRail>>
const wrappers: Rail[] = []
function mountRail(
  layout: RailLayoutDto | null,
  extra: { channels?: ChannelDto[]; projectId?: string; compact?: boolean } = {},
): Rail {
  const wrapper = mount(ServerRail, {
    attachTo: document.body,
    props: {
      projects,
      layout,
      projectId: extra.projectId ?? 'a',
      channels: extra.channels ?? [],
      project: 'Alpha',
      unread: false,
      pendingApprovals: 0,
      compact: extra.compact,
    },
  })
  wrappers.push(wrapper)
  return wrapper
}
afterEach(() => {
  wrappers.splice(0).forEach((wrapper) => wrapper.unmount())
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})
const changes = (wrapper: Rail) =>
  (wrapper.emitted('railChange') ?? []).map(([change]) => change as RailChange)
const unread = (wrapper: Rail, label: string) =>
  wrapper.get(`[aria-label="${label}"]`).attributes('aria-description') === 'Unread messages'

test('a collapsed group is one tile with up to four member avatars and its name as the label', () => {
  const wrapper = mountRail({
    items: [
      group('g', ['a', 'b', 'c', 'd', 'e'], { name: 'Work' }),
      group('h', ['f'], { name: '' }),
    ],
  })
  const tile = wrapper.get('[data-ui-id="rail-group-g"]')
  expect(tile.attributes('aria-label')).toBe('Work')
  expect(tile.attributes('aria-expanded')).toBe('false')
  expect(tile.findAll('[data-ui-id="project-avatar"]')).toHaveLength(4)
  const single = wrapper.get('[data-ui-id="rail-group-h"]')
  expect(single.attributes('aria-label')).toBe('Zeta')
  expect(single.findAll('[data-ui-id="project-avatar"]')).toHaveLength(1)
  expect(wrapper.find('[aria-label="Alpha"]').exists()).toBe(false)
})

test('a collapsed group is as wide as an expanded group in every rail size', () => {
  const layout = { items: [group('g', ['a', 'b']), { kind: 'project' as const, projectId: 'c' }] }
  for (const [compact, tile, folder, width] of [
    [false, 'size-12', 'size-14', 'w-14'],
    [true, 'size-9', 'size-11', 'w-11'],
  ] as const) {
    const wrapper = mountRail(layout, { compact })
    expect(wrapper.get('[data-ui-id="rail-group-g"]').classes()).toContain(folder)
    expect(wrapper.get('[aria-label="Gamma"]').classes()).toContain(tile)
    const open = mountRail({ items: [group('g', ['a', 'b'], { collapsed: false })] }, { compact })
    expect(open.get('[data-ui-id="rail-group-container-g"]').classes()).toContain(width)
  }
})

test('clicking a collapsed group expands it, and the header of an expanded group collapses it', async () => {
  const wrapper = mountRail({ items: [group('g', ['a', 'b'])] })
  await wrapper.get('[data-ui-id="rail-group-g"]').trigger('click')
  expect(changes(wrapper)).toEqual([{ type: 'collapse-group', groupId: 'g', collapsed: false }])
  expect(wrapper.emitted('project')).toBeUndefined()
  await wrapper.setProps({ layout: { items: [group('g', ['a', 'b'], { collapsed: false })] } })
  const header = wrapper.get('[data-ui-id="rail-group-header-g"]')
  expect(header.attributes('aria-label')).toBe('Alpha, Beta')
  expect(header.attributes('aria-expanded')).toBe('true')
  await header.trigger('click')
  expect(changes(wrapper)[1]).toEqual({ type: 'collapse-group', groupId: 'g', collapsed: true })
})

test('an expanded group holds the member tiles, which select like ungrouped tiles', async () => {
  const wrapper = mountRail({
    items: [group('g', ['a', 'b'], { collapsed: false }), { kind: 'project', projectId: 'c' }],
  })
  const container = wrapper.get('[data-ui-id="rail-group-container-g"]')
  expect(
    container.findAll('[data-rail-member]').map((tile) => tile.attributes('data-rail-member')),
  ).toEqual(['a', 'b'])
  expect(container.find('[aria-label="Gamma"]').exists()).toBe(false)
  await container.get('[aria-label="Beta"]').trigger('click')
  expect(wrapper.emitted('project')).toEqual([['b']])
  expect(
    wrapper.findAllComponents(ProjectContextMenu).map((menu) => menu.props('project').id),
  ).toEqual(['a', 'b', 'c', 'd', 'e', 'f'])
})

test('a collapsed group shows unread when any member is unread, and ignores muted channels', () => {
  const layout = { items: [group('g', ['a', 'b']), group('h', ['c', 'd'])] }
  const wrapper = mountRail(layout, {
    channels: [
      live('quiet', 'a'),
      live('loud', 'b', { unread: true }),
      live('muted', 'c', { unread: true, muted: true }),
    ],
  })
  expect(unread(wrapper, 'Alpha, Beta')).toBe(true)
  expect(unread(wrapper, 'Gamma, Delta')).toBe(false)
})

test('a collapsed group shows the summed approval badge, including muted channels', () => {
  const wrapper = mountRail(
    { items: [group('g', ['a', 'b', 'c'])] },
    {
      channels: [
        live('one', 'a', { pendingApprovals: 1 }),
        live('two', 'b', { pendingApprovals: 2 }),
        live('muted', 'b', { muted: true, pendingApprovals: 4 }),
        live('other', 'd', { pendingApprovals: 8 }),
      ],
    },
  )
  expect(wrapper.get('[data-ui-id="rail-group-g"] [data-testid="approval-badge"]').text()).toBe('7')
})

test('an expanded group keeps the badges and unread marks on its member tiles', () => {
  const wrapper = mountRail(
    { items: [group('g', ['a', 'b'], { collapsed: false })] },
    { channels: [live('x', 'a', { unread: true, pendingApprovals: 3 })] },
  )
  expect(unread(wrapper, 'Alpha')).toBe(true)
  expect(unread(wrapper, 'Beta')).toBe(false)
  expect(wrapper.get('[aria-label="Alpha"] [data-testid="approval-badge"]').text()).toBe('3')
  expect(
    wrapper.find('[data-ui-id="rail-group-header-g"] [data-testid="approval-badge"]').exists(),
  ).toBe(false)
})

test('the selection pill shows on a collapsed group that holds the selected project', () => {
  const layout = { items: [group('g', ['a', 'b']), { kind: 'project' as const, projectId: 'c' }] }
  const inside = mountRail(layout, { projectId: 'b' })
  expect(inside.get('[data-ui-id="rail-group-g"]').attributes('aria-current')).toBe('page')
  expect(
    inside
      .get('[data-ui-id="project-selection-pill"]')
      .element.parentElement?.contains(inside.get('[data-ui-id="rail-group-g"]').element),
  ).toBe(true)
  expect(inside.find('[data-ui-id="project-selection-pill-c"]').exists()).toBe(true)
  const outside = mountRail(layout, { projectId: 'c' })
  expect(outside.get('[data-ui-id="rail-group-g"]').attributes('aria-current')).toBeUndefined()
  expect(outside.find('[data-ui-id="group-selection-pill-g"]').exists()).toBe(true)
})

test('an expanded group keeps the selection pill on the selected member tile', () => {
  const wrapper = mountRail(
    { items: [group('g', ['a', 'b'], { collapsed: false })] },
    { projectId: 'b' },
  )
  expect(wrapper.get('[aria-label="Beta"]').attributes('aria-current')).toBe('page')
  expect(wrapper.find('[data-ui-id="project-selection-pill"]').exists()).toBe(true)
  expect(wrapper.find('[data-ui-id="project-selection-pill-a"]').exists()).toBe(true)
})

test('the project menu of a rail tile gets the groups and its own group', () => {
  const wrapper = mountRail({
    items: [group('g', ['a'], { name: 'Work' }), { kind: 'project', projectId: 'b' }],
  })
  const menus = wrapper.findAllComponents(ProjectContextMenu)
  expect(menus.map((menu) => menu.props('project').id)).toEqual(['b', 'c', 'd', 'e', 'f'])
  for (const menu of menus)
    expect(menu.props('rail')).toEqual({ groups: [{ id: 'g', label: 'Work' }], groupId: null })
})

test('project menu rail actions become rail changes', async () => {
  const wrapper = mountRail({
    items: [
      group('g', ['a', 'b', 'c'], { collapsed: false }),
      group('h', ['d'], { name: 'Other' }),
      { kind: 'project', projectId: 'e' },
    ],
  })
  const menu = (id: string) =>
    wrapper.findAllComponents(ProjectContextMenu).find((entry) => entry.props('project').id === id)!
  const project = (id: string) => projects.find((entry) => entry.id === id)!
  menu('b').vm.$emit('rail', 'group-none', project('b'))
  menu('e').vm.$emit('rail', 'group-join', project('e'), 'g')
  menu('b').vm.$emit('rail', 'group-join', project('b'), 'h')
  menu('b').vm.$emit('rail', 'move-first', project('b'))
  menu('b').vm.$emit('rail', 'move-last', project('b'))
  // Already first, last, or in the group: nothing to change.
  menu('a').vm.$emit('rail', 'move-first', project('a'))
  menu('c').vm.$emit('rail', 'move-last', project('c'))
  menu('a').vm.$emit('rail', 'group-join', project('a'), 'g')
  menu('e').vm.$emit('rail', 'group-none', project('e'))
  menu('f').vm.$emit('rail', 'move-last', project('f'))
  menu('e').vm.$emit('rail', 'move-first', project('e'))
  await nextTick()
  expect(changes(wrapper)).toEqual([
    { type: 'move-project', projectId: 'b', groupId: null, index: 1 },
    { type: 'move-project', projectId: 'e', groupId: 'g', index: 3 },
    { type: 'move-project', projectId: 'b', groupId: 'h', index: 1 },
    { type: 'move-project', projectId: 'b', groupId: 'g', index: 0 },
    { type: 'move-project', projectId: 'b', groupId: 'g', index: 2 },
    { type: 'move-project', projectId: 'e', groupId: null, index: 0 },
  ])
})

test('New Group… creates a group and then asks for its name', async () => {
  const wrapper = mountRail({ items: [{ kind: 'project', projectId: 'a' }] })
  wrapper.findAllComponents(ProjectContextMenu)[0]!.vm.$emit('rail', 'group-new', projects[0])
  const [change] = changes(wrapper)
  expect(change).toMatchObject({ type: 'create-group', projectId: 'a' })
  expect(document.body.textContent).not.toContain('Rename group')
  ;(change as Extract<RailChange, { type: 'create-group' }>).created!('new-group')
  await wrapper.setProps({ layout: { items: [group('new-group', ['a'])] } })
  await flushPromises()
  expect(document.body.textContent).toContain('Rename group')
  const input = document.getElementById('rename-group-name') as HTMLInputElement
  expect(input.placeholder).toBe('Alpha')
  input.value = '  Work  '
  input.dispatchEvent(new Event('input', { bubbles: true }))
  await nextTick()
  await nextTick()
  document
    .getElementById('rename-group-form')!
    .dispatchEvent(new Event('submit', { cancelable: true }))
  await flushPromises()
  expect(changes(wrapper)[1]).toEqual({ type: 'rename-group', groupId: 'new-group', name: 'Work' })
  expect(document.body.textContent).not.toContain('Rename group')
})

test('group menu actions become rail changes', async () => {
  const wrapper = mountRail({ items: [group('g', ['a', 'b'])] })
  const menu = wrapper.getComponent(GroupContextMenu)
  menu.vm.$emit('action', 'group-color', 'ansi-3')
  menu.vm.$emit('action', 'group-color', undefined)
  menu.vm.$emit('action', 'group-toggle')
  menu.vm.$emit('action', 'group-ungroup')
  menu.vm.$emit('action', 'group-rename')
  await flushPromises()
  expect(changes(wrapper)).toEqual([
    { type: 'color-group', groupId: 'g', color: 'ansi-3' },
    { type: 'color-group', groupId: 'g', color: null },
    { type: 'collapse-group', groupId: 'g', collapsed: false },
    { type: 'ungroup', groupId: 'g' },
  ])
  expect(document.body.textContent).toContain('Rename group')
})

test('the group menu action Mark Group as Read emits the group ID and no rail change', async () => {
  const wrapper = mountRail({ items: [group('g', ['a', 'b'])] })
  wrapper.getComponent(GroupContextMenu).vm.$emit('action', 'group-mark-read')
  await flushPromises()
  expect(wrapper.emitted('markGroupRead')).toEqual([['g']])
  expect(changes(wrapper)).toEqual([])
})

test('the group menu opens from a right click with planned and real items', async () => {
  const wrapper = mountRail({ items: [group('g', ['a', 'b'])] })
  await wrapper
    .get('[data-ui-id="rail-group-g"]')
    .trigger('contextmenu', { button: 2, clientX: 5, clientY: 5 })
  await flushPromises()
  const items = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')]
  const item = (text: string) => items.find((entry) => entry.textContent?.includes(text))
  expect(item('Mark Group as Read')?.getAttribute('aria-disabled')).toBeNull()
  expect(item('Mark Group as Read')?.textContent).not.toContain('Soon')
  expect(item('Rename Group')?.getAttribute('aria-disabled')).toBeNull()
  expect(item('Expand')).toBeDefined()
  expect(item('Ungroup')).toBeDefined()
  expect(item('Group Color')).toBeDefined()
})

test('the header of an expanded group opens the group menu with Collapse', async () => {
  const wrapper = mountRail({ items: [group('g', ['a', 'b'], { collapsed: false })] })
  await wrapper
    .get('[data-ui-id="rail-group-header-g"]')
    .trigger('contextmenu', { button: 2, clientX: 5, clientY: 5 })
  await flushPromises()
  const items = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].map((entry) =>
    entry.textContent?.trim(),
  )
  expect(items.some((text) => text?.startsWith('Collapse'))).toBe(true)
  expect(items.some((text) => text?.startsWith('Expand'))).toBe(false)
})

// Drag and drop. jsdom has no layout, so each tile reports a fixed rectangle.
const rects: Record<string, [number, number]> = {
  scroller: [0, 600],
  'project:a': [0, 48],
  'project:b': [56, 104],
  'project:c': [112, 160],
  'group:g': [112, 160],
}
beforeEach(() => {
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    const data = (this as HTMLElement).dataset ?? {}
    const key =
      data.uiId === 'rail-projects' ? 'scroller' : (data.railEntry ?? data.railMember ?? '')
    const [top, bottom] = rects[key] ?? [0, 0]
    return {
      top,
      bottom,
      left: 0,
      right: 72,
      width: 72,
      height: bottom - top,
      x: 0,
      y: top,
      toJSON: () => ({}),
    }
  })
})
const flat = {
  items: [
    { kind: 'project' as const, projectId: 'a' },
    { kind: 'project' as const, projectId: 'b' },
    { kind: 'project' as const, projectId: 'c' },
  ],
}
const pointer = (type: string, x: number, y: number, target: EventTarget = window) =>
  target.dispatchEvent(new MouseEvent(type, { bubbles: true, button: 0, clientX: x, clientY: y }))
const press = (wrapper: Rail, label: string, y: number) =>
  pointer('pointerdown', 10, y, wrapper.get(`[aria-label="${label}"]`).element)

test('a press that moves less than the threshold stays a click', async () => {
  const wrapper = mountRail(flat)
  press(wrapper, 'Alpha', 24)
  pointer('pointermove', 12, 26)
  pointer('pointerup', 12, 26)
  await wrapper.get('[aria-label="Alpha"]').trigger('click')
  expect(changes(wrapper)).toEqual([])
  expect(wrapper.emitted('project')).toEqual([['a']])
})

test('dropping a project on the middle of another makes a group, and the click after the drop is ignored', async () => {
  const wrapper = mountRail(flat)
  press(wrapper, 'Alpha', 24)
  pointer('pointermove', 10, 80)
  await nextTick()
  expect(wrapper.find('[data-testid="rail-drag-ghost"]').exists()).toBe(true)
  pointer('pointerup', 10, 80)
  await nextTick()
  expect(changes(wrapper)).toEqual([{ type: 'group-projects', draggedId: 'a', targetId: 'b' }])
  expect(wrapper.find('[data-testid="rail-drag-ghost"]').exists()).toBe(false)
  await wrapper.get('[aria-label="Beta"]').trigger('click')
  expect(wrapper.emitted('project')).toBeUndefined()
})

test('dropping between tiles reorders and shows a drop line while the pointer moves', async () => {
  const wrapper = mountRail(flat)
  press(wrapper, 'Alpha', 24)
  pointer('pointermove', 10, 150)
  await nextTick()
  expect(wrapper.find('[data-testid="rail-drop-line"]').exists()).toBe(true)
  pointer('pointerup', 10, 150)
  expect(changes(wrapper)).toEqual([
    { type: 'move-project', projectId: 'a', groupId: null, index: 2 },
  ])
})

test('a drop that changes nothing, or lands outside the rail, emits nothing', async () => {
  const wrapper = mountRail(flat)
  press(wrapper, 'Beta', 80)
  pointer('pointermove', 10, 100)
  await nextTick()
  expect(wrapper.find('[data-testid="rail-drop-line"]').exists()).toBe(false)
  pointer('pointerup', 10, 100)
  press(wrapper, 'Alpha', 24)
  pointer('pointermove', 400, 150)
  await nextTick()
  expect(wrapper.find('[data-testid="rail-drop-line"]').exists()).toBe(false)
  pointer('pointerup', 400, 150)
  expect(changes(wrapper)).toEqual([])
})

test('Escape cancels a drag', async () => {
  const wrapper = mountRail(flat)
  press(wrapper, 'Alpha', 24)
  pointer('pointermove', 10, 80)
  await nextTick()
  window.dispatchEvent(
    new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }),
  )
  await nextTick()
  expect(wrapper.find('[data-testid="rail-drag-ghost"]').exists()).toBe(false)
  pointer('pointerup', 10, 80)
  expect(changes(wrapper)).toEqual([])
})

test('dragging a group moves it in the top-level list', async () => {
  const wrapper = mountRail({
    items: [
      { kind: 'project', projectId: 'a' },
      { kind: 'project', projectId: 'b' },
      group('g', ['c']),
    ],
  })
  pointer('pointerdown', 10, 130, wrapper.get('[data-ui-id="rail-group-g"]').element)
  pointer('pointermove', 10, 60)
  pointer('pointerup', 10, 60)
  expect(changes(wrapper)).toEqual([{ type: 'move-group', groupId: 'g', index: 1 }])
})
