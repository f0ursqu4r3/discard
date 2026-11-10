import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import AgentProfileEditor from './AgentProfileEditor.vue'
import AgentIconPicker from './AgentIconPicker.vue'
import AgentProfileDialog from './AgentProfileDialog.vue'
import AddAgentDialog from './AddAgentDialog.vue'
import { Select } from '@/components/ui/select'
const original = {
  id: 'a',
  name: 'Robin',
  avatar: 'old.png',
  provider: 'claude',
  model: 'exact/model-v1',
  systemPrompt: '',
  policy: 'deny',
  pluginWrite: false,
  workspaceWrite: false,
  sandboxAutorun: false,
  sandboxAutorunUnavailable: null as string | null,
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
      if (name === 'settings_provider_models')
        return [
          { id: 'same-model', name: 'Same model', description: '' },
          { id: 'unavailable', name: 'Unavailable', description: '' },
          { id: 'provider/exact-ID:2026', name: 'New model', description: '' },
          { id: 'unsaved-model', name: 'Unsaved model', description: '' },
          { id: 'exact/new-model', name: 'Updated model', description: '' },
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
const dialogOptions = { global: { stubs: { DialogContent: { template: '<div><slot /></div>' } } } }
async function editor() {
  const view = mount(AgentProfileEditor, { props: { agent: { ...original } } })
  await flushPromises()
  return view
}
test('edits identity, exact model, optional personality and avatar without restart', async () => {
  const view = await editor()
  await view.get('#agent-name').setValue('River')
  expect(view.find('#agent-avatar').exists()).toBe(false)
  view.getComponent(AgentIconPicker).vm.$emit('update:modelValue', 'lucide:rocket:#123456')
  view.findAllComponents(Select)[0]!.vm.$emit('update:modelValue', 'codex')
  await flushPromises()
  view.findAllComponents(Select)[1]!.vm.$emit('update:modelValue', 'provider/exact-ID:2026')
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
        avatar: 'lucide:rocket:#123456',
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
  const view = mount(AddAgentDialog, { props: { open: true }, ...dialogOptions })
  await flushPromises()
  expect((view.get('#new-agent-name').element as HTMLInputElement).value).toBe('Alex')
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeDefined()
  await view.get('#new-agent-name').setValue('Sam')
  view.findAllComponents(Select)[1]!.vm.$emit('update:modelValue', 'same-model')
  await flushPromises()
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
  await view.setProps({ agent: { ...original, archived: true } })
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
  view.findAllComponents(Select)[1]!.vm.$emit('update:modelValue', 'unsaved-model')
  await flushPromises()
  vi.mocked(invoke).mockRejectedValueOnce('Model is unavailable')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toBe('Model is unavailable')
  expect(view.findAllComponents(Select)[1]!.props('modelValue')).toBe('unsaved-model')
})

test('a profile opens in view mode and edits only after Edit profile', async () => {
  const view = mount(AgentProfileDialog, { props: { agentId: 'a' }, ...dialogOptions })
  await flushPromises()
  expect(view.text()).toContain('Robin')
  expect(view.find('form').exists()).toBe(false)
  await view
    .findAll('button')
    .find((b) => b.text() === 'Edit profile')!
    .trigger('click')
  await flushPromises()
  await view.get('#agent-name').setValue('River')
  await flushPromises()
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(view.find('form').exists()).toBe(false)
  expect(view.text()).toContain('River')
  expect(view.text()).toContain('Saved. Changes apply to the next response.')
  await view
    .findAll('button')
    .find((b) => b.text() === 'Message')!
    .trigger('click')
  expect(view.emitted('message')).toEqual([['a']])
  view.unmount()
})
test('failed creation keeps the entered name and model', async () => {
  const view = mount(AddAgentDialog, { props: { open: true }, ...dialogOptions })
  await flushPromises()
  await view.get('#new-agent-name').setValue('River')
  view.findAllComponents(Select)[1]!.vm.$emit('update:modelValue', 'unavailable')
  await flushPromises()
  vi.mocked(invoke).mockRejectedValueOnce('Model is unavailable')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(view.text()).toContain('Model is unavailable')
  expect((view.get('#new-agent-name').element as HTMLInputElement).value).toBe('River')
  expect(view.emitted('created')).toBeUndefined()
  view.unmount()
})
test('the plugin write switch says it works only in a builder session', async () => {
  const view = await editor()
  const toggle = view.get('#agent-plugin-write')
  const description = view.get(`#${toggle.attributes('aria-describedby')}`)
  expect(description.text()).toContain('Works only in a builder session.')
  expect(description.get('code').text()).toBe('discard --builder')
  expect(description.text()).toContain('Normal chats cannot write plugins.')
  view.unmount()
})

test('permission expansion still requires confirmation in the profile', async () => {
  const view = await editor()
  view.findAllComponents(Select)[1]!.vm.$emit('update:modelValue', 'exact/new-model')
  await flushPromises()
  vi.mocked(invoke).mockRejectedValueOnce('Confirmation is required')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(view.emitted('saved')).toBeUndefined()
  await view
    .findAll('button')
    .find((b) => b.text() === 'Confirm and save')!
    .trigger('click')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_save',
    expect.objectContaining({ id: 'a', confirmed: true }),
  )
  expect(view.emitted('saved')).toHaveLength(1)
  view.unmount()
})

test('disables autorun until file edits are on', async () => {
  const view = await editor()
  const autorun = view.get('#agent-sandbox-autorun')
  expect(autorun.attributes('disabled')).toBeDefined()
  expect(view.text()).toContain('Turn on file edits first.')
  await view.get('#agent-workspace-write').trigger('click')
  await flushPromises()
  expect(view.get('#agent-sandbox-autorun').attributes('disabled')).toBeUndefined()
  expect(view.text()).not.toContain('Turn on file edits first.')
})
test('shows autorun off and sends no autorun value while file edits are off', async () => {
  const agent = { ...original, workspaceWrite: false, sandboxAutorun: true }
  const view = mount(AgentProfileEditor, { props: { agent } })
  await flushPromises()
  expect(view.get('#agent-sandbox-autorun').attributes('aria-checked')).toBe('false')
  const review = vi.mocked(invoke).mock.calls.filter(([name]) => name === 'settings_agent_review')
  for (const [, args] of review)
    expect((args as { draft: Record<string, unknown> }).draft).not.toHaveProperty('sandboxAutorun')
  view.unmount()
})
test('sends the autorun choice once file edits are on', async () => {
  const agent = { ...original, workspaceWrite: true, sandboxAutorun: true }
  const view = mount(AgentProfileEditor, { props: { agent } })
  await flushPromises()
  expect(view.get('#agent-sandbox-autorun').attributes('aria-checked')).toBe('true')
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_review',
    expect.objectContaining({ draft: expect.objectContaining({ sandboxAutorun: true }) }),
  )
  view.unmount()
})
test('labels autorun Soon for providers without support', async () => {
  const agent = { ...original, workspaceWrite: true, sandboxAutorunUnavailable: 'No sandbox.' }
  const view = mount(AgentProfileEditor, { props: { agent } })
  await flushPromises()
  expect(view.get('#agent-sandbox-autorun').attributes('disabled')).toBeDefined()
  expect(view.text()).toContain('Soon')
  expect(view.text()).toContain('No sandbox.')
  view.unmount()
})
