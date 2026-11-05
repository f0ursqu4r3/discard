import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import Agents from './pages/Agents.vue'
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async (name: string) => {
  if (name === 'settings_agent_get') return [{ id: 'a', name: 'Builder', provider: 'claude', model: '', systemPrompt: 'hello', policy: 'deny', pluginWrite: false }]
  return { changes: [], choices: [], blocked: null, needsConfirm: false }
}) }))
describe('agents', () => { it('lists agents and opens their editor', async () => {
  const view = mount(Agents); await flushPromises(); await view.get('button').trigger('click'); await flushPromises()
  expect(view.text()).toContain('Agent: Builder · claude')
  expect(view.text()).toContain('Approval policy')
}) })
