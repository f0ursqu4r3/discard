import { expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { useAgentAvatar } from '../useAgentAvatar'
import BrailleSpinner from '../../BrailleSpinner.vue'

vi.mock('@/bridge/api', () => ({
  getAgents: vi.fn(async () => [
    { id: 'agent-a', avatar: null },
    { id: 'agent-b', avatar: 'lucide:bot:#61afef' },
  ]),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

test('working indicators use the full avatar catalog already loaded for the workspace', async () => {
  const view = mount(
    defineComponent({
      setup() {
        useAgentAvatar(() => 'agent-a')
        return () => h(BrailleSpinner, { agentId: 'agent-b' })
      },
    }),
  )
  await flushPromises()
  expect(view.get('[aria-label="Working"]').attributes('style')).toContain('--agent-color: #61afef')
  view.unmount()
})
