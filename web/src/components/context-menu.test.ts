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
