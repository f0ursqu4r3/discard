import { describe, expect, it, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import Providers from '../pages/Providers.vue'
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => [
    {
      id: 'test',
      setup: 'Ready',
      command: null,
      capabilities: [{ label: 'Cancellation', supported: true }],
      hooks: [],
      blocked: null,
    },
  ]),
}))
describe('providers settings', () => {
  it('shows provider setup and capability rows', async () => {
    const view = mount(Providers)
    await flushPromises()
    expect(view.text()).toContain('Setup status: Ready')
    expect(view.text()).toContain('Cancellation: Yes')
  })
})
