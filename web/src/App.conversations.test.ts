import { shallowMount, flushPromises } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import AgentsNavigation from './components/workspace/AgentsNavigation.vue'
import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import fixture from './bridge/fixtures/snapshot.json'
import type { MessageDto, SnapshotDto } from './bridge/types'
const mocks = vi.hoisted(() => ({
  store: {
    snapshot: null as SnapshotDto | null,
    history: [] as MessageDto[],
    refresh: vi.fn(async () => {}),
  },
  send: vi.fn(async () => {}),
  copy: vi.fn(async () => {}),
  run: vi.fn(),
}))
const catalogHandlers = new Map<string, Set<() => void>>()
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: () => false }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: () => void) => {
    if (!catalogHandlers.has(event)) catalogHandlers.set(event, new Set())
    catalogHandlers.get(event)!.add(handler)
    return () => catalogHandlers.get(event)?.delete(handler)
  }),
}))
vi.mock('./bridge/store', () => ({ useStore: () => mocks.store }))
vi.mock('./bridge/shortcuts', () => ({ useWebShortcuts: vi.fn() }))
vi.mock('./bridge/api', async (original) => ({
  ...(await original<object>()),
  getProjects: vi.fn(async () => []),
  sendMessage: mocks.send,
  runAction: mocks.run,
}))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }))
import App from './App.vue'
import ChannelHeader from './components/ChannelHeader.vue'
import AgentHistorySearch from './components/agents/AgentHistorySearch.vue'
import ContextPanel from './components/workspace/ContextPanel.vue'
import FriendsList from './components/agents/FriendsList.vue'
import AddAgentDialog from './components/agents/AddAgentDialog.vue'
import AgentProfileDialog from './components/agents/AgentProfileDialog.vue'
import Composer from './components/Composer.vue'
import MessageList from './components/MessageList.vue'
import NavigationPanel from './components/workspace/NavigationPanel.vue'
import CreateChannelDialog from './components/workspace/CreateChannelDialog.vue'
import MemoryBrowser from './components/memory/MemoryBrowser.vue'
import { Button } from './components/ui/button'
let view: ReturnType<typeof shallowMount> | undefined
beforeEach(() => {
  vi.clearAllMocks()
  catalogHandlers.clear()
  mocks.store.snapshot = {
    ...structuredClone(fixture),
    projectId: '',
    project: '',
    projectPath: '',
    activeProjectId: null,
    composerDisabled: false,
    turnRunning: false,
    liveBlocks: [],
  }
  mocks.store.history = []
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
  )
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: mocks.copy },
  })
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
        ResizablePanelGroup: { template: '<div><slot /></div>' },
        ResizablePanel: { template: '<div><slot /></div>' },
      },
    },
  })
  return view
}
test('a valid DM opens with no project and uses contact navigation', () => {
  const wrapper = app()
  expect(wrapper.find('[data-ui-id="main-panel"]').exists()).toBe(true)
  expect(wrapper.getComponent(Composer).props('archived')).toBe(false)
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('direct-messages')
})
test('fresh startup has contact navigation and no active composer', () => {
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    channelId: '',
    agentId: '',
    composerDisabled: true,
  }
  const wrapper = app()
  expect(wrapper.findComponent(NavigationPanel).exists()).toBe(true)
  expect(wrapper.findComponent(Composer).exists()).toBe(false)
  expect(wrapper.findComponent(FriendsList).exists()).toBe(true)
})
test('DM new-conversation context stays direct while a sidebar project is selected', () => {
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    projectId: 'selected-project',
    project: 'Project',
  }
  const wrapper = app()
  expect(wrapper.getComponent(CreateChannelDialog).props('projectId')).toBeNull()
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('direct-messages')
})
test('retained deleted-agent history disables its composer and keeps history readable', () => {
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    agent: 'Deleted agent',
    composerDisabled: true,
  }
  const wrapper = app()
  expect(wrapper.getComponent(Composer).props('archived')).toBe(true)
  expect(wrapper.text()).toContain('Read only')
})
test('model failure offers retry and model settings; failed persistence offers copy first', async () => {
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    notice: 'Reply was not saved. Copy the reply before sending again.',
    liveBlocks: [{ type: 'markdown', text: 'Unsaved answer' }],
  }
  mocks.store.history = [
    {
      id: 'user',
      role: 'user',
      blocks: [{ type: 'markdown', text: 'Question' }],
      continuesGroup: false,
    },
  ]
  const wrapper = app()
  const buttons = () => wrapper.findAllComponents(Button)
  await buttons()
    .find((button) => button.text() === 'Copy reply before sending again')!
    .trigger('click')
  await flushPromises()
  expect(mocks.copy).toHaveBeenCalledWith('Unsaved answer')
  await buttons()
    .find((button) => button.text() === 'Retry last message')!
    .trigger('click')
  await flushPromises()
  expect(mocks.send).toHaveBeenCalledWith('channel', 'Question')
  await buttons()
    .find((button) => button.text() === 'Choose model')!
    .trigger('click')
  await flushPromises()
  expect(wrapper.getComponent(AgentProfileDialog).props('agentId')).toBe(
    mocks.store.snapshot!.agentId,
  )
  expect(mocks.run).not.toHaveBeenCalledWith('open_agent_settings')
})

