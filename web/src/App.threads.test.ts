import { shallowMount, flushPromises } from '@vue/test-utils'
import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import thread from './bridge/fixtures/thread.json'
import type { MessageDto, OpenThreadDto, SnapshotDto, ThreadHistoryDto } from './bridge/types'
const mocks = vi.hoisted(() => ({
  store: null as unknown as {
    snapshot: SnapshotDto | null
    history: MessageDto[]
    threadHistory: ThreadHistoryDto | null
    refresh: ReturnType<typeof vi.fn>
  },
  api: {
    sendMessage: vi.fn(async () => {}),
    startThread: vi.fn(async () => 'thread'),
    openThread: vi.fn(async () => {}),
    closeThread: vi.fn(async () => {}),
    selectThread: vi.fn(async () => 'channel'),
    leaveThread: vi.fn(async () => {}),
    cancelTurn: vi.fn(async () => {}),
    setPanels: vi.fn<(channelList: number, memberList: number) => Promise<void>>(async () => {}),
  },
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: () => false }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('./bridge/store', async () => {
  const { reactive } = await import('vue')
  mocks.store = reactive({
    snapshot: null,
    history: [],
    threadHistory: null,
    refresh: vi.fn(async () => {}),
  }) as unknown as typeof mocks.store
  return { useStore: () => mocks.store }
})
vi.mock('./bridge/shortcuts', () => ({ useWebShortcuts: vi.fn() }))
vi.mock('./bridge/api', async (original) => ({
  ...(await original<object>()),
  ...mocks.api,
  getProjects: vi.fn(async () => []),
  getProjectChannels: vi.fn(async () => []),
}))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() } }))
import App from './App.vue'
import ChannelHeader from './components/ChannelHeader.vue'
import Composer from './components/Composer.vue'
import MessageList from './components/MessageList.vue'
import NavigationPanel from './components/workspace/NavigationPanel.vue'
import ContextPanel from './components/workspace/ContextPanel.vue'
import ThreadPanel from './components/workspace/ThreadPanel.vue'
import { ResizableHandle } from './components/ui/resizable'
import { Sheet } from './components/ui/sheet'

