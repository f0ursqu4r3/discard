import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
const { menu } = vi.hoisted(() => ({
  menu: vi.fn(async () => [
    { kind: 'action', label: 'Copy', id: 'copy_channel_name', enabled: true },
  ]),
}))
vi.mock('../bridge/api', () => ({
  contextMenu: menu,
  runAction: vi.fn(),
  pluginMenuAction: vi.fn(),
}))
import ContextMenuHost from './ContextMenuHost.vue'

test('the target menu is requested on right click', async () => {
  const target = { type: 'channel' as const, id: 'c' }
  const wrapper = mount(ContextMenuHost, {
    props: { target },
    slots: { default: '<span>Channel</span>' },
  })
  await wrapper.get('span').trigger('contextmenu', { button: 2 })
  expect(menu).toHaveBeenCalledWith(target)
})

test.each([
  [{ type: 'message', id: 'm', hasText: true }, true],
  [{ type: 'code_block', messageId: 'm', block: 0 }, true],
  [{ type: 'diff_block', messageId: 'm', block: 0 }, true],
  [{ type: 'tool_call', messageId: 'm', block: 0, hasOutput: true }, true],
  [{ type: 'channel', id: 'c' }, false],
  [{ type: 'agent', id: 'a', name: 'Agent' }, false],
  [{ type: 'server', id: 's', name: 'Server', path: '/' }, false],
] as const)('the %j wrapper allows text selection: %s', (target, selectable) => {
  const wrapper = mount(ContextMenuHost, {
    props: { target },
    slots: { default: '<span>Text</span>' },
  })
  const trigger = wrapper.get('[data-slot="context-menu-trigger"]')
  expect(trigger.classes().includes('select-text')).toBe(selectable)
  expect(trigger.classes().includes('select-none')).toBe(!selectable)
})
