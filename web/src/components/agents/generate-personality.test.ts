import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import AgentProfileEditor from './AgentProfileEditor.vue'
import ReasoningSelect from './ReasoningSelect.vue'
import { Select } from '@/components/ui/select'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const original = {
  id: 'a',
  name: 'Robin',
  avatar: null,
  provider: 'claude',
  model: 'saved-model',
  reasoningEffort: null,
  systemPrompt: 'You are patient.',
  policy: 'deny',
  pluginWrite: false,
  workspaceWrite: false,
  sandboxAutorun: false,
  sandboxAutorunUnavailable: null,
  archived: false,
}
let generate: () => Promise<string>
let view: VueWrapper
beforeEach(() => {
  generate = async () => 'You are direct.'
  vi.mocked(invoke)
    .mockReset()
    .mockImplementation(async (command) => {
      if (command === 'settings_providers')
        return [
          { id: 'claude', setup: 'Ready' },
          { id: 'codex', setup: 'Ready' },
        ]
      if (command === 'settings_provider_models')
        return [
          {
            id: 'draft-model',
            name: 'Draft model',
            description: '',
            reasoningEfforts: [{ value: 'high', description: '' }],
          },
        ]
      if (command === 'settings_agent_review')
        return {
          changes: [{ field: 'Name', before: 'Robin', after: 'River' }],
          choices: [],
          blocked: null,
          needsConfirm: false,
        }
      if (command === 'settings_agent_generate_personality') return generate()
      return undefined
    })
})
afterEach(() => {
  view?.unmount()
})
async function editor(model = original.model) {
  view = mount(AgentProfileEditor, {
    props: { agent: { ...original, model } },
    attachTo: document.body,
  })
  await flushPromises()
}
function button(label: string) {
  return view.findAll('button').find((item) => item.text() === label)!
}
async function describe() {
  await button('Generate').trigger('click')
  await flushPromises()
  await view.get('#agent-personality-description').setValue('terse reviewer')
}
function generateButton() {
  return view
    .findAll('button')
    .filter((item) => item.text() === 'Generate')
    .at(-1)!
}
function personality() {
  return (view.get('#agent-prompt').element as HTMLTextAreaElement).value
}
function deferred() {
  let resolve!: (value: string) => void
  generate = () =>
    new Promise<string>((done) => {
      resolve = done
    })
  return (value: string) => resolve(value)
}

test('generation uses every draft value, fills Personality, focuses it, and does not save', async () => {
  await editor()
  await view.get('#agent-name').setValue('River')
  view.findAllComponents(Select)[0]!.vm.$emit('update:modelValue', 'codex')
  await flushPromises()
  view.findAllComponents(Select)[1]!.vm.$emit('update:modelValue', 'draft-model')
  await flushPromises()
  view.getComponent(ReasoningSelect).vm.$emit('update:modelValue', 'high')
  await view.get('#agent-prompt').setValue('You are careful.')
  await describe()
  expect(document.activeElement?.id).toBe('agent-personality-description')
  await generateButton().trigger('click')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('settings_agent_generate_personality', {
    provider: 'codex',
    model: 'draft-model',
    reasoningEffort: 'high',
    name: 'River',
    description: 'terse reviewer',
    current: 'You are careful.',
  })
  expect(personality()).toBe('You are direct.')
  expect(document.activeElement?.id).toBe('agent-prompt')
  expect(view.find('#agent-personality-description').exists()).toBe(false)
  expect(vi.mocked(invoke).mock.calls.some(([name]) => name === 'settings_agent_save')).toBe(false)
})

test('Undo restores the previous personality and manual edits remove Undo', async () => {
  await editor()
  await describe()
  await generateButton().trigger('click')
  await flushPromises()
  await button('Undo').trigger('click')
  expect(personality()).toBe(original.systemPrompt)
  expect(button('Undo')).toBeUndefined()
  await describe()
  await generateButton().trigger('click')
  await flushPromises()
  await view.get('#agent-prompt').setValue('My own text')
  expect(button('Undo')).toBeUndefined()
})

test('failure stays in the Personality field and keeps the description and previous text', async () => {
  generate = async () => {
    throw 'The model is unavailable.'
  }
  await editor()
  await describe()
  await generateButton().trigger('click')
  await flushPromises()
  const input = view.get('#agent-personality-description')
  expect((input.element as HTMLInputElement).value).toBe('terse reviewer')
  expect(view.get('[role="alert"]').text()).toBe('The model is unavailable.')
  expect(personality()).toBe(original.systemPrompt)
  expect(view.get('#agent-prompt').attributes('disabled')).toBeUndefined()
})

test('a result after the model changed and changed back is discarded', async () => {
  const resolve = deferred()
  await editor()
  await describe()
  await generateButton().trigger('click')
  view.findAllComponents(Select)[1]!.vm.$emit('update:modelValue', 'draft-model')
  view.findAllComponents(Select)[1]!.vm.$emit('update:modelValue', 'saved-model')
  await flushPromises()
  resolve('stale text')
  await flushPromises()
  expect(personality()).toBe(original.systemPrompt)
  expect(button('Undo')).toBeUndefined()
})

test('Generate is disabled without a valid model', async () => {
  await editor('')
  expect(button('Generate').attributes('disabled')).toBeDefined()
  expect(view.text()).toContain('Choose a model to generate a personality.')
})

test('Enter runs generation without submitting the profile form', async () => {
  await editor()
  await describe()
  const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
  view.get('#agent-personality-description').element.dispatchEvent(event)
  await flushPromises()
  expect(event.defaultPrevented).toBe(true)
  expect(personality()).toBe('You are direct.')
  expect(vi.mocked(invoke).mock.calls.some(([name]) => name === 'settings_agent_save')).toBe(false)
})

test('Cancel discards a running result while Save and other fields remain available', async () => {
  const resolve = deferred()
  await editor()
  await describe()
  await generateButton().trigger('click')
  expect(view.get('#agent-prompt').attributes('disabled')).toBeDefined()
  expect(view.get('#agent-name').attributes('disabled')).toBeUndefined()
  expect(button('Save profile').attributes('disabled')).toBeUndefined()
  expect(button('Generating…').attributes('aria-busy')).toBe('true')
  await button('Cancel').trigger('click')
  resolve('cancelled text')
  await flushPromises()
  expect(personality()).toBe(original.systemPrompt)
  expect(view.find('#agent-personality-description').exists()).toBe(false)
})

test('Escape hides generation and does not reach the dialog', async () => {
  await editor()
  await describe()
  const outer = vi.fn()
  view.element.addEventListener('keydown', outer)
  const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
  view.get('#agent-personality-description').element.dispatchEvent(event)
  await flushPromises()
  expect(event.defaultPrevented).toBe(true)
  expect(outer).not.toHaveBeenCalled()
  expect(view.find('#agent-personality-description').exists()).toBe(false)
})

test('a disposed editor ignores a pending result', async () => {
  const resolve = deferred()
  await editor()
  await describe()
  await generateButton().trigger('click')
  view.unmount()
  resolve('too late')
  await flushPromises()
  expect(view.emitted('saved')).toBeUndefined()
})
