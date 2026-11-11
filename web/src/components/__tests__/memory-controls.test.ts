import { mount, flushPromises } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vite-plus/test'
import MemoryBrowser from './memory/MemoryBrowser.vue'
import { Select } from './ui/select'
import MemoryResponse from './memory/MemoryResponse.vue'
import { invoke } from '@tauri-apps/api/core'
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
const item = {
  id: 'item',
  scope: 'agent:a',
  kind: 'preference',
  content: 'Use short replies',
  version: 2,
  createdAt: 1,
  updatedAt: 2,
  deleted: false,
  sourceConversation: 'source',
  sourceAvailable: true,
  history: [
    {
      version: 1,
      scope: 'agent:a',
      content: 'Old reply style',
      deleted: false,
      updatedAt: 1,
      origin: 'Automatic',
    },
  ],
}
const data = {
  items: [item],
  scopes: [
    { scope: 'global', title: 'Global' },
    { scope: 'agent:a', title: 'Agent A' },
  ],
  proposals: [],
  responses: [],
  paused: true,
}
beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(invoke).mockImplementation(async (name) =>
    name === 'memory_get'
      ? structuredClone(data)
      : name === 'create_conversation'
        ? 'fresh'
        : undefined,
  )
})
async function button(view: ReturnType<typeof mount>, text: string) {
  const found = view.findAll('button').find((b) => b.text() === text)
  expect(found, text).toBeDefined()
  await found!.trigger('click')
  await flushPromises()
}
test('conversation browser inspects history, edits with version, moves to shared domain and opens source', async () => {
  const view = mount(MemoryBrowser, {
    props: { channelId: 'current', agentId: 'a', projectId: null },
  })
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('memory_get', { channelId: 'current' })
  await button(view, 'Inspect')
  expect(view.text()).toContain('Old reply style')
  await view.get('textarea').setValue('Updated preference')
  await button(view, 'Save edit')
  expect(invoke).toHaveBeenCalledWith(
    'memory_mutate',
    expect.objectContaining({
      id: 'item',
      expectedVersion: 2,
      action: 'edit',
      content: 'Updated preference',
      channelId: 'current',
    }),
  )
  view.getComponent(Select).vm.$emit('update:modelValue', 'global')
  await flushPromises()
  await button(view, 'Move')
  expect(invoke).toHaveBeenCalledWith(
    'memory_mutate',
    expect.objectContaining({ action: 'move', scope: 'global' }),
  )
  await button(view, 'Reveal item file')
  expect(invoke).toHaveBeenCalledWith('memory_reveal_item', { id: 'item' })
  await button(view, 'Open source conversation')
  expect(invoke).toHaveBeenCalledWith('switch_channel', { channelId: 'source' })
})
test('forget passes requester, explains transcript, shows pause and supports resume and fresh conversation', async () => {
  const view = mount(MemoryBrowser, {
    props: { channelId: 'current', agentId: 'a', projectId: 'p' },
  })
  await flushPromises()
  expect(view.text()).toContain('Automatic capture is paused')
  await button(view, 'Inspect')
  expect(view.text()).toContain('Transcript messages stay saved')
  await button(view, 'Forget')
  expect(invoke).toHaveBeenCalledWith(
    'memory_mutate',
    expect.objectContaining({ action: 'forget', channelId: 'current', expectedVersion: 2 }),
  )
  await button(view, 'Resume automatic capture')
  expect(invoke).toHaveBeenCalledWith('memory_resume', { channelId: 'current' })
  await button(view, 'Start fresh conversation')
  expect(invoke).toHaveBeenCalledWith('memory_fresh_conversation', { channelId: 'current' })
})
test('the domain Select filters memory and can return to all domains', async () => {
  const view = mount(MemoryBrowser)
  try {
    await flushPromises()
    const domains = view.getComponent(Select)
    expect(view.get('label').attributes('for')).toBe(view.get('[role="combobox"]').attributes('id'))
    domains.vm.$emit('update:modelValue', 'global')
    await flushPromises()
    expect(view.text()).not.toContain(item.content)
    domains.vm.$emit('update:modelValue', 'agent:a')
    await flushPromises()
    expect(view.text()).toContain(item.content)
    domains.vm.$emit('update:modelValue', 'all')
    await flushPromises()
    expect(view.text()).toContain(item.content)
  } finally {
    view.unmount()
  }
})
test('settings browses all domains and source-unavailable items; errors keep draft and offer import/reload', async () => {
  vi.mocked(invoke).mockResolvedValueOnce({
    ...structuredClone(data),
    items: [{ ...item, sourceAvailable: false }],
  })
  const view = mount(MemoryBrowser)
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('memory_get', { channelId: null })
  await button(view, 'Inspect')
  expect(view.text()).toContain('Source conversation is unavailable')
  await view.get('textarea').setValue('Unsaved edit')
  vi.mocked(invoke).mockRejectedValueOnce('Conflict. Reload memory.')
  await button(view, 'Save edit')
  expect(view.get('[role="alert"]').text()).toContain('Conflict')
  expect(view.get('textarea').element.value).toBe('Unsaved edit')
  await button(view, 'Import file edits and reload')
  expect(invoke).toHaveBeenCalledWith('memory_reconcile')
})
test('response cards use persisted versions for undo and resolve proposals after response', async () => {
  const view = mount(MemoryResponse, {
    props: {
      channelId: 'current',
      records: [
        {
          key: 'item:1',
          responseId: 'persisted',
          type: 'automatic',
          item: { ...item, version: 1 },
        },
        { key: 'p', responseId: 'persisted', type: 'proposal', proposalId: 'p', pending: true },
      ],
      proposals: [
        {
          id: 'p',
          action: 'Create',
          scope: 'global',
          content: 'Shared preference',
          sourceConversation: 'current',
        },
      ],
    },
  })
  expect(view.text()).toContain('Automatic save')
  expect(view.text()).toContain('Shared preference')
  vi.mocked(invoke).mockRejectedValueOnce('Stale Undo. Reload memory.')
  await button(view, 'Undo')
  expect(invoke).toHaveBeenCalledWith(
    'memory_mutate',
    expect.objectContaining({ expectedVersion: 1, action: 'undo' }),
  )
  expect(view.get('[role="alert"]').text()).toContain('Stale Undo')
  await button(view, 'Approve')
  expect(invoke).toHaveBeenCalledWith('memory_resolve', {
    id: 'p',
    approve: true,
    channelId: 'current',
  })
  expect(
    view
      .findAll('button')
      .find((button) => button.text() === 'Reject')!
      .attributes('disabled'),
  ).toBeDefined()
})

