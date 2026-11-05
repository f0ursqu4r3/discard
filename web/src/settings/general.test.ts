import { describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import General from './pages/General.vue'
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async (name: string) => {
  if (name === 'settings_get') return { defaultProvider: 'claude', confirmWhileWorking: true, providers: ['claude', 'codex'], issues: [] }
  throw 'invalid setting'
}) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
describe('general settings', () => {
  it('shows a rejected patch and keeps the old provider', async () => {
    const view = mount(General)
    await flushPromises()
    await view.get('button:nth-child(2)').trigger('click')
    await flushPromises()
    expect(view.get('[role="alert"]').text()).toBe('invalid setting')
    expect(view.get('button:first-child').attributes('data-slot')).toBe('button')
    expect(view.get('button:first-child').classes()).toContain('bg-primary')
  })
})
