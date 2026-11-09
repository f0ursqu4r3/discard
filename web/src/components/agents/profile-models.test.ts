import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { DOMWrapper, flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { Select } from '@/components/ui/select'
import AgentProfileEditor from './AgentProfileEditor.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const agent = {
  id: 'a',
  name: 'Robin',
  avatar: null,
  provider: 'claude',
  model: 'saved-model',
  systemPrompt: '',
  policy: 'deny',
  pluginWrite: false,
  workspaceWrite: false,
  archived: false,
}
const claude = { id: 'claude-model', name: 'Claude model', description: '' }
const codex = { id: 'codex-model', name: 'Codex model', description: '' }
let view: ReturnType<typeof mount>
let list: (provider: string) => Promise<(typeof claude)[]>
const originalScroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollIntoView')
beforeEach(() => {
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  })
  list = async (provider) => [provider === 'claude' ? claude : codex]
  vi.mocked(invoke)
    .mockReset()
    .mockImplementation(async (name, args) => {
      if (name === 'settings_providers')
        return [
          { id: 'claude', setup: 'Ready' },
          { id: 'codex', setup: 'Ready' },
        ]
      if (name === 'settings_provider_models') return list((args as { provider: string }).provider)
      if (name === 'settings_agent_review')
        return {
          changes: [{ field: 'Name', before: 'Robin', after: 'River' }],
          choices: [],
          blocked: null,
          needsConfirm: false,
        }
      if (name === 'settings_agent_get') return [agent]
    })
})
afterEach(() => {
  view?.unmount()
  if (originalScroll) Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', originalScroll)
  else Reflect.deleteProperty(HTMLElement.prototype, 'scrollIntoView')
})
async function editor() {
  view = mount(AgentProfileEditor, { props: { agent }, attachTo: document.body })
  await flushPromises()
}
async function choose(index: number, value: string) {
  view.findAllComponents(Select)[index]!.vm.$emit('update:modelValue', value)
  await flushPromises()
}
test('the model menu supports keyboard selection and preserves an unlisted saved model', async () => {
  await editor()
  expect(view.get('#agent-provider').attributes('role')).toBe('combobox')
  expect(view.get('#agent-model').attributes('role')).toBe('combobox')
  expect(view.findAllComponents(Select)[1]!.props('modelValue')).toBe(agent.model)
  await view.get('#agent-model').trigger('keydown', { key: 'Enter' })
  await flushPromises()
  const options = [...document.querySelectorAll('[role="option"]')]
  expect(options.some((option) => option.textContent?.includes(agent.model))).toBe(true)
  const option = options.find((option) => option.textContent?.includes(claude.name))!
  await new DOMWrapper(option).trigger('keydown', { key: 'Enter' })
  await flushPromises()
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_save',
    expect.objectContaining({
      draft: expect.objectContaining({ model: claude.id, provider: 'claude' }),
    }),
  )
})
test('changing provider clears the model and ignores a late previous-provider response', async () => {
  let old!: (models: (typeof claude)[]) => void
  list = (provider) =>
    provider === 'claude'
      ? new Promise((resolve) => {
          old = resolve
        })
      : Promise.resolve([codex])
  await editor()
  await choose(0, 'codex')
  expect(view.findAllComponents(Select)[1]!.props('modelValue')).toBe('')
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeDefined()
  old([claude])
  await flushPromises()
  await choose(1, claude.id)
  await view.get('form').trigger('submit')
  expect(invoke).not.toHaveBeenCalledWith('settings_agent_save', expect.anything())
  await choose(1, codex.id)
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_save',
    expect.objectContaining({
      draft: expect.objectContaining({ provider: 'codex', model: codex.id }),
    }),
  )
})
test('failed discovery retains the saved model for profile edits and supports retry', async () => {
  list = async () => {
    throw new Error('Provider unavailable')
  }
  await editor()
  expect(view.get('[role="alert"]').text()).toContain('Provider unavailable')
  await view.get('#agent-name').setValue('River')
  await flushPromises()
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_save',
    expect.objectContaining({
      draft: expect.objectContaining({ name: 'River', model: agent.model }),
    }),
  )
  await choose(0, 'codex')
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeDefined()
  list = async () => [codex]
  await view
    .findAll('button')
    .find((button) => button.text() === 'Retry models')!
    .trigger('click')
  await flushPromises()
  expect(view.find('[role="alert"]').exists()).toBe(false)
  await choose(1, codex.id)
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeUndefined()
})

test('profile editing saves a reasoning-only change and can restore provider default', async () => {
  list = async () => [
    {
      ...claude,
      id: agent.model,
      reasoningEfforts: [
        { value: 'low', description: '' },
        { value: 'high', description: 'Think longer' },
      ],
    },
  ]
  view = mount(AgentProfileEditor, {
    props: { agent: { ...agent, reasoningEffort: 'low' } },
    attachTo: document.body,
  })
  await flushPromises()
  expect(view.findAllComponents(Select)[2]!.props('modelValue')).toBe('low')
  await choose(2, 'high')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_save',
    expect.objectContaining({
      draft: expect.objectContaining({ reasoningEffort: 'high', model: agent.model }),
    }),
  )
  await choose(2, 'default')
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_save',
    expect.objectContaining({ draft: expect.objectContaining({ reasoningEffort: null }) }),
  )
})

test('an unavailable catalogue keeps the saved reasoning level until the model changes or the user resets it', async () => {
  list = async () => {
    throw new Error('Offline')
  }
  view = mount(AgentProfileEditor, {
    props: { agent: { ...agent, reasoningEffort: 'high' } },
    attachTo: document.body,
  })
  await flushPromises()
  expect(view.findAllComponents(Select)[2]!.props('modelValue')).toBe('high')
  expect(view.findAllComponents(Select)[2]!.props('disabled')).toBe(false)
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith(
    'settings_agent_save',
    expect.objectContaining({ draft: expect.objectContaining({ reasoningEffort: 'high' }) }),
  )
  list = async () => [{ ...claude, reasoningEfforts: [] }]
  await view
    .findAll('button')
    .find((b) => b.text() === 'Retry models')!
    .trigger('click')
  await flushPromises()
  expect(view.findAllComponents(Select)[2]!.props('modelValue')).toBe('high')
  await choose(1, claude.id)
  expect(view.findAllComponents(Select)[2]!.props('modelValue')).toBe('default')
  expect(view.findAllComponents(Select)[2]!.props('disabled')).toBe(true)
})
