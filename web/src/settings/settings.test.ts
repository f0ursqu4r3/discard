import { describe, expect, it, vi } from 'vite-plus/test'
import { mount, flushPromises } from '@vue/test-utils'
import SettingsApp from './SettingsApp.vue'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (name: string, args: { query?: string }) => {
    if (name === 'settings_search')
      return args.query === 'theme'
        ? [{ title: 'Appearance', available: true }]
        : [
            { title: 'General', available: true },
            { title: 'Appearance', available: true },
            { title: 'Future', available: false },
          ]
    return {}
  }),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
describe('settings sidebar', () => {
  it('searches page labels and disables unavailable pages', async () => {
    const view = mount(SettingsApp)
    await flushPromises()
    expect(view.get('button[data-page="Future"]').attributes('disabled')).toBeDefined()
    await view.get('input').setValue('theme')
    await flushPromises()
    expect(view.text()).toContain('Appearance')
    expect(view.text()).not.toContain('General')
  })
})
