import { describe, expect, it, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import Advanced from '../pages/Advanced.vue'
import Plugins from '../pages/Plugins.vue'
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (name: string) =>
    name === 'settings_advanced_actions'
      ? [{ id: 'clear_logs', title: 'Clear logs', lines: ['Empties the current log file.'] }]
      : {
          reviews: [
            {
              id: '1',
              name: 'plain',
              kind: 'New plugin',
              description: '',
              capabilities: ['ui'],
              changes: [],
              diff: '<img onerror=evil>',
              fullAccess: false,
              trustOffered: false,
            },
          ],
          statuses: [
            {
              name: 'changed',
              state: 'disabled: files changed outside review',
              modified: true,
              trusted: false,
            },
          ],
          problems: [],
          safeMode: false,
        },
  ),
}))
describe('security pages', () => {
  it('requires an explicit confirmation for advanced actions', async () => {
    const view = mount(Advanced)
    await flushPromises()
    expect(view.text()).not.toContain('Confirm')
    await view.get('button').trigger('click')
    expect(view.text()).toContain('Confirm')
  })
  it('renders plugin text inert and hides unavailable trust', async () => {
    const view = mount(Plugins)
    await flushPromises()
    expect(view.find('img').exists()).toBe(false)
    expect(view.text()).toContain('<img onerror=evil>')
    expect(view.text()).not.toContain('Trust future edits')
    const status = view.text().split('Installed plugins')[1]!
    expect(status).toContain('Show diff')
    expect(status).toContain('Roll back')
    expect(status).not.toContain('Approve')
  })
})
