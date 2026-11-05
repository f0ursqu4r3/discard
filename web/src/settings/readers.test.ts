import { describe, expect, it, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import Memory from './pages/Memory.vue'
import Workspaces from './pages/Workspaces.vue'
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (name: string) =>
    name === 'settings_workspaces'
      ? [
          {
            name: 'Project',
            path: '/p',
            exists: true,
            channels: [
              {
                name: 'main',
                mode: 'Main checkout',
                archived: false,
                current: true,
                worktree: null,
              },
            ],
          },
        ]
      : [{ scope: 'global', title: 'Global', files: ['note.md'] }],
  ),
}))
describe('read-only pages', () => {
  it('lists workspace channels', async () => {
    const view = mount(Workspaces)
    await flushPromises()
    expect(view.text()).toContain('main · Main checkout · Current')
  })
  it('lists memory scope files', async () => {
    const view = mount(Memory)
    await flushPromises()
    expect(view.text()).toContain('Global (1)')
    expect(view.text()).toContain('note.md')
  })
})