test('top-level menu creation refreshes the same-context conversation catalog', async () => {
  let created = false
  const agent = { id: 'a', name: 'Robin', provider: 'codex', model: 'exact', archived: false }
  vi.mocked(invoke).mockImplementation(async (name, payload) => {
    if (name === 'settings_agent_get') return [agent]
    if (name === 'get_conversations')
      return created
        ? [
            {
              id: 'new-menu-chat',
              inDmList: true,
              isDefault: false,
              name: 'New menu discussion',
              agentId: 'a',
              agent: 'Robin',
              projectId: null,
              archived: false,
              agentArchived: false,
              deleted: false,
            },
          ]
        : []
    if (name === 'create_conversation') {
      expect(payload).toEqual({ agentId: 'a', projectId: null, name: 'New menu discussion' })
      created = true
      catalogHandlers.get('discard://conversations')?.forEach((handler) => handler())
      return 'new-menu-chat'
    }
  })
  const panel = defineComponent({
    name: 'NavigationPanel',
    components: { AgentsNavigation },
    template: '<AgentsNavigation :project-id="null" />',
  })
  view = shallowMount(App, {
    global: {
      renderStubDefaultSlot: true,
      stubs: {
        NavigationPanel: panel,
        AgentsNavigation: false,
        CreateChannelDialog: false,
        DialogContent: { template: '<div><slot /></div>' },
        Button: false,
        Input: false,
        ResizablePanelGroup: { template: '<div><slot /></div>' },
        ResizablePanel: { template: '<div><slot /></div>' },
      },
    },
  })
  await flushPromises()
  const navigation = view.getComponent(AgentsNavigation)
  expect(navigation.text()).not.toContain('New menu discussion')
  view.getComponent({ name: 'NavigationPanel' }).vm.$emit('createChannel')
  await flushPromises()
  const topLevelDialog = view
    .findAllComponents(CreateChannelDialog)
    .find((dialog) => dialog.props('open'))!
  expect(topLevelDialog).toBeDefined()
  await topLevelDialog.get('#channel-name').setValue('New menu discussion')
  await topLevelDialog.get('#conversation-agent').setValue('a')
  await topLevelDialog.get('form').trigger('submit')
  await flushPromises()
  expect(mocks.store.refresh).toHaveBeenCalled()
  expect(navigation.props('projectId')).toBeNull()
  expect(navigation.text()).not.toContain('New menu discussion')
  expect(navigation.text()).toContain('Robin')
})

