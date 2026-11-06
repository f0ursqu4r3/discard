import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import ProjectChannelDetails from './ProjectChannelDetails.vue'

const api = vi.hoisted(() => ({
  getAgents: vi.fn(),
  getChannelDetails: vi.fn(),
  getProjectMembers: vi.fn(),
  updateChannelTopic: vi.fn(),
  setProjectMainAgent: vi.fn(),
  selectCanonicalProjectChannel: vi.fn(),
  selectCanonicalBranch: vi.fn(),
  switchChannel: vi.fn(),
}))
const events = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: string }) => void>(),
}))
vi.mock('@/bridge/api', () => api)
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: string }) => void) => {
    events.handlers.set(name, handler)
    return () => events.handlers.delete(name)
  }),
}))

const details = (channelId: string, topic = 'old topic', version = 2) => ({
  channel: {
    id: channelId,
    projectId: 'project',
    name: 'general',
    agentId: 'agent-a',
    workspaceMode: 'main_checkout',
    kind: 'general',
    branch: null,
    archived: false,
    archivedInto: null,
  },
  topic: { channelId, text: topic, version },
  topicHistory: [
    { channelId, text: 'old topic', version: 1, authorKind: 'user', authorId: null },
    { channelId, text: topic, version, authorKind: 'user', authorId: null },
  ],
  members: [{ id: 'agent-a', name: 'Robin', provider: 'codex', archived: false }],
  mainAgentId: 'agent-a',
})

beforeEach(() => {
  Object.values(api).forEach((mock) => mock.mockReset())
  api.getAgents.mockResolvedValue([
    { id: 'agent-a', name: 'Robin', provider: 'codex', model: 'model', archived: false },
    { id: 'agent-b', name: 'Taylor', provider: 'anthropic', model: 'model-2', archived: false },
  ])
  api.getProjectMembers.mockResolvedValue([
    { id: 'agent-a', name: 'Robin', provider: 'codex', archived: false },
  ])
  api.getChannelDetails.mockResolvedValue(details('general-id'))
  api.updateChannelTopic.mockResolvedValue({ channelId: 'general-id', text: 'saved', version: 3 })
  api.switchChannel.mockResolvedValue(undefined)
  events.handlers.clear()
})

afterEach(() => {
  vi.clearAllMocks()
  events.handlers.clear()
})

test('saves a topic with the loaded version and reloads persisted topic history', async () => {
  api.getChannelDetails.mockResolvedValueOnce(details('general-id'))
  api.updateChannelTopic.mockResolvedValueOnce({
    channelId: 'general-id',
    text: 'new topic',
    version: 3,
  })
  api.getChannelDetails.mockResolvedValueOnce(details('general-id', 'new topic', 3))
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'general-id', generalRunning: false },
  })
  await flushPromises()
  await wrapper.get('textarea[aria-label="Channel topic"]').setValue('new topic')
  await wrapper.get('[data-ui-id="save-topic"]').trigger('click')
  await flushPromises()

  expect(api.updateChannelTopic).toHaveBeenCalledWith('project', 'general-id', 2, 'new topic')
  expect(api.getChannelDetails).toHaveBeenLastCalledWith('project', 'general-id')
  expect(wrapper.text()).toContain('new topic')
  expect(wrapper.text()).toContain('Version 3')
})

test('shows stale topic feedback and keeps edits available after a version conflict', async () => {
  api.getChannelDetails
    .mockResolvedValueOnce(details('general-id'))
    .mockResolvedValueOnce(details('general-id', 'latest topic', 3))
  api.updateChannelTopic.mockRejectedValueOnce(
    new Error('Topic changed. Expected version 2, found 3.'),
  )
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'general-id', generalRunning: false },
  })
  await flushPromises()
  await wrapper.get('textarea[aria-label="Channel topic"]').setValue('my edit')
  await wrapper.get('[data-ui-id="save-topic"]').trigger('click')
  await flushPromises()
  expect(wrapper.get('[role="alert"]').text()).toContain('changed elsewhere')
  expect(
    (wrapper.get('textarea[aria-label="Channel topic"]').element as HTMLTextAreaElement).value,
  ).toBe('my edit')
})

