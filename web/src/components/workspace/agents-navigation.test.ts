import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { mount, flushPromises } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import AgentsNavigation from './AgentsNavigation.vue'
import CreateChannelDialog from './CreateChannelDialog.vue'
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const handlers = new Map<string, () => void>()
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, callback: () => void) => {
    handlers.set(name, callback)
    return () => handlers.delete(name)
  }),
}))
const agents = [
  {
    id: 'a',
    name: 'Robin',
    avatar: null,
    provider: 'codex',
    model: 'exact-model',
    archived: false,
  },
  { id: 'b', name: 'Alex', avatar: null, provider: 'codex', model: 'exact-model', archived: false },
]
const conversations = [
  {
    id: 'dm',
    name: 'General',
    agentId: 'a',
    agent: 'Robin',
    projectId: null,
    deleted: false,
    agentArchived: false,
    archived: false,
  },
  {
    id: 'kept',
    name: 'Kept history',
    agentId: 'gone',
    agent: 'Deleted agent',
    projectId: null,
    deleted: true,
    agentArchived: false,
    archived: false,
  },
  {
    id: 'project-one',
    name: 'Design',
    agentId: 'a',
    agent: 'Robin',
    projectId: 'project',
    deleted: false,
    agentArchived: true,
    archived: false,
  },
  {
    id: 'project-two',
    name: 'Implementation',
    agentId: 'b',
    agent: 'Alex',
    projectId: 'project',
    deleted: false,
    agentArchived: false,
    archived: false,
  },
]
beforeEach(() => {
  handlers.clear()
  vi.mocked(invoke)
    .mockReset()
    .mockImplementation(async (name) => {
      if (name === 'settings_agent_get') return agents
      if (name === 'get_conversations') return conversations
      if (name === 'open_agent_chat') return 'dm'
      if (name === 'create_conversation') return 'created'
    })
})
test('lists global agents with shared models and opens their default DM without a project', async () => {
  const view = mount(AgentsNavigation, {
    props: { projectId: null },
    global: { stubs: { CreateChannelDialog: true } },
  })
  await flushPromises()
  expect(view.text()).toContain('Robin')
  expect(view.text()).toContain('Alex')
  expect(view.text()).toContain('Kept history')
  expect(view.text()).toContain('Read only')
  expect(view.text()).not.toContain('Implementation')
  await view
    .findAll('button')
    .find((b) => b.text().includes('Robin') && b.text().includes('exact-model'))!
    .trigger('click')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('open_agent_chat', { agentId: 'a' })
  expect(view.emitted('select')).toEqual([['dm']])
  await view
    .findAll('button')
    .find((b) => b.text().includes('Kept history'))!
    .trigger('click')
  expect(view.emitted('select')?.at(-1)).toEqual(['kept'])
  view.unmount()
})
test('project lists retain multiple conversations and archived histories without selecting another agent', async () => {
  const view = mount(AgentsNavigation, {
    props: { projectId: 'project' },
    global: { stubs: { CreateChannelDialog: true } },
  })
  await flushPromises()
  expect(view.text()).toContain('Design')
  expect(view.text()).toContain('Implementation')
  expect(view.text()).toContain('Archived')
  expect(view.text()).not.toContain('Kept history')
  await view
    .findAll('button')
    .find((b) => b.text().includes('Design'))!
    .trigger('click')
  expect(view.emitted('select')).toEqual([['project-one']])
  expect(invoke).not.toHaveBeenCalledWith('open_agent_chat', expect.anything())
  view.unmount()
})
test.each([null, 'project'])(
  'conversation creation requires an explicit agent and pins context %s',
  async (projectId) => {
    const view = mount(CreateChannelDialog, {
      props: { open: false, project: 'Context', projectId },
      global: { stubs: { DialogContent: { template: '<div><slot /></div>' } } },
    })
    await view.setProps({ open: true })
    await flushPromises()
    const form = view
    await form.get('#channel-name').setValue('New discussion')
    expect(form.get('[data-ui-id="create-channel-submit"]').attributes('disabled')).toBeDefined()
    await form.get('#conversation-agent').setValue('b')
    await view.setProps({ projectId: 'different-project' })
    await form.get('form').trigger('submit')
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('create_conversation', {
      agentId: 'b',
      projectId,
      name: 'New discussion',
    })
    expect(view.emitted('created')).toEqual([['created']])
    view.unmount()
  },
)

test('shared conversation notification refreshes a menu-created chat in the same context', async () => {
  const view = mount(AgentsNavigation, {
    props: { projectId: null },
    global: { stubs: { CreateChannelDialog: true } },
  })
  await flushPromises()
  expect(view.text()).not.toContain('Menu-created discussion')
  vi.mocked(invoke).mockImplementation(async (name) => {
    if (name === 'settings_agent_get') return agents
    if (name === 'get_conversations')
      return [
        ...conversations,
        { ...conversations[0], id: 'from-menu', name: 'Menu-created discussion' },
      ]
  })
  expect(handlers.has('discard://conversations')).toBe(true)
  handlers.get('discard://conversations')!()
  await flushPromises()
  expect(view.text()).toContain('Menu-created discussion')
  await view
    .findAll('button')
    .find((b) => b.text().includes('Menu-created discussion'))!
    .trigger('click')
  expect(view.emitted('select')).toEqual([['from-menu']])
  view.unmount()
})