test('memory source and fresh-conversation events open the transcript and keep sidebar selection', async () => {
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    projectId: 'separate-sidebar-project',
    activeProjectId: null,
  }
  const wrapper = app()
  await flushPromises()
  for (const action of ['source link', 'fresh conversation']) {
    wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'memory')
    await flushPromises()
    expect(wrapper.findComponent(MessageList).exists(), action).toBe(false)
    catalogHandlers.get('discard://conversation-opened')?.forEach((handler) => handler())
    await flushPromises()
    expect(wrapper.findComponent(MessageList).exists(), action).toBe(true)
    expect(mocks.store.snapshot?.projectId).toBe('separate-sidebar-project')
    expect(mocks.store.snapshot?.activeProjectId).toBeNull()
  }
})

test.each([true, false])(
  'Memory source selection opens retained history only when selection succeeds (selected=%s)',
  async (selected) => {
    mocks.store.snapshot = {
      ...mocks.store.snapshot!,
      projectId: 'separate-sidebar-project',
      activeProjectId: 'separate-sidebar-project',
    }
    const copyNotice = 'Reply was not saved. Copy the reply before sending again.'
    const restartNotice =
      'Conversation history is open, but its session could not restart: provider unavailable. Select this conversation again to retry.'
    vi.mocked(invoke).mockImplementation(async (name, payload) => {
      if (name === 'get_conversations') return []
      if (name === 'memory_get')
        return {
          items: [
            {
              id: 'memory-item',
              scope: 'global',
              kind: 'fact',
              content: 'Saved fact',
              version: 1,
              createdAt: 1,
              updatedAt: 1,
              deleted: false,
              history: [],
              sourceConversation: 'retained-chat',
              sourceAvailable: true,
            },
          ],
          scopes: [],
          proposals: [],
          responses: [],
          paused: false,
        }
      if (name === 'switch_channel') {
        expect(payload).toEqual({ channelId: 'retained-chat' })
        if (!selected) throw new Error('Source conversation is unavailable')
        // Backend contract: retained-history selection succeeds despite restart failure.
        // switch_channel then emits the shared event and focuses the main window.
        mocks.store.snapshot = {
          ...mocks.store.snapshot!,
          channelId: 'retained-chat',
          activeProjectId: null,
          notice: `${copyNotice}\n\n${restartNotice}`,
          liveBlocks: [{ type: 'markdown', text: 'Retained unsaved reply' }],
        }
        catalogHandlers.get('discard://conversation-opened')?.forEach((handler) => handler())
      }
    })
    view = shallowMount(App, {
      global: {
        renderStubDefaultSlot: true,
        stubs: {
          WorkspacePageView: false,
          MemoryBrowser: false,
          Notices: false,
          NoticeList: false,
          ResizablePanelGroup: { template: '<div><slot /></div>' },
          ResizablePanel: { template: '<div><slot /></div>' },
        },
      },
    })
    await flushPromises()
    view.getComponent(NavigationPanel).vm.$emit('navigate', 'memory')
    await flushPromises()
    expect(view.findComponent(MessageList).exists()).toBe(false)
    const browser = view.getComponent(MemoryBrowser)
    await browser
      .findAllComponents(Button)
      .find((button) => button.text() === 'Inspect')!
      .trigger('click')
    await browser
      .findAllComponents(Button)
      .find((button) => button.text() === 'Open source conversation')!
      .trigger('click')
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('switch_channel', { channelId: 'retained-chat' })
    expect(mocks.store.snapshot?.projectId).toBe('separate-sidebar-project')
    if (selected) {
      expect(view.findComponent(MemoryBrowser).exists()).toBe(false)
      expect(view.getComponent(MessageList).props('live')).toEqual([
        { type: 'markdown', text: 'Retained unsaved reply' },
      ])
      const notices = view.get('[data-ui-id="protected-notices"]')
      expect(notices.text()).toContain(copyNotice)
      expect(notices.text()).toContain(restartNotice)
      await view
        .findAllComponents(Button)
        .find((button) => button.text() === 'Copy reply before sending again')!
        .trigger('click')
      await flushPromises()
      expect(mocks.copy).toHaveBeenCalledWith('Retained unsaved reply')
      expect(mocks.store.snapshot?.activeProjectId).toBeNull()
    } else {
      expect(view.findComponent(MessageList).exists()).toBe(false)
      expect(browser.get('[role="alert"]').text()).toContain('Source conversation is unavailable')
      expect(mocks.store.snapshot?.channelId).toBe('channel')
    }
  },
)

