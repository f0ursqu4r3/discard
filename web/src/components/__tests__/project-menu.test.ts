import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import { nextTick } from 'vue'
import { groupMenu, projectMenu, type ProjectMenuEntry } from '../project-menu'
import ProjectContextMenu from '../ProjectContextMenu.vue'
import ProjectMenuItems from '../ProjectMenuItems.vue'
import ServerRail from '../ServerRail.vue'
import { ContextMenu, ContextMenuItem } from '../ui/context-menu'

const { loadMenu, pluginAction } = vi.hoisted(() => ({
  loadMenu: vi.fn(async () => [
    { kind: 'plugin', plugin: 'tools', index: 0, label: 'Inspect project' },
  ]),
  pluginAction: vi.fn(async () => {}),
}))
vi.mock('@/bridge/api', () => ({ contextMenu: loadMenu, pluginMenuAction: pluginAction }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
const first = { id: 'first', name: 'First', path: '/first' }
const second = { id: 'second', name: 'Second', path: '/second' }
const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => {
  wrappers.splice(0).forEach((wrapper) => wrapper.unmount())
  document.body.innerHTML = ''
  vi.clearAllMocks()
})
function flatten(entries: ProjectMenuEntry[]): ProjectMenuEntry[] {
  return entries.flatMap((entry) =>
    entry.kind === 'submenu' ? [entry, ...flatten(entry.children)] : [entry],
  )
}

test('all planned entries lack executable actions, including nested notification and move options', () => {
  const entries = flatten(projectMenu(false))
  const planned = entries.filter(
    (entry) => entry.kind === 'planned' || entry.kind === 'planned-radio',
  )
  expect(planned.length).toBeGreaterThan(15)
  for (const entry of planned) expect(entry).not.toHaveProperty('action')
  expect(entries).toContainEqual({
    kind: 'action',
    label: 'Mark Project as Read',
    action: 'mark-read',
  })
  expect(entries).toContainEqual({ kind: 'planned', label: 'New Channel…', control: undefined })
  expect(entries).toContainEqual({
    kind: 'planned-radio',
    labels: ['All Activity', 'Needs Attention', 'None'],
  })
})

test('busy projects block navigation and removal but retain copy and reveal', () => {
  const actions = flatten(projectMenu(true)).filter((entry) => entry.kind === 'action')
  // `group-none` is blocked by state: the project has no group.
  expect(actions.filter((entry) => entry.disabled).map((entry) => entry.action)).toEqual([
    'settings',
    'group-none',
    'remove',
  ])
  expect(actions.filter((entry) => !entry.disabled).map((entry) => entry.action)).toEqual([
    'mark-read',
    'open-window',
    'reveal',
    'group-new',
    'move-first',
    'move-last',
    'copy-name',
    'copy-path',
    'copy-id',
  ])
})

test('every rail project has its own target without selecting it', () => {
  const wrapper = mount(ServerRail, {
    props: {
      projects: [first, second],
      projectId: first.id,
      project: first.name,
      unread: false,
      pendingApprovals: 0,
    },
  })
  wrappers.push(wrapper)
  const menus = wrapper.findAllComponents(ProjectContextMenu)
  expect(menus.map((menu) => menu.props('project'))).toEqual([first, second])
  menus[1]!.vm.$emit('action', 'settings', second)
  expect(wrapper.emitted('projectAction')).toEqual([['settings', second]])
  expect(wrapper.emitted('project')).toBeUndefined()
  expect(
    wrapper.get('[data-ui-id="direct-messages-button"]').attributes('disabled'),
  ).toBeUndefined()
})

test('opening an inactive project menu preserves its target and disabled entries', async () => {
  const wrapper = mount(ProjectContextMenu, {
    attachTo: document.body,
    props: { project: second },
    slots: { default: '<button>Second</button>' },
  })
  wrappers.push(wrapper)
  await wrapper.get('button').trigger('contextmenu', { button: 2, clientX: 20, clientY: 20 })
  await flushPromises()
  expect(loadMenu).toHaveBeenCalledWith({ type: 'server', ...second })
  expect(wrapper.emitted('action')).toBeUndefined()
  const planned = [...document.querySelectorAll('[role="menuitem"]')].find((item) =>
    item.textContent?.includes('Add Agent'),
  )
  expect(planned?.getAttribute('aria-disabled')).toBe('true')
  expect(planned?.textContent).toContain('Soon')
  const newChannel = [...document.querySelectorAll('[role="menuitem"]')].find((item) =>
    item.textContent?.includes('New Channel'),
  )
  expect(newChannel?.getAttribute('aria-disabled')).toBe('true')
  expect(newChannel?.textContent).toContain('Soon')
  const renderer = wrapper.getComponent(ProjectMenuItems)
  renderer.vm.$emit('action', 'copy-path')
  expect(wrapper.emitted('action')).toEqual([['copy-path', second]])
  const plugin = wrapper
    .findAllComponents(ContextMenuItem)
    .find((item) => item.text().includes('Inspect project'))
  expect(plugin).toBeDefined()
  plugin!.vm.$emit('select')
  await flushPromises()
  expect(pluginAction).toHaveBeenCalledWith('tools', 0, { type: 'server', ...second })
})

test('remove requires confirmation and uses the clicked project', async () => {
  const wrapper = mount(ProjectContextMenu, {
    attachTo: document.body,
    props: { project: second },
    slots: { default: '<button>Second</button>' },
  })
  wrappers.push(wrapper)
  await wrapper.get('button').trigger('contextmenu', { button: 2 })
  await flushPromises()
  wrapper.getComponent(ProjectMenuItems).vm.$emit('action', 'remove')
  wrapper.getComponent(ContextMenu).vm.$emit('update:open', false)
  await nextTick()
  expect(wrapper.emitted('action')).toBeUndefined()
  expect(document.body.textContent).toContain('Remove Second?')
  const confirm = [...document.querySelectorAll('button')].find(
    (button) => button.textContent?.trim() === 'Remove project',
  )
  expect(confirm).toBeDefined()
  confirm!.click()
  await nextTick()
  expect(wrapper.emitted('action')).toEqual([['remove', second]])
})

test('Open in New Window reaches the rail while the menu is busy', async () => {
  const wrapper = mount(ProjectContextMenu, {
    attachTo: document.body,
    props: { project: second, busy: true },
    slots: { default: '<button>Second</button>' },
  })
  wrappers.push(wrapper)
  await wrapper.get('button').trigger('contextmenu', { button: 2 })
  await flushPromises()
  wrapper.getComponent(ProjectMenuItems).vm.$emit('action', 'open-window')
  expect(wrapper.emitted('action')).toEqual([['open-window', second]])
})

test('Shift+F10 opens a focused project menu and nested groups remain inspectable', async () => {
  const wrapper = mount(ProjectContextMenu, {
    attachTo: document.body,
    props: { project: second },
    slots: { default: '<button>Second</button>' },
  })
  wrappers.push(wrapper)
  await wrapper.get('button').trigger('keydown', { key: 'F10', shiftKey: true })
  await flushPromises()
  expect(loadMenu).toHaveBeenCalledWith({ type: 'server', ...second })
  const move = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((item) =>
    item.textContent?.includes('Move to'),
  )
  expect(move).toBeDefined()
  expect(move!.getAttribute('aria-disabled')).not.toBe('true')
  move!.focus()
  move!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
  await flushPromises()
  const group = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
    (item) => item.textContent?.trim() === 'Group',
  )
  expect(group).toBeDefined()
  group!.focus()
  group!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
  await flushPromises()
  const create = [...document.querySelectorAll('[role="menuitem"]')].find((item) =>
    item.textContent?.includes('New Group'),
  )
  expect(create?.getAttribute('aria-disabled')).toBeNull()
  expect(create?.textContent).not.toContain('Soon')
  const none = [...document.querySelectorAll('[role="menuitem"]')].find(
    (item) => item.textContent?.trim() === 'No Group',
  )
  expect(none?.getAttribute('aria-disabled')).toBe('true')
  expect(none?.textContent).not.toContain('Soon')
  expect(wrapper.emitted('action')).toBeUndefined()
})

