import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import NavigationContextMenu from './NavigationContextMenu.vue'
import ChannelList from './ChannelList.vue'
import { ContextMenuItem } from './ui/context-menu'

const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => {
  wrappers.splice(0).forEach((wrapper) => wrapper.unmount())
  document.body.innerHTML = ''
  localStorage.clear()
  vi.unstubAllGlobals()
})
for (const kind of ['branch', 'worktree', 'channel'] as const) {
  test(`${kind} menu targets its row without selecting it`, async () => {
    const copy = vi.fn().mockResolvedValue(undefined)
    vi.stubGlobal('navigator', {
      userAgent: navigator.userAgent,
      platform: navigator.platform,
      clipboard: { writeText: copy },
    })
    const wrapper = mount(NavigationContextMenu, {
      attachTo: document.body,
      props: { kind, name: 'feature', id: 'target-id', path: '/target-worktree' },
      slots: { default: '<button>Target</button>' },
    })
    wrappers.push(wrapper)
    await wrapper.get('button').trigger('contextmenu', { button: 2, clientX: 20, clientY: 20 })
    await flushPromises()
    expect(document.querySelector(`[data-ui-id="${kind}-context-menu"]`)).not.toBeNull()
    expect(wrapper.emitted('open')).toBeUndefined()
    const items = wrapper.findAllComponents(ContextMenuItem)
    const planned = items.filter((item) => item.text().includes('Soon'))
    expect(planned.length).toBeGreaterThan(0)
    for (const item of planned) {
      expect(item.attributes('aria-disabled')).toBe('true')
      item.vm.$emit('select')
    }
    expect(wrapper.emitted('open')).toBeUndefined()
    items.find((item) => item.text().endsWith('Name'))!.vm.$emit('select')
    items.find((item) => item.text().endsWith('ID'))!.vm.$emit('select')
    if (kind === 'worktree')
      items.find((item) => item.text() === 'Copy Worktree Path')!.vm.$emit('select')
    await flushPromises()
    expect(copy).toHaveBeenCalledWith('feature')
    expect(copy).toHaveBeenCalledWith('target-id')
    if (kind === 'worktree') expect(copy).toHaveBeenCalledWith('/target-worktree')
    items.find((item) => item.text().startsWith('Open'))!.vm.$emit('select')
    expect(wrapper.emitted('open')).toEqual([[]])
  })
}

test('keyboard menus block opening while busy but allow copying', async () => {
  const wrapper = mount(NavigationContextMenu, {
    attachTo: document.body,
    props: { kind: 'channel', name: 'general', id: 'general-id', busy: true },
    slots: { default: '<button>General</button>' },
  })
  wrappers.push(wrapper)
  await wrapper.get('button').trigger('keydown', { key: 'F10', shiftKey: true })
  await flushPromises()
  const items = wrapper.findAllComponents(ContextMenuItem)
  const open = items.find((item) => item.text() === 'Open Channel')!
  expect(open.attributes('aria-disabled')).toBe('true')
  open.vm.$emit('select')
  expect(wrapper.emitted('open')).toBeUndefined()
  expect(
    items.find((item) => item.text() === 'Copy Channel Name')!.attributes('aria-disabled'),
  ).not.toBe('true')
})

test('detached worktrees are selectable and every sidebar row has a target menu', async () => {
  const key = JSON.stringify(['', 'abcdef1', '/detached'])
  const base = { depth: 0, current: false, collapsed: false, hasChildren: false }
  const channel = {
    projectId: 'p',
    agentId: 'a',
    workspaceMode: 'worktree',
    archived: false,
    archivedInto: null,
    branch: null,
  }
  const wrapper = mount(ChannelList, {
    props: {
      projectId: 'p',
      projectPath: '/repo',
      branches: [
        { ...base, key: 'main', label: 'main', kind: 'main' },
        { ...base, key, label: 'abcdef1', kind: 'worktree', detached: true, worktree: '/detached' },
      ],
      projectChannels: [
        { ...channel, id: 'general', name: 'general', kind: 'general' },
        { ...channel, id: 'saved', name: 'saved', kind: 'legacy' },
      ],
    },
  })
  wrappers.push(wrapper)
  const detached = wrapper
    .findAll('[data-ui-id^="worktrees-row-"]')
    .find((row) => row.text().includes('abcdef1'))!
  expect(detached.attributes('disabled')).toBeUndefined()
  await detached.trigger('click')
  expect(wrapper.emitted('openBranch')).toEqual([[key]])
  await wrapper.get('[data-ui-id="saved-channel-toggle"]').trigger('click')
  const menus = wrapper.findAllComponents(NavigationContextMenu)
  expect(menus.map((menu) => [menu.props('kind'), menu.props('id')])).toEqual([
    ['channel', 'general'],
    ['branch', 'main'],
    ['worktree', 'main'],
    ['worktree', key],
    ['channel', 'saved'],
  ])
  menus.find((menu) => menu.props('id') === 'saved')!.vm.$emit('open')
  expect(wrapper.emitted('selectChannel')).toEqual([['saved']])
})