test('Friends opens the add modal and the new agent profile', async () => {
  const wrapper = app()
  wrapper.getComponent(NavigationPanel).vm.$emit('action', 'show_friends')
  await flushPromises()
  wrapper.getComponent(FriendsList).vm.$emit('action', 'add_agent')
  await flushPromises()
  expect(wrapper.getComponent(AddAgentDialog).props('open')).toBe(true)
  wrapper.getComponent(AddAgentDialog).vm.$emit('created', 'new-friend')
  await flushPromises()
  expect(wrapper.getComponent(AgentProfileDialog).props('agentId')).toBe('new-friend')
})
test('Message from a profile opens the agent direct conversation', async () => {
  mocks.store.snapshot = { ...mocks.store.snapshot!, projectId: 'selected-project' }
  vi.mocked(invoke).mockImplementation(async (name) =>
    name === 'open_agent_chat' ? 'friend-dm' : undefined,
  )
  const wrapper = app()
  wrapper.getComponent(AgentProfileDialog).vm.$emit('message', 'friend')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('open_agent_chat', { agentId: 'friend' })
  expect(invoke).toHaveBeenCalledWith('switch_channel', { channelId: 'friend-dm' })
  expect(mocks.store.snapshot!.projectId).toBe('selected-project')
  expect(wrapper.getComponent(AgentProfileDialog).props('agentId')).toBeNull()
})

test('native profile actions open the named agent and cannot replace an active save', async () => {
  const wrapper = app()
  await flushPromises()
  const handler = vi
    .mocked(listen)
    .mock.calls.find((call) => call[0] === 'discard://agent-profile')![1]
  handler({ event: 'discard://agent-profile', id: 1, payload: 'target-agent' })
  await flushPromises()
  expect(wrapper.getComponent(AgentProfileDialog).props('agentId')).toBe('target-agent')
  wrapper.getComponent(AgentProfileDialog).vm.$emit('busy', true)
  handler({ event: 'discard://agent-profile', id: 2, payload: 'another-agent' })
  await flushPromises()
  expect(wrapper.getComponent(AgentProfileDialog).props('agentId')).toBe('target-agent')
})

test('history search opens its sidebar and selecting a match targets the exact saved message', async () => {
  const wrapper = app()
  wrapper.getComponent(ChannelHeader).vm.$emit('search', 'release')
  await flushPromises()
  const results = wrapper.getComponent(AgentHistorySearch)
  expect(results.props('agentId')).toBe(mocks.store.snapshot!.agentId)
  expect(results.props('query')).toBe('release')
  const match = {
    channelId: 'channel',
    channelName: 'Chat',
    messageId: 'old-message',
    role: 'user',
    preview: 'Release notes',
  }
  results.vm.$emit('select', match)
  await flushPromises()
  expect(wrapper.getComponent(MessageList).props('jumpTo')).toEqual(
    expect.objectContaining({ messageId: 'old-message' }),
  )
  wrapper.getComponent(ChannelHeader).vm.$emit('toggleMembers')
  await flushPromises()
  // The search sheet was closed by selecting the result. A opens the short profile.
  expect(wrapper.findComponent(AgentHistorySearch).exists()).toBe(false)
  expect(wrapper.findComponent(ContextPanel).exists()).toBe(true)
})
test('failed history selection does not jump or discard the current transcript', async () => {
  const wrapper = app()
  wrapper.getComponent(ChannelHeader).vm.$emit('search', 'query')
  await flushPromises()
  vi.mocked(invoke).mockRejectedValueOnce('Conversation is unavailable')
  wrapper.getComponent(AgentHistorySearch).vm.$emit('select', {
    channelId: 'missing',
    channelName: 'Chat',
    messageId: 'old-message',
    role: 'user',
    preview: 'query',
  })
  await flushPromises()
  expect(wrapper.getComponent(MessageList).props('jumpTo')).toBeUndefined()
  expect(mocks.store.snapshot!.channelId).toBe('channel')
})

