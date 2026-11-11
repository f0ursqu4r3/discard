import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import NavigationContextMenu from '../NavigationContextMenu.vue'
import ChannelList from '../ChannelList.vue'
import { ContextMenuItem } from '../ui/context-menu'

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
    // The seven actions are implemented. No item of these menus is a planned item.
    expect(items.filter((item) => item.text().includes('Soon'))).toEqual([])
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
    hasMessages: false,
    branch: null,
  }
  const wrapper = mount(ChannelList, {
    props: {
      projectId: 'p',
      projectPath: '/repo',
      showArchived: true,
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
  await wrapper.get('[data-ui-id="archived-channel-toggle"]').trigger('click')
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

const texts = () =>
  [...document.querySelectorAll('[role="menuitem"]')].map((item) => item.textContent?.trim())
async function openMenu(props: Record<string, unknown>) {
  const wrapper = mount(NavigationContextMenu, {
    attachTo: document.body,
    props: { name: 'feature', id: 'target-id', ...props } as never,
    slots: { default: '<button>Target</button>' },
  })
  wrappers.push(wrapper)
  await wrapper.get('button').trigger('contextmenu', { button: 2, clientX: 20, clientY: 20 })
  await flushPromises()
  return wrapper
}
const item = (wrapper: ReturnType<typeof mount>, label: string) =>
  wrapper.findAllComponents(ContextMenuItem).find((entry) => entry.text() === label)!
const disabled = (wrapper: ReturnType<typeof mount>, label: string) =>
  item(wrapper, label).attributes('aria-disabled') === 'true'

test('the channel menu offers Mute Channel, then Unmute Channel, and Archive Channel…', async () => {
  const wrapper = await openMenu({ kind: 'channel', renamable: true })
  expect(texts().slice(-3)).toEqual(['Mute Channel', 'Rename Channel…', 'Archive Channel…'])
  expect(disabled(wrapper, 'Mute Channel')).toBe(false)
  expect(disabled(wrapper, 'Archive Channel…')).toBe(false)
  item(wrapper, 'Mute Channel').vm.$emit('select')
  item(wrapper, 'Archive Channel…').vm.$emit('select')
  expect(wrapper.emitted('mute')).toEqual([[]])
  expect(wrapper.emitted('archive')).toEqual([[]])
  await wrapper.setProps({ muted: true })
  expect(texts().slice(-3)).toEqual(['Unmute Channel', 'Rename Channel…', 'Archive Channel…'])
})

test('a blocked Archive Channel… is disabled, has no Soon label and does not run', async () => {
  const wrapper = await openMenu({ kind: 'channel', archiveDisabled: true })
  expect(disabled(wrapper, 'Archive Channel…')).toBe(true)
  expect(item(wrapper, 'Archive Channel…').text()).not.toContain('Soon')
  item(wrapper, 'Archive Channel…').vm.$emit('select')
  expect(wrapper.emitted('archive')).toBeUndefined()
  expect(disabled(wrapper, 'Mute Channel')).toBe(false)
})

test('a channel with no ID cannot be muted or archived', async () => {
  const wrapper = await openMenu({ kind: 'channel', id: undefined })
  expect(disabled(wrapper, 'Mute Channel')).toBe(true)
  expect(disabled(wrapper, 'Archive Channel…')).toBe(true)
  item(wrapper, 'Mute Channel').vm.$emit('select')
  expect(wrapper.emitted('mute')).toBeUndefined()
})

test('the branch menu has Create Worktree…, Rename Branch… and Delete Branch… that emit when enabled', async () => {
  const wrapper = await openMenu({ kind: 'branch' })
  expect(texts().slice(-3)).toEqual(['Create Worktree…', 'Rename Branch…', 'Delete Branch…'])
  expect(document.body.textContent).not.toContain('Soon')
  for (const label of ['Create Worktree…', 'Rename Branch…', 'Delete Branch…']) {
    expect(disabled(wrapper, label)).toBe(false)
    item(wrapper, label).vm.$emit('select')
  }
  expect(wrapper.emitted('createWorktree')).toEqual([[]])
  expect(wrapper.emitted('renameBranch')).toEqual([[]])
  expect(wrapper.emitted('deleteBranch')).toEqual([[]])
})

test('each blocked branch action is disabled with no Soon label and does not emit', async () => {
  const wrapper = await openMenu({
    kind: 'branch',
    createWorktreeDisabled: true,
    renameBranchDisabled: true,
    deleteBranchDisabled: true,
  })
  for (const label of ['Create Worktree…', 'Rename Branch…', 'Delete Branch…']) {
    expect(disabled(wrapper, label)).toBe(true)
    expect(item(wrapper, label).text()).not.toContain('Soon')
    item(wrapper, label).vm.$emit('select')
  }
  expect(wrapper.emitted('createWorktree')).toBeUndefined()
  expect(wrapper.emitted('renameBranch')).toBeUndefined()
  expect(wrapper.emitted('deleteBranch')).toBeUndefined()
})

test('each branch action is blocked on its own rule', async () => {
  const wrapper = await openMenu({ kind: 'branch', deleteBranchDisabled: true })
  expect(disabled(wrapper, 'Delete Branch…')).toBe(true)
  expect(disabled(wrapper, 'Create Worktree…')).toBe(false)
  expect(disabled(wrapper, 'Rename Branch…')).toBe(false)
})

test('the worktree menu has Create Branch… and Remove Worktree… and blocks each on its own rule', async () => {
  const wrapper = await openMenu({ kind: 'worktree', path: '/tree' })
  expect(texts().slice(-2)).toEqual(['Create Branch…', 'Remove Worktree…'])
  expect(document.body.textContent).not.toContain('Soon')
  item(wrapper, 'Create Branch…').vm.$emit('select')
  item(wrapper, 'Remove Worktree…').vm.$emit('select')
  expect(wrapper.emitted('createBranch')).toEqual([[]])
  expect(wrapper.emitted('removeWorktree')).toEqual([[]])
  await wrapper.setProps({ removeWorktreeDisabled: true })
  expect(disabled(wrapper, 'Remove Worktree…')).toBe(true)
  expect(disabled(wrapper, 'Create Branch…')).toBe(false)
  item(wrapper, 'Remove Worktree…').vm.$emit('select')
  expect(wrapper.emitted('removeWorktree')).toHaveLength(1)
  await wrapper.setProps({ createBranchDisabled: true })
  expect(disabled(wrapper, 'Create Branch…')).toBe(true)
})
