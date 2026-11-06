import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount, DOMWrapper } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { Select } from '@/components/ui/select'
import AddAgentDialog from './AddAgentDialog.vue'
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const claude = { id: 'claude-exact', name: 'Claude model', description: 'Claude description' }
const codex = { id: 'codex-exact', name: 'Codex model', description: 'Codex description' }
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
      if (name === 'settings_agent_defaults')
        return {
          name: 'Robin',
          avatar: null,
          provider: '',
          model: '',
          systemPrompt: '',
          policy: 'deny',
          pluginWrite: false,
        }
      if (name === 'settings_providers')
        return [
          { id: 'claude', setup: 'Ready' },
          { id: 'codex', setup: 'Ready' },
        ]
      if (name === 'settings_provider_models') return list((args as { provider: string }).provider)
      if (name === 'settings_agent_create') return 'new-agent'
    })
})
afterEach(() => {
  view?.unmount()
  vi.unstubAllGlobals()
  if (originalScroll) Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', originalScroll)
  else Reflect.deleteProperty(HTMLElement.prototype, 'scrollIntoView')
})
async function modal() {
  view = mount(AddAgentDialog, {
    props: { open: true },
    attachTo: document.body,
    global: { stubs: { DialogContent: { template: '<div><slot /></div>' } } },
  })
  await flushPromises()
  return view
}
async function choose(index: number, value: string) {
  view.findAllComponents(Select)[index]!.vm.$emit('update:modelValue', value)
  await flushPromises()
}
test('provider changes clear the selected model and save only the new provider model', async () => {
  await modal()
  expect(view.get('#new-agent-provider').attributes('role')).toBe('combobox')
  expect(view.get('#new-agent-model').attributes('role')).toBe('combobox')
  await choose(1, claude.id)
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeUndefined()
  await choose(0, 'codex')
  expect(view.findAllComponents(Select)[1]!.props('modelValue')).toBe('')
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeDefined()
  expect(invoke).toHaveBeenCalledWith('settings_provider_models', { provider: 'codex' })
  await choose(1, codex.id)
  await view.get('form').trigger('submit')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('settings_agent_create', {
    draft: expect.objectContaining({ provider: 'codex', model: codex.id }),
  })
})
test('slow previous-provider responses do not replace the current model options', async () => {
  let old!: (models: (typeof claude)[]) => void
  list = (provider) =>
    provider === 'claude'
      ? new Promise((resolve) => {
          old = resolve
        })
      : Promise.resolve([codex])
  await modal()
  expect(view.get('#new-agent-model').attributes('disabled')).toBeDefined()
  await choose(0, 'codex')
  old([claude])
  await flushPromises()
  await choose(1, claude.id)
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeDefined()
  await choose(1, codex.id)
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeUndefined()
})
test('model discovery failures and empty lists block creation and allow retry', async () => {
  list = async () => {
    throw new Error('Provider unavailable')
  }
  await modal()
  expect(view.get('[role="alert"]').text()).toContain('Provider unavailable')
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeDefined()
  list = async () => []
  await view
    .findAll('button')
    .find((b) => b.text() === 'Retry models')!
    .trigger('click')
  await flushPromises()
  expect(view.text()).toContain('no available models')
  list = async () => [claude]
  await view
    .findAll('button')
    .find((b) => b.text() === 'Retry models')!
    .trigger('click')
  await flushPromises()
  await choose(1, claude.id)
  expect(view.get('button[type="submit"]').attributes('disabled')).toBeUndefined()
})
test('the shadcn model menu supports keyboard opening and selecting a displayed model', async () => {
  await modal()
  await view.get('#new-agent-model').trigger('keydown', { key: 'Enter' })
  await flushPromises()
  const option = document.querySelector('[role="option"]')
  expect(option?.textContent).toContain(claude.name)
  await new DOMWrapper(option as Element).trigger('keydown', { key: 'Enter' })
  await flushPromises()
  expect(view.findAllComponents(Select)[1]!.props('modelValue')).toBe(claude.id)
})
