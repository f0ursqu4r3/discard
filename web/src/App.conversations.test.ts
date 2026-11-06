import { shallowMount, flushPromises } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { invoke } from '@tauri-apps/api/core'
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
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
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
  expect(wrapper.text()).toContain('Choose an agent or conversation')
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
  expect(mocks.run).toHaveBeenCalledWith('open_agent_settings')
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
  expect(navigation.text()).toContain('New menu discussion')
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
