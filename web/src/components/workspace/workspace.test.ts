import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
import WorkspacePage from './WorkspacePage.vue'
import ContextPanel from './ContextPanel.vue'
import UserBar from './UserBar.vue'
import snapshotFixture from '@/bridge/fixtures/snapshot.json'
import type { MessageDto, SnapshotDto } from '@/bridge/types'
vi.mock('@/bridge/api', () => ({ runAction: vi.fn(), contextMenu: vi.fn(() => []) }))
const snapshot = snapshotFixture as SnapshotDto
const messages: MessageDto[] = [
  {
    id: 'project-message',
    role: 'user',
    blocks: [{ type: 'markdown', text: 'Private project conversation' }],
    continuesGroup: false,
  },
]

test('DM view cannot display or send into the project conversation', async () => {
  const view = mount(WorkspacePage, { props: { page: 'direct-messages', snapshot, messages } })
  expect(view.text()).not.toContain('Private project conversation')
  expect(view.find('textarea').exists()).toBe(false)
  expect(view.text()).toContain('Direct messages are not available yet')
  await view.get('button:not([role="tab"])').trigger('click')
  expect(view.emitted('conversation')).toHaveLength(1)
})

test('DM profile hides project-specific branch and plugin content', () => {
  const view = mount(ContextPanel, { props: { snapshot, directMessage: true } })
  expect(view.find('[data-ui-id="branch-context"]').exists()).toBe(false)
  expect(view.find('[data-ui-id="context-plugin-slot"]').exists()).toBe(false)
  expect(view.text()).toContain(snapshot.agent)
  expect(view.text()).toContain('Profile')
})

test('shared user bar opens application settings when reviews are pending', async () => {
  const view = mount(UserBar, { props: { reviewCount: 3 } })
  await view.get('[aria-label="Application settings"]').trigger('click')
  expect(view.emitted('settings')).toHaveLength(1)
})