const fixture = thread.snapshot as SnapshotDto
const openThread = fixture.openThread as OpenThreadDto
const anchor = (thread.history as { messages: MessageDto[] }).messages[0]!
const plain: MessageDto = {
  id: 'plain',
  role: 'agent',
  continuesGroup: false,
  blocks: [{ type: 'markdown', text: 'No thread yet.' }],
}
let view: ReturnType<typeof shallowMount> | undefined
let wide = true
beforeEach(() => {
  vi.clearAllMocks()
  wide = true
  mocks.store.snapshot = { ...structuredClone(fixture), openThread: null }
  mocks.store.history = [plain]
  mocks.store.threadHistory = null
  vi.stubGlobal(
    'matchMedia',
    vi.fn((query: string) => ({
      matches: wide && query.includes('min-width: 1280px'),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  )
})
afterEach(() => {
  view?.unmount()
  vi.unstubAllGlobals()
})
function app() {
  view = shallowMount(App, {
    global: {
      renderStubDefaultSlot: true,
      stubs: {
        ResizablePanelGroup: {
          name: 'GroupStub',
          template: '<div><slot /></div>',
          emits: ['layout'],
        },
        ResizablePanel: { template: '<div><slot /></div>' },
      },
    },
  })
  return view
}
// The narrow-screen sheet always renders its stub content. Only the panel beside the chat counts.
const sideContext = (wrapper: ReturnType<typeof app>) =>
  wrapper.findAllComponents(ContextPanel).filter((panel) => !panel.element.closest('sheet-stub'))
const open = (patch: Partial<OpenThreadDto> = {}) => {
  mocks.store.snapshot = { ...mocks.store.snapshot!, openThread: { ...openThread, ...patch } }
  mocks.store.threadHistory = thread.threadHistory as ThreadHistoryDto
}

test('Start thread opens a draft without a backend call and the first send creates the thread', async () => {
  const wrapper = app()
  expect(wrapper.findComponent(ThreadPanel).exists()).toBe(false)
  wrapper.getComponent(MessageList).vm.$emit('start-thread', plain)
  await flushPromises()
  const panel = wrapper.getComponent(ThreadPanel)
  expect(panel.props('thread')).toBeNull()
  expect(panel.props('draftAnchor')).toMatchObject({ id: 'plain' })
  expect(panel.props('parentChannelId')).toBe('channel')
  expect(mocks.api.startThread).not.toHaveBeenCalled()
  expect(mocks.api.openThread).not.toHaveBeenCalled()
  mocks.api.startThread.mockImplementationOnce(async () => {
    open()
    return 'thread'
  })
  panel.vm.$emit('send', 'Why?', [{ id: 'file' }], true)
  await flushPromises()
  expect(mocks.api.startThread).toHaveBeenCalledTimes(1)
  expect(mocks.api.startThread).toHaveBeenCalledWith('channel', 'plain', 'Why?', ['file'], true)
  expect(mocks.api.sendMessage).not.toHaveBeenCalled()
  expect(wrapper.getComponent(ThreadPanel).props('thread')).toMatchObject({ channelId: 'thread' })
  expect(wrapper.getComponent(ThreadPanel).props('draftAnchor')).toBeUndefined()
})

test('a failed first send keeps the draft and its text', async () => {
  const wrapper = app()
  wrapper.getComponent(MessageList).vm.$emit('start-thread', plain)
  await flushPromises()
  mocks.api.startThread.mockRejectedValueOnce(new Error('anchor already has a thread'))
  const panel = wrapper.getComponent(ThreadPanel)
  panel.vm.$emit('update:draft', 'Why?')
  panel.vm.$emit('send', 'Why?', [], false)
  await flushPromises()
  expect(wrapper.getComponent(ThreadPanel).props('thread')).toBeNull()
  expect(wrapper.getComponent(ThreadPanel).props('draft')).toBe('Why?')
})

test('closing the draft discards it and its text', async () => {
  const wrapper = app()
  wrapper.getComponent(MessageList).vm.$emit('start-thread', plain)
  await flushPromises()
  wrapper.getComponent(ThreadPanel).vm.$emit('update:draft', 'Unsent')
  await flushPromises()
  expect(wrapper.getComponent(ThreadPanel).props('draft')).toBe('Unsent')
  wrapper.getComponent(ThreadPanel).vm.$emit('close')
  await flushPromises()
  expect(wrapper.findComponent(ThreadPanel).exists()).toBe(false)
  expect(mocks.api.startThread).not.toHaveBeenCalled()
  expect(mocks.api.closeThread).not.toHaveBeenCalled()
  wrapper.getComponent(MessageList).vm.$emit('start-thread', plain)
  await flushPromises()
  expect(wrapper.getComponent(ThreadPanel).props('draft')).toBeUndefined()
})

test('switching channels closes the draft', async () => {
  const wrapper = app()
  wrapper.getComponent(MessageList).vm.$emit('start-thread', plain)
  await flushPromises()
  mocks.store.snapshot = { ...mocks.store.snapshot!, channelId: 'other' }
  await flushPromises()
  expect(wrapper.findComponent(ThreadPanel).exists()).toBe(false)
})

test('a read-only channel cannot start a thread', async () => {
  mocks.store.snapshot = { ...mocks.store.snapshot!, composerDisabled: true }
  const wrapper = app()
  expect(wrapper.getComponent(MessageList).props('threadDisabled')).toBe(true)
  wrapper.getComponent(MessageList).vm.$emit('start-thread', plain)
  await flushPromises()
  expect(wrapper.findComponent(ThreadPanel).exists()).toBe(false)
})

test('starting a draft while a thread is open closes the open thread', async () => {
  open()
  const wrapper = app()
  expect(wrapper.getComponent(ThreadPanel).props('thread')).toMatchObject({ channelId: 'thread' })
  wrapper.getComponent(MessageList).vm.$emit('start-thread', plain)
  await flushPromises()
  expect(mocks.api.closeThread).toHaveBeenCalledTimes(1)
  expect(wrapper.getComponent(ThreadPanel).props('thread')).toBeNull()
  expect(wrapper.getComponent(ThreadPanel).props('draftAnchor')).toMatchObject({ id: 'plain' })
})

test('the chip opens its thread', async () => {
  const wrapper = app()
  wrapper.getComponent(MessageList).vm.$emit('open-thread', 'thread')
  await flushPromises()
  expect(mocks.api.openThread).toHaveBeenCalledWith('thread')
  expect(mocks.store.refresh).toHaveBeenCalled()
})

test('an opened thread replaces the draft', async () => {
  const wrapper = app()
  wrapper.getComponent(MessageList).vm.$emit('start-thread', plain)
  await flushPromises()
  open()
  await flushPromises()
  expect(wrapper.getComponent(ThreadPanel).props('thread')).toMatchObject({ channelId: 'thread' })
})

test('the thread panel replaces the context panel and closing restores it', async () => {
  const wrapper = app()
  expect(sideContext(wrapper)).toHaveLength(1)
  expect(wrapper.findComponent(ThreadPanel).exists()).toBe(false)
  open()
  await flushPromises()
  expect(wrapper.findComponent(ThreadPanel).exists()).toBe(true)
  expect(sideContext(wrapper)).toHaveLength(0)
  wrapper.getComponent(ThreadPanel).vm.$emit('close')
  await flushPromises()
  expect(mocks.api.closeThread).toHaveBeenCalledTimes(1)
  mocks.store.snapshot = { ...mocks.store.snapshot!, openThread: null }
  await flushPromises()
  expect(wrapper.findComponent(ThreadPanel).exists()).toBe(false)
  expect(sideContext(wrapper)).toHaveLength(1)
})

test('the thread panel shows even when the context panel is hidden', async () => {
  mocks.store.snapshot = { ...mocks.store.snapshot!, memberListVisible: false }
  const wrapper = app()
  expect(sideContext(wrapper)).toHaveLength(0)
  open()
  await flushPromises()
  expect(wrapper.findComponent(ThreadPanel).exists()).toBe(true)
})

test('resizing the thread panel does not overwrite the saved context panel width', async () => {
  open()
  const wrapper = app()
  await flushPromises()
  wrapper.getComponent({ name: 'GroupStub' }).vm.$emit('layout', [300, 700, 520])
  const handle = wrapper
    .findAllComponents(ResizableHandle)
    .find((item) => item.attributes('aria-label') === 'Resize thread')!
  handle.vm.$emit('dragging', false)
  await flushPromises()
  expect(mocks.api.setPanels).toHaveBeenCalledTimes(1)
  expect(mocks.api.setPanels.mock.calls[0]![1]).toBe(fixture.panels.memberList)
})

test('on a narrow screen the thread shows in the right sheet', async () => {
  wide = false
  open()
  const wrapper = app()
  await flushPromises()
  const sheets = wrapper.findAllComponents(Sheet)
  const threadSheet = sheets.find((sheet) => sheet.findComponent(ThreadPanel).exists())!
  expect(threadSheet.props('open')).toBe(true)
  expect(sheets.filter((sheet) => sheet.props('open'))).toHaveLength(1)
  threadSheet.vm.$emit('update:open', false)
  await flushPromises()
  expect(mocks.api.closeThread).toHaveBeenCalledTimes(1)
})

test('sending in an open thread uses the thread channel and the also-post choice', async () => {
  open()
  const wrapper = app()
  wrapper.getComponent(ThreadPanel).vm.$emit('send', 'More', [{ id: 'f' }], true)
  await flushPromises()
  expect(mocks.api.sendMessage).toHaveBeenCalledWith('thread', 'More', ['f'], true)
  expect(mocks.api.startThread).not.toHaveBeenCalled()
})

test('Stop in the thread cancels the thread turn, not the channel turn', async () => {
  open({ turnRunning: true })
  const wrapper = app()
  wrapper.getComponent(ThreadPanel).vm.$emit('stop')
  await flushPromises()
  expect(mocks.api.cancelTurn).toHaveBeenCalledTimes(1)
  expect(mocks.api.cancelTurn).toHaveBeenCalledWith('thread')
})

test('quoting a thread message appends to the thread draft and leaves the channel draft alone', async () => {
  open()
  const wrapper = app()
  const reply = (thread.threadHistory as ThreadHistoryDto).messages[1]!
  wrapper.getComponent(ThreadPanel).vm.$emit('quote', reply)
  await flushPromises()
  const draft = wrapper.getComponent(ThreadPanel).props('draft')
  expect(draft).toContain('> [Message from Agent](#discard-message/thread/reply)')
  expect(draft).toContain('It keeps the channel clean.')
  expect(wrapper.getComponent(Composer).props('draft')).toBeUndefined()
})

test('quoting the anchor links to its parent channel', async () => {
  open()
  const wrapper = app()
  wrapper.getComponent(ThreadPanel).vm.$emit('quote', anchor)
  await flushPromises()
  expect(wrapper.getComponent(ThreadPanel).props('draft')).toContain(
    '(#discard-message/channel/answer)',
  )
})

test('a quote link to a thread message scrolls the thread panel', async () => {
  open()
  const wrapper = app()
  const link = document.createElement('a')
  link.href = '#discard-message/thread/ask'
  wrapper.getComponent(ThreadPanel).element.parentElement!.append(link)
  await link.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
  await flushPromises()
  expect(wrapper.getComponent(ThreadPanel).props('jumpTo')).toMatchObject({ messageId: 'ask' })
})

test('a thread row in the channel bar selects its thread, and Leave leaves it', async () => {
  const wrapper = app()
  const bar = wrapper.getComponent(NavigationPanel)
  bar.vm.$emit('select-thread', 'thread')
  await flushPromises()
  expect(mocks.api.selectThread).toHaveBeenCalledWith('thread')
  expect(mocks.store.refresh).toHaveBeenCalled()
  bar.vm.$emit('leave-thread', 'thread')
  await flushPromises()
  expect(mocks.api.leaveThread).toHaveBeenCalledWith('thread')
})

test('the header project context button closes the thread panel', async () => {
  open()
  const wrapper = app()
  expect(wrapper.getComponent(ChannelHeader).props('memberListVisible')).toBe(false)
  wrapper.getComponent(ChannelHeader).vm.$emit('toggle-members')
  await flushPromises()
  expect(mocks.api.closeThread).toHaveBeenCalledTimes(1)
})
