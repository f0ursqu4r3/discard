import { describe, expect, it, vi } from 'vite-plus/test'
import { mount, flushPromises } from '@vue/test-utils'
import SettingsApp from '../SettingsApp.vue'

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
const listen = vi.hoisted(() => vi.fn(async () => () => {}))
vi.mock('@tauri-apps/api/event', () => ({ listen }))
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
  // The Settings window has no view. It listens with the default target, so it gets the snapshot of every view.
  it('listens for the snapshot with no window target', async () => {
    const view = mount(SettingsApp)
    await flushPromises()
    const calls = (listen.mock.calls as unknown[][]).filter(
      ([name]) => name === 'discard://snapshot',
    )
    expect(calls).toHaveLength(1)
    expect(calls[0]).toHaveLength(2)
    view.unmount()
  })
})