const labels = (entries: ProjectMenuEntry[]) =>
  entries.map((entry) => ('label' in entry ? entry.label : entry.kind))
const groupSubmenu = (entries: ProjectMenuEntry[]) => {
  const move = entries.find((entry) => entry.kind === 'submenu' && entry.label === 'Move to…')
  const group =
    move?.kind === 'submenu'
      ? move.children.find((entry) => entry.kind === 'submenu' && entry.label === 'Group')
      : undefined
  return group?.kind === 'submenu' ? group.children : []
}

test('an ungrouped project cannot leave a group, and no group items show without groups', () => {
  const children = groupSubmenu(projectMenu(false))
  expect(labels(children)).toEqual(['No Group', 'separator', 'New Group…'])
  expect(children[0]).toMatchObject({ action: 'group-none', disabled: true })
  expect(children[2]).toMatchObject({ action: 'group-new' })
  expect(children[2]).not.toHaveProperty('disabled')
})

test('the group submenu lists each group, blocks the current one, and enables No Group', () => {
  const rail = {
    groups: [
      { id: 'g1', label: 'Work' },
      { id: 'g2', label: 'Play' },
    ],
    groupId: 'g2',
  }
  const children = groupSubmenu(projectMenu(false, rail))
  expect(labels(children)).toEqual([
    'No Group',
    'separator',
    'Work',
    'Play',
    'separator',
    'New Group…',
  ])
  expect(children[0]).toMatchObject({ action: 'group-none', disabled: false })
  expect(children[2]).toMatchObject({ action: 'group-join', arg: 'g1', disabled: false })
  expect(children[3]).toMatchObject({ action: 'group-join', arg: 'g2', disabled: true })
})

