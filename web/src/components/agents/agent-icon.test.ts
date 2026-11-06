import { afterEach, expect, test } from 'vite-plus/test'
import { DOMWrapper, flushPromises, mount } from '@vue/test-utils'
import AgentIconPicker from './AgentIconPicker.vue'
import AgentAvatar from './AgentAvatar.vue'
import {
  formatAgentIcon,
  iconForeground,
  iconNames,
  parseAgentIcon,
  searchIcons,
} from './agent-icon'

const body = new DOMWrapper(document.body)
let picker: ReturnType<typeof mount> | undefined
afterEach(() => picker?.unmount())
test('the catalog includes every installed icon module and supports spaced or camel-case searches', () => {
  const installed = import.meta.glob('/node_modules/@lucide/vue/dist/esm/icons/*.mjs')
  expect(iconNames).toHaveLength(Object.keys(installed).length - 1)
  expect(iconNames.length).toBeGreaterThan(2000)
  expect(iconNames).not.toContain('index')
  expect(searchIcons('alarm clock')).toContain('alarm-clock')
  expect(searchIcons('AlarmClock')).toEqual(searchIcons('alarm clock'))
  expect(searchIcons('zoology-no-such-icon')).toEqual([])
})
test('icon references round trip and reject invalid names or colors', () => {
  expect(parseAgentIcon(formatAgentIcon('rocket', '#ABCDEF'))).toEqual({
    icon: 'rocket',
    color: '#abcdef',
  })
  expect(parseAgentIcon('https://example.com/avatar.png')).toBeNull()
  expect(parseAgentIcon('lucide:missing-icon:#ffffff')).toBeNull()
  expect(() => formatAgentIcon('rocket', 'red;display:none')).toThrow()
  expect(iconForeground('#ffffff')).toBe('#000000')
  expect(iconForeground('#000000')).toBe('#ffffff')
})
test('search chooses any matching icon, applies a custom color, and restores trigger focus', async () => {
  picker = mount(AgentIconPicker, {
    attachTo: document.body,
    props: {
      name: 'Robin',
      modelValue: null,
      'onUpdate:modelValue': (value) => picker!.setProps({ modelValue: value }),
    },
  })
  await picker.get('[aria-label="Choose agent icon"]').trigger('click')
  await flushPromises()
  expect(picker.find('[data-slot="popover-content"]').exists()).toBe(false)
  expect(body.find('[data-slot="popover-content"]').exists()).toBe(true)
  expect(body.findAll('[aria-label^="Choose "]').length).toBe(49)
  const search = body.get('input[placeholder="Search all icons…"]')
  await search.setValue('rocket')
  await body.get('[aria-label="Choose rocket"]').trigger('click')
  await flushPromises()
  expect(parseAgentIcon(picker.getComponent(AgentAvatar).props('avatar'))?.icon).toBe('rocket')
  expect(document.activeElement).toBe(picker.get('[aria-label="Choose agent icon"]').element)
  await picker.get('[aria-label="Custom agent color"]').setValue('#123456')
  expect(picker.getComponent(AgentAvatar).props('avatar')).toBe('lucide:rocket:#123456')
  expect(picker.get('[data-agent-icon="rocket"]').attributes('style')).toContain('rgb(18, 52, 86)')
  await picker.get('[aria-label="Use pink"]').trigger('click')
  expect(picker.getComponent(AgentAvatar).props('avatar')).toBe('lucide:rocket:#f472b6')
})
test('empty searches and disabled controls cannot change the icon', async () => {
  picker = mount(AgentIconPicker, { props: { name: 'Robin', modelValue: null } })
  await picker.get('[aria-label="Choose agent icon"]').trigger('click')
  await body.get('input[placeholder="Search all icons…"]').setValue('no-such-icon-xyz')
  expect(body.text()).toContain('No matching icons')
  await picker.setProps({ disabled: true })
  expect(
    picker.findAll('button').every((button) => button.attributes('disabled') !== undefined),
  ).toBe(true)
  await picker.get('[aria-label="Use blue"]').trigger('click')
  expect(picker.emitted('update:modelValue')).toBeUndefined()
})
test('agent color can change without selecting an icon', async () => {
  picker = mount(AgentIconPicker, {
    attachTo: document.body,
    props: {
      name: 'Robin',
      modelValue: null,
      'onUpdate:modelValue': (value) => picker!.setProps({ modelValue: value }),
    },
  })
  await picker.get('[aria-label="Use pink"]').trigger('click')
  expect(picker.text()).toContain('Agent color')
  expect(parseAgentIcon(picker.getComponent(AgentAvatar).props('avatar'))).toEqual({
    icon: null,
    color: '#f472b6',
  })
  expect(picker.get('[data-agent-color]').text()).toBe('RO')
  expect(picker.find('[data-agent-icon]').exists()).toBe(false)
  expect(picker.get('[aria-label="Choose agent icon"]').text()).toBe('Choose an icon')
  await picker.get('[aria-label="Choose agent icon"]').trigger('click')
  await body.get('input[placeholder="Search all icons…"]').setValue('rocket')
  await body.get('[aria-label="Choose rocket"]').trigger('click')
  expect(parseAgentIcon(picker.getComponent(AgentAvatar).props('avatar'))).toEqual({
    icon: 'rocket',
    color: '#f472b6',
  })
})
test('avatars render the saved color and retain initials for invalid icon references', async () => {
  const view = mount(AgentAvatar, { props: { name: 'Robin', avatar: 'lucide:rocket:#123456' } })
  expect(view.get('[data-agent-icon="rocket"]').attributes('style')).toContain(
    'color: rgb(255, 255, 255)',
  )
  await view.setProps({ avatar: 'lucide:missing:#123456' })
  expect(view.find('[data-agent-icon]').exists()).toBe(false)
  expect(view.find('img').exists()).toBe(false)
  expect(view.text()).toBe('RO')
  view.unmount()
})
