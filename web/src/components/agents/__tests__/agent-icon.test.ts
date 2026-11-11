import { afterEach, expect, test } from 'vite-plus/test'
import { DOMWrapper, flushPromises, mount } from '@vue/test-utils'
import AgentIconPicker from './AgentIconPicker.vue'
import AgentAvatar from './AgentAvatar.vue'
import {
  formatAgentIcon,
  hashedSlot,
  resolveColor,
  contrastRatio,
  readableOn,
  agentIdentity,
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
  expect(picker.get('[data-agent-icon="rocket"]').attributes('style')).toContain(
    'rgb(101, 121, 143)',
  )
  await picker.get('[aria-label="Use magenta"]').trigger('click')
  expect(picker.getComponent(AgentAvatar).props('avatar')).toBe('lucide:rocket:ansi-5')
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
  await picker.get('[aria-label="Use magenta"]').trigger('click')
  expect(picker.text()).toContain('Agent color')
  expect(parseAgentIcon(picker.getComponent(AgentAvatar).props('avatar'))).toEqual({
    icon: null,
    color: 'ansi-5',
  })
  expect(picker.get('[data-agent-color]').text()).toBe('RO')
  expect(picker.find('[data-agent-icon]').exists()).toBe(false)
  expect(picker.get('[aria-label="Choose agent icon"]').text()).toBe('Choose an icon')
  await picker.get('[aria-label="Choose agent icon"]').trigger('click')
  await body.get('input[placeholder="Search all icons…"]').setValue('rocket')
  await body.get('[aria-label="Choose rocket"]').trigger('click')
  expect(parseAgentIcon(picker.getComponent(AgentAvatar).props('avatar'))).toEqual({
    icon: 'rocket',
    color: 'ansi-5',
  })
})
test('avatars render the saved color and retain initials for invalid icon references', async () => {
  const view = mount(AgentAvatar, { props: { name: 'Robin', avatar: 'lucide:rocket:#123456' } })
  expect(view.get('[data-agent-icon="rocket"]').attributes('style')).toContain(
    'color: rgb(101, 121, 143)',
  )
  await view.setProps({ avatar: 'lucide:missing:#123456' })
  expect(view.find('[data-agent-icon]').exists()).toBe(false)
  expect(view.find('img').exists()).toBe(false)
  expect(view.text()).toBe('RO')
  view.unmount()
})

test('parses and formats palette slots', () => {
  expect(parseAgentIcon('lucide:bot:ansi-5')).toEqual({ icon: 'bot', color: 'ansi-5' })
  expect(parseAgentIcon('lucide::ANSI-2')).toEqual({ icon: null, color: 'ansi-2' })
  expect(parseAgentIcon('lucide:bot:ansi-7')).toBeNull()
  expect(formatAgentIcon('bot', 'ansi-3')).toBe('lucide:bot:ansi-3')
})
test('hashedSlot is stable and in range', () => {
  expect(hashedSlot('agent-a')).toBe(5)
  expect(hashedSlot('agent-b')).toBe(2)
  expect(hashedSlot('coordinator')).toBe(6)
  expect(hashedSlot('0194f3a2-7c1e-7b8a-9d2e-5f6a7b8c9d0e')).toBe(4)
})
test('resolveColor uses the theme token, then the fallback', () => {
  expect(resolveColor('ansi-4', { ansi_4: '#61afef' })).toBe('#61afef')
  expect(resolveColor('ansi-4', {})).toBe('#81a2be')
  expect(resolveColor('#123456', {})).toBe('#123456')
})
test('readableOn keeps a readable color', () => {
  expect(readableOn('#61afef', '#21252b', '#abb2bf', 4.5)).toBe('#61afef')
})
test('readableOn fixes a color equal to the background', () => {
  const out = readableOn('#21252b', '#21252b', '#abb2bf', 4.5)
  expect(contrastRatio(out, '#21252b')).toBeGreaterThanOrEqual(4.5)
})
test('readableOn darkens palette yellow on a light theme', () => {
  const out = readableOn('#f0c674', '#ffffff', '#000000', 4.5)
  expect(contrastRatio(out, '#ffffff')).toBeGreaterThanOrEqual(4.5)
})
test('agentIdentity hashes when nothing is stored or the avatar is an image', () => {
  const tokens = { ansi_5: '#c678dd', background: '#21252b', foreground: '#abb2bf' }
  expect(agentIdentity(null, 'agent-a', tokens).color).toBe('#c678dd')
  expect(agentIdentity('https://x/y.png', 'agent-a', tokens).color).toBe('#c678dd')
})
test('stored custom colors keep their value while names and marks gain contrast', () => {
  const identity = agentIdentity('lucide:bot:#21252b', 'agent-a', {
    background: '#21252b',
    foreground: '#abb2bf',
  })
  expect(identity.color).toBe('#21252b')
  expect(contrastRatio(identity.text, '#21252b')).toBeGreaterThanOrEqual(4.5)
  expect(contrastRatio(identity.mark, '#21252b')).toBeGreaterThanOrEqual(3)
})
test('agent avatars without a stored color are circles with a colored mark', () => {
  const view = mount(AgentAvatar, { props: { name: 'Robin', agentId: 'agent-a' } })
  const mark = view.get('[data-agent-mark]')
  expect(mark.classes()).toContain('rounded-full')
  expect((mark.element as HTMLElement).style.color).toBe('rgb(178, 148, 187)')
  expect(mark.attributes('style')).toContain('color-mix')
  view.unmount()
})
test('the picker offers six theme swatches and stores the selected slot', async () => {
  picker = mount(AgentIconPicker, {
    props: { name: 'Robin', modelValue: null, fallbackColor: 'ansi-5' },
  })
  expect(
    picker.findAll('[aria-label^="Use "]').map((item) => item.attributes('aria-label')),
  ).toEqual(['Use red', 'Use green', 'Use yellow', 'Use blue', 'Use magenta', 'Use cyan'])
  expect(picker.get('[aria-label="Use magenta"]').attributes('aria-pressed')).toBe('true')
  await picker.get('[aria-label="Use magenta"]').trigger('click')
  expect(picker.emitted('update:modelValue')).toEqual([['lucide::ansi-5']])
})

test('avatar marks keep three-to-one contrast on their final tinted surface', () => {
  const identity = agentIdentity('lucide:bot:#8f8f8f', 'agent', {
    background: '#ffffff',
    foreground: '#000000',
  })
  expect(identity.mark).toBe('#8f8f8f')
  // Fifteen percent of this mark over white rounds to #eeeeee.
  expect(contrastRatio(identity.avatarMark, '#eeeeee')).toBeGreaterThanOrEqual(3)
})