test('remote topic updates preserve an unsaved draft and block a stale save', async () => {
  api.getChannelDetails
    .mockResolvedValueOnce(details('general-id'))
    .mockResolvedValueOnce(details('general-id', 'remote topic', 3))
    .mockResolvedValueOnce(details('general-id', 'remote topic', 3))
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'general-id', generalRunning: false },
  })
  await flushPromises()
  await wrapper.get('textarea[aria-label="Channel topic"]').setValue('my draft')
  events.handlers.get('discard://channel-details')?.({ payload: 'general-id' })
  await flushPromises()
  expect(
    (wrapper.get('textarea[aria-label="Channel topic"]').element as HTMLTextAreaElement).value,
  ).toBe('my draft')
  expect(wrapper.get('[role="alert"]').text()).toContain('changed while you were editing')
  expect(wrapper.get('[data-ui-id="save-topic"]').attributes('disabled')).toBeDefined()
  await wrapper.get('[data-ui-id="reload-topic"]').trigger('click')
  await flushPromises()
  expect(
    (wrapper.get('textarea[aria-label="Channel topic"]').element as HTMLTextAreaElement).value,
  ).toBe('remote topic')
})

test('keeps typing that starts during a same-version details refresh', async () => {
  let resolveRefresh!: (value: ReturnType<typeof details>) => void
  api.getChannelDetails
    .mockResolvedValueOnce(details('general-id'))
    .mockImplementationOnce(() => new Promise((resolve) => (resolveRefresh = resolve)))
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'general-id', generalRunning: false },
  })
  await flushPromises()
  events.handlers.get('discard://channel-details')?.({ payload: 'general-id' })
  await wrapper.get('textarea[aria-label="Channel topic"]').setValue('typed during refresh')
  resolveRefresh(details('general-id'))
  await flushPromises()
  expect(
    (wrapper.get('textarea[aria-label="Channel topic"]').element as HTMLTextAreaElement).value,
  ).toBe('typed during refresh')
  expect(wrapper.find('[role="alert"]').exists()).toBe(false)
})

test('keeps the latest draft and marks conflict when the topic changes during refresh', async () => {
  let resolveRefresh!: (value: ReturnType<typeof details>) => void
  api.getChannelDetails
    .mockResolvedValueOnce(details('general-id'))
    .mockImplementationOnce(() => new Promise((resolve) => (resolveRefresh = resolve)))
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'general-id', generalRunning: false },
  })
  await flushPromises()
  await wrapper.get('textarea[aria-label="Channel topic"]').setValue('first draft')
  events.handlers.get('discard://channel-details')?.({ payload: 'general-id' })
  await wrapper.get('textarea[aria-label="Channel topic"]').setValue('latest draft')
  resolveRefresh(details('general-id', 'remote topic', 3))
  await flushPromises()
  expect(
    (wrapper.get('textarea[aria-label="Channel topic"]').element as HTMLTextAreaElement).value,
  ).toBe('latest draft')
  expect(wrapper.get('[role="alert"]').text()).toContain('changed while you were editing')
})

test('reloads the main agent after a command error that may follow persistence', async () => {
  const updated = { ...details('general-id'), mainAgentId: 'agent-b' }
  api.getChannelDetails.mockResolvedValueOnce(details('general-id')).mockResolvedValueOnce(updated)
  api.setProjectMainAgent.mockRejectedValueOnce(new Error('Launcher failed after saving the agent'))
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'general-id', generalRunning: false },
  })
  await flushPromises()
  await wrapper.get('[data-ui-id="project-main-agent"]').setValue('agent-b')
  await flushPromises()
  expect(api.setProjectMainAgent).toHaveBeenCalledWith('project', 'agent-b')
  expect(api.getChannelDetails).toHaveBeenCalledTimes(2)
  expect(wrapper.get('[data-ui-id="project-main-agent"]').element).toHaveProperty(
    'value',
    'agent-b',
  )
  expect(wrapper.get('[role="alert"]').text()).toContain('Launcher failed after saving the agent')
})

