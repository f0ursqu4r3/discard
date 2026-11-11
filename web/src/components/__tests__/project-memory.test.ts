import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { invoke } from '@tauri-apps/api/core'
import MemoryBrowser from '../memory/MemoryBrowser.vue'
import WorkspacePage from '../workspace/WorkspacePage.vue'
import { Select } from '../ui/select'
import snapshotFixture from '@/bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '@/bridge/types'
import type { MemoryView } from '../memory/types'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
const item = {
  id: 'shared',
  scope: 'project:p',
  kind: 'decision',
  content: 'Use SQLite',
  version: 2,
  createdAt: 1,
  updatedAt: 2,
  deleted: false,
  sourceConversation: null,
  sourceAvailable: false,
  history: [],
}
const data: MemoryView = {
  projectId: 'p',
  items: [
    item,
    { ...item, id: 'ada', scope: 'agent-project:a:p', kind: 'fact', content: 'Release uses tags' },
    { ...item, id: 'ben', scope: 'agent-project:b:p', kind: 'instruction', content: 'Run checks' },
    { ...item, id: 'global', scope: 'global', kind: 'preference', content: 'Use short replies' },
  ],
  scopes: [
    { scope: 'project:p', title: 'Shared project memory' },
    { scope: 'agent-project:a:p', title: 'Ada', agentId: 'a' },
    { scope: 'agent-project:b:p', title: 'Ben', agentId: 'b' },
    { scope: 'global', title: 'Global · all projects' },
  ],
  proposals: [],
  responses: [],
  paused: true,
}
beforeEach(() => {
  vi.resetAllMocks()
  vi.mocked(invoke).mockImplementation(async (name) =>
    name === 'memory_get' ? structuredClone(data) : name === 'get_conversations' ? [] : undefined,
  )
})
async function click(view: ReturnType<typeof mount>, text: string) {
  const button = view.findAll('button').find((b) => b.text() === text)
  expect(button, text).toBeDefined()
  await button!.trigger('click')
  await flushPromises()
}
test.each([null, 'other'])(
  'project tab loads the selected project without an active conversation (%s)',
  async (activeProjectId) => {
    const view = mount(WorkspacePage, {
      props: {
        page: 'memory',
        messages: [],
        snapshot: {
          ...snapshotFixture,
          projectId: 'p',
          activeProjectId,
          channelId: '',
        } as SnapshotDto,
      },
    })
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('memory_get', { channelId: null, projectId: 'p' })
    expect(view.text()).toContain('Shared project memory')
    expect(view.text()).toContain('Ada')
    expect(view.text()).toContain('Ben')
    expect(view.text()).toContain('Inherited global memory')
    expect(view.text()).not.toContain('Start fresh conversation')
    expect(view.text()).not.toContain('Resume automatic capture')
    view.unmount()
  },
)
test('project memory search and filters show a clear no-match state', async () => {
  const view = mount(MemoryBrowser, { props: { mode: 'project', projectId: 'p' } })
  await flushPromises()
  const search = view.get('input[placeholder="Search memory"]')
  await search.setValue('SQLITE')
  expect(view.text()).toContain('Use SQLite')
  expect(view.text()).not.toContain('Release uses tags')
  await search.setValue('missing')
  expect(view.text()).toContain('No matching memory')
  expect(view.text()).not.toContain('No project memory yet')
  await click(view, 'Clear filters')
  const filters = view.findAllComponents(Select)
  filters[0]!.vm.$emit('update:modelValue', 'agent-project')
  filters[1]!.vm.$emit('update:modelValue', 'a')
  filters[2]!.vm.$emit('update:modelValue', 'fact')
  await flushPromises()
  expect(view.text()).toContain('Release uses tags')
  expect(view.text()).not.toContain('Run checks')
  expect(view.text()).not.toContain('Use SQLite')
  view.unmount()
})
test('explicit creation shares with the selected project and retains errors and drafts', async () => {
  const view = mount(MemoryBrowser, { props: { mode: 'project', projectId: 'p' } })
  await flushPromises()
  await click(view, 'Add memory')
  expect(view.text()).toContain('All agents in this project can use this memory')
  await view.get('textarea[aria-label="New memory text"]').setValue('New project fact')
  vi.mocked(invoke).mockRejectedValueOnce('Disk full')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toContain('Disk full')
  expect(view.get('textarea').element.value).toBe('New project fact')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('memory_create', {
    projectId: 'p',
    itemId: expect.any(String),
    kind: 'fact',
    content: 'New project fact',
  })
  const saves = vi.mocked(invoke).mock.calls.filter(([command]) => command === 'memory_create')
  expect(saves).toHaveLength(2)
  expect(saves[0]![1]).toEqual(saves[1]![1])
  expect(view.find('textarea[aria-label="New memory text"]').exists()).toBe(false)
  view.unmount()
})