test('First and Last in Group are real actions', () => {
  const move = flatten(projectMenu(false)).filter(
    (entry) => entry.kind === 'action' && ['move-first', 'move-last'].includes(entry.action),
  )
  expect(move.map((entry) => entry.kind === 'action' && entry.label)).toEqual([
    'First in Group',
    'Last in Group',
  ])
})

test('the group menu has real actions and swatches', () => {
  const entries = flatten(groupMenu(false))
  expect(entries.some((entry) => entry.kind === 'planned')).toBe(false)
  expect(
    entries.filter((entry) => entry.kind === 'action').map((entry) => [entry.label, entry.action]),
  ).toEqual([
    ['Mark Group as Read', 'group-mark-read'],
    ['Rename Group…', 'group-rename'],
    ['Default', 'group-color'],
    ['Red', 'group-color'],
    ['Green', 'group-color'],
    ['Yellow', 'group-color'],
    ['Blue', 'group-color'],
    ['Magenta', 'group-color'],
    ['Cyan', 'group-color'],
    ['Collapse', 'group-toggle'],
    ['Ungroup', 'group-ungroup'],
  ])
  expect(entries).toContainEqual(
    expect.objectContaining({ label: 'Blue', arg: 'ansi-4', swatch: 'ansi-4' }),
  )
  expect(entries).toContainEqual(
    expect.not.objectContaining({ label: 'Default', arg: expect.anything() }),
  )
  expect(labels(groupMenu(true))).toContain('Expand')
})

test('Open in New Window sits directly above Reveal in Finder and is always enabled', () => {
  for (const busy of [false, true]) {
    const labels = projectMenu(busy).map((entry) => ('label' in entry ? entry.label : entry.kind))
    expect(labels.indexOf('Open in New Window')).toBe(labels.indexOf('Reveal in Finder') - 1)
    const entry = projectMenu(busy).find(
      (item) => item.kind === 'action' && item.action === 'open-window',
    )
    expect(entry).toMatchObject({ kind: 'action', label: 'Open in New Window' })
    expect(entry).not.toHaveProperty('disabled')
  }
})
