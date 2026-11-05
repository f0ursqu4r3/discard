import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vitest'
vi.mock('../bridge/api', () => ({ pluginAction: vi.fn(), confirmResolve: vi.fn(), archiveConfirm: vi.fn() }))
import { pluginAction } from '../bridge/api'
import PluginElement from './PluginElement.vue'
import Notices from '../components/Notices.vue'
import { clampNumber } from './props'
import type { ElementDto } from '../bridge/types'
const element = (tag: string, props: Record<string, string | number | boolean> = {}, children: ElementDto['children'] = []): ElementDto => ({ tag, props, children })

test.each(['row', 'column', 'text', 'markdown', 'button', 'avatar', 'badge', 'divider', 'spacer', 'dot'])('renders the %s tag', tag => {
  const wrapper = mount(PluginElement, { props: { plugin: 'demo', element: element(tag, { name: 'Agent', count: 2 }, [{ Text: 'hello' }]) } })
  expect(wrapper.find(`[data-plugin-tag="${tag}"]`).exists()).toBe(true)
})
test('hostile text is inert and unknown properties are ignored', () => {
  const wrapper = mount(PluginElement, { props: { plugin: 'demo', element: element('text', { style: 'display:none', class: 'evil', onclick: 'alert(1)', dangerouslySetInnerHTML: '<img>' }, [{ Text: '<img src=x onerror=alert(1)>' }]) } })
  expect(wrapper.text()).toContain('<img src=x onerror=alert(1)>')
  expect(wrapper.find('img').exists()).toBe(false)
  expect(wrapper.attributes('style')).toBeUndefined()
  expect(wrapper.classes()).not.toContain('evil')
  expect(wrapper.attributes('onclick')).toBeUndefined()
})
test('numeric props are clamped', () => { expect(clampNumber('gap', 1e9)).toBeLessThanOrEqual(400) })
test('a large valid tree renders', () => {
  const children = Array.from({ length: 1999 }, () => ({ Element: element('text', {}, [{ Text: 'x' }]) }))
  const wrapper = mount(PluginElement, { props: { plugin: 'demo', element: element('column', {}, children) } })
  expect(wrapper.findAll('[data-plugin-tag="text"]')).toHaveLength(1999)
})
test('unknown colors fall back to default', () => {
  const wrapper = mount(PluginElement, { props: { plugin: 'demo', element: element('dot', { color: 'evil' }) } })
  expect(wrapper.attributes('style') || '').not.toContain('evil')
})
test('button calls only a declared action', async () => {
  const wrapper = mount(PluginElement, { props: { plugin: 'demo', element: element('button', {}, [{ Text: 'Click' }]) } })
  await wrapper.trigger('click')
  expect(pluginAction).not.toHaveBeenCalled()
  await wrapper.setProps({ element: element('button', { on_click: 'go' }, [{ Text: 'Click' }]) })
  await wrapper.trigger('click')
  expect(pluginAction).toHaveBeenCalledWith('demo', 'go')
})
test('protected notices remain visible next to a hostile slot', () => {
  const node = document.createElement('div')
  const notices = mount(Notices, { attachTo: node, props: { banners: { reviewCount: 1, problemLines: ['bad'], confirm: 'Quit?', archive: { message: 'Archive?' }, safeMode: false }, notice: null, archived: false } })
  mount(PluginElement, { attachTo: node, props: { plugin: 'demo', element: element('text', { style: 'display:none' }, [{ Text: '<img src=x>' }]) } })
  expect(notices.findAll('[data-protected="true"]')).toHaveLength(4)
  expect(node.querySelector('img')).toBeNull()
})
