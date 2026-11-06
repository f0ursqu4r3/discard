import { shallowMount, flushPromises } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { beforeEach, afterEach, expect, test, vi } from 'vite-plus/test'
import fixture from './bridge/fixtures/snapshot.json'
import type { MessageDto, SnapshotDto } from './bridge/types'
import type { WorkspacePage } from './components/workspace/types'
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
  getProjectChannels: vi.fn(async () => []),
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
import MemoryBrowser from './components/memory/MemoryBrowser.vue'
import { Button } from './components/ui/button'
let view: ReturnType<typeof shallowMount> | undefined
beforeEach(() => {
  vi.clearAllMocks()
  localStorage.removeItem('discard:last-direct-message')
  catalogHandlers.clear()
  mocks.store.snapshot = {
    ...structuredClone(fixture),
    projectId: '',
    project: '',
    projectPath: '',
    projectDisplayPath: '',
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
  expect(wrapper.getComponent(MessageList).props('agent')).toBe(mocks.store.snapshot!.agent)
})

test('project transcript labels saved replies neutrally after the main agent changes', () => {
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    activeProjectId: 'project',
    agent: 'Previous main agent',
  }
  const previous = app()
  expect(previous.getComponent(MessageList).props('agent')).toBe('Agent')
  previous.unmount()
  mocks.store.snapshot = { ...mocks.store.snapshot!, agent: 'New main agent' }
  const current = app()
  expect(current.getComponent(MessageList).props('agent')).toBe('Agent')
})

test('project selection opens the returned general channel', async () => {
  vi.mocked(invoke).mockImplementation(async (name) => {
    if (name === 'select_project') return 'general-channel'
  })
  mocks.store.refresh.mockImplementationOnce(async () => {
    mocks.store.snapshot = {
      ...mocks.store.snapshot!,
      projectId: 'target-project',
      activeProjectId: 'target-project',
      channelId: 'general-channel',
    }
  })
  const wrapper = app()
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'overview')
  wrapper.getComponent(NavigationPanel).vm.$emit('action', 'select_project:target-project')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('select_project', { projectId: 'target-project' })
  expect(mocks.store.snapshot?.channelId).toBe('general-channel')
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('conversation')
})

test('conversation-opened event returns to the restored project transcript', async () => {
  mocks.store.snapshot = { ...mocks.store.snapshot!, activeProjectId: null, channelId: 'dm' }
  const wrapper = app()
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'overview')
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    activeProjectId: 'project',
    channelId: 'general',
  }
  catalogHandlers.get('discard://conversation-opened')?.forEach((handler) => handler())
  await flushPromises()
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('conversation')
})