test('transcript attaches saved cards only to their persisted response and never to unsaved live reply', async () => {
  const { default: MessageList } = await import('./MessageList.vue')
  vi.mocked(invoke).mockImplementation(async (name) =>
    name === 'memory_get'
      ? {
          ...structuredClone(data),
          responses: [{ key: 'item:2', responseId: 'saved-response', type: 'automatic', item }],
          agentId: 'a',
          projectId: null,
        }
      : undefined,
  )
  const view = mount(MessageList, {
    props: {
      conversationId: 'current',
      messages: [
        {
          id: 'saved-response',
          role: 'agent',
          blocks: [{ type: 'markdown', text: 'Saved reply' }],
          continuesGroup: false,
        },
      ],
      live: [{ type: 'markdown', text: 'Unsaved reply' }],
      running: false,
      newAnchorId: null,
    },
  })
  await flushPromises()
  expect(view.findAll('[data-ui-id="response-memory"]')).toHaveLength(1)
  expect(view.get('[data-message-id="saved-response"]').text()).toContain('Automatic save')
  expect(view.get('[data-message-id="streaming-reply"]').text()).not.toContain('Automatic save')
  expect(view.text()).toContain('Automatic capture is paused')
})

test('runtime memory revision reloads cards after response text is already present', async () => {
  const { default: MessageList } = await import('./MessageList.vue')
  let saved = false
  vi.mocked(invoke).mockImplementation(async (name) =>
    name === 'memory_get'
      ? {
          ...structuredClone(data),
          responses: saved
            ? [{ key: 'item:2', responseId: 'persisted', type: 'automatic', item }]
            : [],
        }
      : undefined,
  )
  const view = mount(MessageList, {
    props: {
      conversationId: 'current',
      memoryRevision: 0,
      messages: [{ id: 'persisted', role: 'agent', blocks: [], continuesGroup: false }],
      live: [],
      running: false,
      newAnchorId: null,
    },
  })
  await flushPromises()
  expect(view.find('[data-ui-id="response-memory"]').exists()).toBe(false)
  saved = true
  await view.setProps({ memoryRevision: 1 })
  await flushPromises()
  expect(view.get('[data-ui-id="response-memory"]').text()).toContain('Automatic save')
})

test('committed approval with a pending response notice disables decisions and refreshes clients', async () => {
  const { default: MemoryProposalCard } = await import('./memory/MemoryProposal.vue')
  vi.mocked(invoke).mockResolvedValueOnce({
    status: 'approved',
    committed: true,
    associationPending: true,
    warning: 'Memory is approved and saved. Reload memory to retry its response notice.',
  })
  const view = mount(MemoryProposalCard, {
    props: {
      channelId: 'current',
      proposal: {
        id: 'p',
        action: 'Create',
        scope: 'global',
        content: 'Shared value',
        sourceConversation: 'current',
      },
    },
  })
  await button(view, 'Approve')
  expect(view.emitted('changed')).toHaveLength(1)
  expect(view.text()).toContain('Memory is approved and saved')
  expect(view.findAll('button').every((b) => b.attributes('disabled') !== undefined)).toBe(true)
  await view.get('button').trigger('click')
  expect(invoke).toHaveBeenCalledTimes(1)
})

test('pending notice warnings provide a working reload and rejection settles its own proposal', async () => {
  vi.mocked(invoke).mockResolvedValueOnce({
    ...structuredClone(data),
    warnings: ['Memory is approved and saved. Reload memory to retry its response notice.'],
  })
  const browser = mount(MemoryBrowser, { props: { channelId: 'current' } })
  await flushPromises()
  expect(browser.get('[role="status"]').text()).toContain('Memory is approved and saved')
  await button(browser, 'Reload memory')
  expect(browser.find('[role="status"]').exists()).toBe(false)
  const { default: MemoryProposalCard } = await import('./memory/MemoryProposal.vue')
  vi.mocked(invoke).mockResolvedValueOnce({
    status: 'rejected',
    committed: false,
    associationPending: false,
    warning: null,
  })
  const view = mount(MemoryProposalCard, {
    props: {
      proposal: {
        id: 'other',
        action: 'Create',
        scope: 'global',
        content: 'Do not keep',
        sourceConversation: 'source',
      },
    },
  })
  await button(view, 'Reject')
  expect(view.emitted('changed')).toHaveLength(1)
  expect(view.findAll('button').every((b) => b.attributes('disabled') !== undefined)).toBe(true)
})
