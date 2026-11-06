import { describe, expect, it, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import Memory from './pages/Memory.vue'
import Workspaces from './pages/Workspaces.vue'
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
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
      : {
          items: [
            {
              id: 'item',
              scope: 'global',
              kind: 'fact',
              content: 'A saved fact',
              version: 1,
              createdAt: 1,
              updatedAt: 1,
              sourceConversation: null,
              sourceAvailable: false,
              history: [],
            },
          ],
          scopes: [{ scope: 'global', title: 'Global' }],
          responses: [],
          proposals: [],
          paused: false,
        },
  ),
}))
describe('storage settings pages', () => {
  it('lists workspace channels', async () => {
    const view = mount(Workspaces)
    await flushPromises()
    expect(view.text()).toContain('main · Main checkout · Current')
  })
  it('lists saved memory items', async () => {
    const view = mount(Memory)
    await flushPromises()
    expect(view.text()).toContain('All saved memory')
    expect(view.text()).toContain('A saved fact')
    expect(view.text()).toContain('global · fact · v1')
  })
})