test.each(['project chat', 'direct message', 'project overview'] as const)(
  'opens an archived project transcript from %s without changing its canonical state',
  async (origin) => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({
        matches: query === '(min-width: 1280px)',
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    )
    mocks.store.snapshot = {
      ...mocks.store.snapshot!,
      projectId: 'selected-project',
      activeProjectId: origin === 'direct message' ? null : 'selected-project',
      channelId: origin === 'direct message' ? 'dm-id' : 'current-channel',
      memberListVisible: false,
    }
    if (origin === 'project overview')
      mocks.store.snapshot = { ...mocks.store.snapshot!, projectId: 'selected-project' }
    mocks.store.refresh.mockImplementationOnce(async () => {
      mocks.store.snapshot = {
        ...mocks.store.snapshot!,
        activeProjectId: 'selected-project',
        projectId: 'selected-project',
        channelId: 'archived-id',
        channel: 'Archived history',
        memberListVisible: true,
      }
    })
    vi.mocked(invoke).mockResolvedValue(undefined)
    const wrapper = app()
    if (origin === 'project overview')
      wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'overview')
    wrapper.getComponent(NavigationPanel).vm.$emit('inspect-channel', 'archived-id')
    await flushPromises()

    expect(invoke).toHaveBeenCalledWith('switch_channel', { channelId: 'archived-id' })
    expect(mocks.store.snapshot?.channelId).toBe('archived-id')
    if (origin === 'project overview')
      expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('conversation')
    expect(invoke).not.toHaveBeenCalledWith('select_canonical_project_channel', expect.anything())
    expect(invoke).not.toHaveBeenCalledWith('select_canonical_branch', expect.anything())
  },
)

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
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('direct-messages')
  expect(wrapper.getComponent(ContextPanel).props('directMessage')).toBe(true)
})
test('Direct Messages returns to the active DM from a project page without reopening it', async () => {
  mocks.store.snapshot = { ...mocks.store.snapshot!, projectId: 'project' }
  const wrapper = app()
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'overview')
  await flushPromises()
  expect(wrapper.findComponent(MessageList).exists()).toBe(false)
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'direct-messages')
  await flushPromises()
  expect(wrapper.findComponent(MessageList).exists()).toBe(true)
  expect(wrapper.findComponent(FriendsList).exists()).toBe(false)
  expect(invoke).not.toHaveBeenCalledWith('switch_channel', expect.anything())
})
test('Direct Messages restores the last opened DM after remounting with a project conversation', async () => {
  mocks.store.snapshot = { ...mocks.store.snapshot!, channelId: 'last-dm' }
  app().unmount()
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    channelId: 'project-chat',
    activeProjectId: 'project',
    projectId: 'project',
  }
  vi.mocked(invoke).mockImplementation(async (name, args) => {
    if (name === 'get_conversations')
      return [
        { id: 'another-dm', projectId: null },
        { id: 'last-dm', projectId: null },
      ]
    if (name === 'switch_channel') {
      mocks.store.snapshot = {
        ...mocks.store.snapshot!,
        channelId: (args as { channelId: string }).channelId,
        activeProjectId: null,
      }
    }
  })
  const wrapper = app()
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'overview')
  await flushPromises()
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'direct-messages')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('switch_channel', { channelId: 'last-dm' })
  expect(wrapper.getComponent(MessageList).props('conversationId')).toBe('last-dm')
  expect(wrapper.getComponent(NavigationPanel).props('page')).toBe('direct-messages')
  wrapper.getComponent(NavigationPanel).vm.$emit('action', 'show_friends')
  await flushPromises()
  expect(wrapper.findComponent(FriendsList).exists()).toBe(true)
})
test.each([null, 'deleted-dm'])(
  'Direct Messages falls back to Agents without an available DM (%s)',
  async (saved) => {
    if (saved) localStorage.setItem('discard:last-direct-message', saved)
    mocks.store.snapshot = {
      ...mocks.store.snapshot!,
      activeProjectId: 'project',
      projectId: 'project',
    }
    vi.mocked(invoke).mockImplementation(async (name) =>
      name === 'get_conversations' ? [] : undefined,
    )
    const wrapper = app()
    wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'direct-messages')
    await flushPromises()
    expect(wrapper.findComponent(FriendsList).exists()).toBe(true)
    expect(wrapper.findComponent(MessageList).exists()).toBe(false)
    expect(invoke).not.toHaveBeenCalledWith('switch_channel', expect.anything())
    expect(localStorage.getItem('discard:last-direct-message')).toBeNull()
  },
)
test('a failed DM switch keeps the project view and the remembered DM', async () => {
  localStorage.setItem('discard:last-direct-message', 'last-dm')
  mocks.store.snapshot = {
    ...mocks.store.snapshot!,
    activeProjectId: 'project',
    projectId: 'project',
  }
  vi.mocked(invoke).mockImplementation(async (name) => {
    if (name === 'get_conversations') return [{ id: 'last-dm', projectId: null }]
    if (name === 'switch_channel') throw new Error('Conversation is busy')
  })
  const wrapper = app()
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'overview')
  await flushPromises()
  wrapper.getComponent(NavigationPanel).vm.$emit('navigate', 'direct-messages')
  await flushPromises()
  expect(wrapper.getComponent(ChannelHeader).props('channel')).toBe('Project overview')
  expect(localStorage.getItem('discard:last-direct-message')).toBe('last-dm')
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
    expect(header.props('directMessage')).toBe(!activeProjectId)
    expect(header.props('sidebarAvailable')).toBe(true)
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

test.each<WorkspacePage>(['overview', 'agents', 'memory', 'artifacts', 'project-settings'])(
  'project page %s hides the short-profile control while a DM remains active',
  async (page) => {
    mocks.store.snapshot = { ...mocks.store.snapshot!, projectId: 'project' }
    const wrapper = app()
    wrapper.getComponent(ChannelHeader).vm.$emit('toggleMembers')
    await flushPromises()
    wrapper.getComponent(NavigationPanel).vm.$emit('navigate', page)
    await flushPromises()
    const header = wrapper.getComponent(ChannelHeader)
    expect(header.props('directMessage')).toBe(false)
    expect(header.props('sidebarAvailable')).toBe(false)
    expect(header.props('memberListVisible')).toBe(false)
    expect(header.props('agentName')).toBeUndefined()
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