test('main-agent refresh keeps a valid topic draft when its version is unchanged', async () => {
  api.getChannelDetails
    .mockResolvedValueOnce(details('general-id'))
    .mockResolvedValueOnce({ ...details('general-id'), mainAgentId: 'agent-b' })
  api.setProjectMainAgent.mockResolvedValueOnce(undefined)
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'general-id', generalRunning: false },
  })
  await flushPromises()
  await wrapper.get('textarea[aria-label="Channel topic"]').setValue('unsaved but current')
  await wrapper.get('[data-ui-id="project-main-agent"]').setValue('agent-b')
  await flushPromises()
  expect(
    (wrapper.get('textarea[aria-label="Channel topic"]').element as HTMLTextAreaElement).value,
  ).toBe('unsaved but current')
  expect(wrapper.find('[role="alert"]').exists()).toBe(false)
})

test('ignores stale details responses after a channel switch', async () => {
  let resolveOld!: (value: ReturnType<typeof details>) => void
  api.getChannelDetails.mockImplementationOnce(
    () => new Promise((resolve) => (resolveOld = resolve)),
  )
  api.getChannelDetails.mockResolvedValueOnce(details('second-id', 'second topic'))
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'first-id', generalRunning: false },
  })
  await wrapper.setProps({ channelId: 'second-id' })
  await flushPromises()
  resolveOld(details('first-id', 'stale topic'))
  await flushPromises()
  expect(wrapper.text()).toContain('second topic')
  expect(wrapper.text()).not.toContain('stale topic')
})

test('offers canonical promotion for archived legacy history without a failing restore action', async () => {
  const archived = {
    ...details('old-id'),
    channel: {
      ...details('old-id').channel,
      kind: 'archived_legacy',
      archived: true,
      archivedInto: 'new-id',
      branch: 'feature/x',
    },
  }
  const promoted = {
    ...archived,
    channel: { ...archived.channel, kind: 'branch', archived: false, archivedInto: null },
  }
  api.getChannelDetails.mockResolvedValueOnce(archived).mockResolvedValueOnce(promoted)
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'old-id', generalRunning: false },
  })
  await flushPromises()
  expect(wrapper.text()).toContain('Archived history')
  expect(wrapper.find('[data-ui-id="restore-archived-channel"]').exists()).toBe(false)
  expect(wrapper.get('[data-ui-id="select-canonical-channel"]').text()).toContain(
    'Use as canonical',
  )
  await wrapper.get('[data-ui-id="select-canonical-channel"]').trigger('click')
  await flushPromises()
  expect(api.selectCanonicalBranch).toHaveBeenCalledWith('project', 'feature/x', 'old-id')
  expect(api.switchChannel).toHaveBeenCalledWith('old-id')
  expect(wrapper.emitted('canonicalChanged')).toHaveLength(1)
  expect(wrapper.text()).toContain('Active channel')
})

test('refreshes canonical state and reports an error after a partial promotion failure', async () => {
  const archived = {
    ...details('old-id'),
    channel: {
      ...details('old-id').channel,
      kind: 'archived_legacy',
      archived: true,
      branch: 'feature/x',
    },
  }
  const promoted = {
    ...archived,
    channel: { ...archived.channel, kind: 'branch', archived: false },
  }
  api.getChannelDetails.mockResolvedValueOnce(archived).mockResolvedValueOnce(promoted)
  api.selectCanonicalBranch.mockRejectedValueOnce(new Error('Launcher failed after promotion'))
  const wrapper = mount(ProjectChannelDetails, {
    props: { projectId: 'project', channelId: 'old-id', generalRunning: false },
  })
  await flushPromises()
  await wrapper.get('[data-ui-id="select-canonical-channel"]').trigger('click')
  await flushPromises()
  expect(api.getChannelDetails).toHaveBeenCalledTimes(2)
  expect(wrapper.get('[role="alert"]').text()).toContain('Launcher failed after promotion')
  expect(wrapper.text()).toContain('Active channel')
  expect(wrapper.emitted('canonicalChanged')).toHaveLength(1)
})
