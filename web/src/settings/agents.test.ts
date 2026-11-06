import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import Agents from './pages/Agents.vue'
const original = {
  id: 'a',
  name: 'Robin',
  avatar: 'old.png',
  provider: 'claude',
  model: 'exact/model-v1',
  systemPrompt: '',
  policy: 'deny',
  pluginWrite: false,
  archived: false,
}
const rows = { agents: [structuredClone(original)] }
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
beforeEach(() => {
  rows.agents = [structuredClone(original)]
  vi.mocked(invoke)
    .mockReset()
    .mockImplementation(async (name, args) => {
      const payload = args as Record<string, unknown> | undefined
      if (name === 'settings_agent_get') return rows.agents
      if (name === 'settings_providers')
        return [
          { id: 'claude', setup: 'Ready' },
          { id: 'codex', setup: 'Ready' },
        ]
      if (name === 'settings_agent_defaults')
        return { ...original, name: 'Alex', avatar: null, provider: '', model: '' }
      if (name === 'settings_agent_review')
        return {
          changes: [{ field: 'Name', before: 'Robin', after: 'River' }],
          choices: [{ policy: 'deny', enabled: true, reason: null }],
          blocked: null,
          needsConfirm: false,
        }
      if (name === 'settings_agent_create') {
        rows.agents.push({ ...(payload?.draft as typeof original), id: 'b' })
        return 'b'
      }
      if (name === 'settings_agent_save') {
        rows.agents[0] = { ...(payload?.draft as typeof original) }
        return
      }
      if (name === 'settings_agent_archive') {
        rows.agents[0]!.archived = Boolean(payload?.archived)
        return
      }
      return undefined
    })
})
async function editor() {
  const view = mount(Agents)
  await flushPromises()
  await view
    .findAll('button')
    .find((b) => b.text().includes('Robin'))!
    .trigger('click')
  await flushPromises()
  return view
}
test('edits identity, exact model, optional personality and avatar without restart', async () => {
  const view = await editor()
  await view.get('#agent-name').setValue('River')
  await view.get('#agent-provider').setValue('codex')
  await view.get('#agent-model').setValue('provider/exact-ID:2026')
  await view.get('#agent-prompt').setValue('')
  await flushPromises()
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_save',
    expect.objectContaining({
      id: 'a',
      draft: expect.objectContaining({
        name: 'River',
        avatar: 'old.png',
        provider: 'codex',
        model: 'provider/exact-ID:2026',
        systemPrompt: '',
      }),
    }),
  )
  expect(view.text()).toContain('Changes apply to the next response')
  expect(view.text()).not.toContain('Relaunch')
})
test('creates with a generated editable name and requires an explicit model', async () => {
  const view = mount(Agents)
  await flushPromises()
  await view
    .findAll('button')
    .find((b) => b.text() === 'Create agent')!
    .trigger('click')
  await flushPromises()
  expect((view.get('#agent-name').element as HTMLInputElement).value).toBe('Alex')
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeDefined()
  await view.get('#agent-name').setValue('Sam')
  await view.get('#agent-model').setValue('same-model')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('settings_agent_create', {
    draft: expect.objectContaining({
      name: 'Sam',
      provider: 'claude',
      model: 'same-model',
      systemPrompt: '',
    }),
  })
})
test('archive and restore preserve editable data; delete choices are independent', async () => {
  const view = await editor()
  await view
    .findAll('button')
    .find((b) => b.text() === 'Archive agent')!
    .trigger('click')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('settings_agent_archive', { id: 'a', archived: true })
  await view
    .findAll('button')
    .find((b) => b.text() === 'Restore agent')!
    .trigger('click')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('settings_agent_archive', { id: 'a', archived: false })
  await view
    .findAll('button')
    .find((b) => b.text() === 'Delete agent')!
    .trigger('click')
  await view.findAll('input[type="checkbox"]')[1]!.setValue(true)
  await view
    .findAll('button')
    .find((b) => b.text() === 'Delete permanently')!
    .trigger('click')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('settings_agent_delete', {
    id: 'a',
    deleteConversations: false,
    deletePrivateMemory: true,
  })
})
test('failed save keeps the unsaved draft and shows the real error', async () => {
  const view = await editor()
  await view.get('#agent-model').setValue('unsaved-model')
  await flushPromises()
  vi.mocked(invoke).mockRejectedValueOnce('Model is unavailable')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toBe('Model is unavailable')
  expect((view.get('#agent-model').element as HTMLInputElement).value).toBe('unsaved-model')
})