test.each([null, 'project'])(
  'conversation chrome matches its project context (%s)',
  (activeProjectId) => {
    mocks.store.snapshot = {
      ...mocks.store.snapshot!,
      activeProjectId,
      slots: {
        channel_header: {
          plugin: 'test-header',
          element: { tag: 'text', props: {}, children: [{ Text: '# General' }] },
        },
      },
    }
    const wrapper = app()
    const header = wrapper.getComponent(ChannelHeader)
    expect(header.props('channel')).toBe(
      activeProjectId ? mocks.store.snapshot!.channel : mocks.store.snapshot!.agent,
    )
    expect(wrapper.find('[data-ui-id="conversation-tabs"]').exists()).toBe(!!activeProjectId)
    expect(header.findComponent({ name: 'PluginElement' }).exists()).toBe(!!activeProjectId)
    expect(wrapper.getComponent(Composer).props('directMessage')).toBe(!activeProjectId)
    expect(wrapper.findComponent(MessageList).exists()).toBe(true)
    if (!activeProjectId) expect(header.props('subtitle')).toBeUndefined()
  },
)

test.each([false, true])(
  'window resizing preserves sidebar widths (context=%s)',
  async (contextVisible) => {
    let width = 1400
    const observers = new Map<Element, ResizeObserverCallback>()
    vi.stubGlobal(
      'ResizeObserver',
      class {
        private callback: ResizeObserverCallback
        constructor(callback: ResizeObserverCallback) {
          this.callback = callback
        }
        observe(target: Element) {
          observers.set(target, this.callback)
        }
        unobserve(target: Element) {
          observers.delete(target)
        }
        disconnect() {
          observers.clear()
        }
      },
    )
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({
        matches: query === '(min-width: 1280px)',
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    )
    const rect = vi
      .spyOn(HTMLElement.prototype, 'getBoundingClientRect')
      .mockImplementation(() => ({
        width,
        height: 800,
        top: 0,
        left: 0,
        right: width,
        bottom: 800,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      }))
    mocks.store.snapshot = {
      ...mocks.store.snapshot!,
      memberListVisible: contextVisible,
      panels: { channelList: 240, memberList: 300 },
    }
    try {
      view = shallowMount(App, {
        attachTo: document.body,
        global: {
          renderStubDefaultSlot: true,
          stubs: {
            ResizablePanelGroup: false,
            ResizablePanel: false,
            SplitterGroup: false,
            SplitterPanel: false,
            Primitive: false,
          },
        },
      })
      await flushPromises()
      const { SplitterPanel } = await import('reka-ui')
      const panels = view.findAllComponents(SplitterPanel)
      const group = view.getComponent({ name: 'ResizablePanelGroup' })
      const layout = () => group.emitted('layout')!.at(-1)![0] as number[]
      expect(panels).toHaveLength(contextVisible ? 3 : 2)
      expect(layout()[0]).toBeCloseTo(312)
      if (contextVisible) expect(layout()[2]).toBeCloseTo(300)
      // A user-adjusted width must also survive window changes.
      panels[0]!.vm.resize(360)
      await flushPromises()
      expect(layout()[0]).toBeCloseTo(360)
      for (const nextWidth of [1800, 1300, 1500]) {
        width = nextWidth
        for (const [target, callback] of observers) {
          callback(
            [{ target, contentRect: target.getBoundingClientRect() } as ResizeObserverEntry],
            {} as ResizeObserver,
          )
        }
        await flushPromises()
        expect(layout()[0]).toBeCloseTo(360)
        if (contextVisible) expect(layout()[2]).toBeCloseTo(300)
      }
    } finally {
      view?.unmount()
      view = undefined
      rect.mockRestore()
    }
  },
)
