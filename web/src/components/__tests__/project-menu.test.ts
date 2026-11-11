import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import { nextTick } from 'vue'
import { projectMenu, type ProjectMenuEntry } from '../project-menu'
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
  expect(planned.length).toBeGreaterThan(20)
  for (const entry of planned) expect(entry).not.toHaveProperty('action')
  expect(entries).toContainEqual({ kind: 'planned', label: 'New Group…', control: undefined })
  expect(entries).toContainEqual({ kind: 'planned', label: 'New Channel…', control: undefined })
  expect(entries).toContainEqual({
    kind: 'planned-radio',
    labels: ['All Activity', 'Needs Attention', 'None'],
  })
})

test('busy projects block navigation and removal but retain copy and reveal', () => {
  const actions = flatten(projectMenu(true)).filter((entry) => entry.kind === 'action')
  expect(actions.filter((entry) => entry.disabled).map((entry) => entry.action)).toEqual([
    'settings',
    'remove',
  ])
  expect(actions.filter((entry) => !entry.disabled).map((entry) => entry.action)).toEqual([
    'reveal',
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
  expect(create?.getAttribute('aria-disabled')).toBe('true')
  expect(wrapper.emitted('action')).toBeUndefined()
})