test('a committed save with an index warning closes the form and reports the saved item', async () => {
  const warning = 'Memory is saved, but its search index could not update.'
  vi.mocked(invoke).mockImplementation(async (name) =>
    name === 'memory_get' ? structuredClone(data) : { warning },
  )
  const view = mount(MemoryBrowser, { props: { mode: 'project', projectId: 'p' } })
  await flushPromises()
  await click(view, 'Add memory')
  await view.get('textarea').setValue('Saved fact')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(view.find('form').exists()).toBe(false)
  expect(view.get('[role="status"]').text()).toContain(warning)
  expect(invoke).toHaveBeenCalledWith('memory_get', { channelId: null, projectId: 'p' })
  view.unmount()
})
test('project edits and approvals carry project context', async () => {
  vi.mocked(invoke).mockResolvedValueOnce({
    ...data,
    proposals: [
      {
        id: 'proposal',
        action: 'Create',
        scope: 'project:p',
        content: 'Shared decision',
        sourceConversation: 'chat',
      },
    ],
  })
  const view = mount(MemoryBrowser, { props: { mode: 'project', projectId: 'p' } })
  await flushPromises()
  await click(view, 'Inspect')
  await view.get('textarea').setValue('New text')
  await click(view, 'Save edit')
  expect(invoke).toHaveBeenCalledWith(
    'memory_mutate',
    expect.objectContaining({ channelId: null, projectId: 'p', expectedVersion: 2 }),
  )
  view.unmount()
  vi.mocked(invoke).mockResolvedValueOnce({
    ...data,
    proposals: [
      {
        id: 'proposal',
        action: 'Create',
        scope: 'project:p',
        content: 'Shared decision',
        sourceConversation: 'chat',
      },
    ],
  })
  const proposals = mount(MemoryBrowser, { props: { mode: 'project', projectId: 'p' } })
  await flushPromises()
  await click(proposals, 'Approve')
  expect(invoke).toHaveBeenCalledWith('memory_resolve', {
    id: 'proposal',
    approve: true,
    channelId: null,
    projectId: 'p',
  })
  proposals.unmount()
})
test('switching projects clears the inspector and ignores an old response', async () => {
  const view = mount(MemoryBrowser, { props: { mode: 'project', projectId: 'p' } })
  await flushPromises()
  await click(view, 'Inspect')
  await view.get('textarea').setValue('Unsaved old draft')
  let resolveOld!: (value: MemoryView) => void
  vi.mocked(invoke).mockImplementation(async (name, args) => {
    if (name !== 'memory_get') return
    if ((args as { projectId: string }).projectId === 'q')
      return new Promise<MemoryView>((resolve) => {
        resolveOld = resolve
      })
    return { ...data, projectId: 'r', items: [] }
  })
  await view.setProps({ projectId: 'q' })
  await view.setProps({ projectId: 'r' })
  await flushPromises()
  expect(view.find('textarea').exists()).toBe(false)
  resolveOld(data)
  await flushPromises()
  expect(view.text()).not.toContain('Use SQLite')
  expect(view.text()).toContain('No project memory yet')
  view.unmount()
})
test('a failed load shows an error without claiming that the project is empty', async () => {
  vi.mocked(invoke).mockRejectedValueOnce('Read failed')
  const view = mount(MemoryBrowser, { props: { mode: 'project', projectId: 'p' } })
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toContain('Read failed')
  expect(view.text()).not.toContain('No project memory yet')
  view.unmount()
})
