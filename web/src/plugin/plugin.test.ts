import { enableAutoUnmount, mount } from '@vue/test-utils'
import { afterEach, expect, test, vi } from 'vite-plus/test'
import { nextTick } from 'vue'
vi.mock('../bridge/api', () => ({
  pluginAction: vi.fn(),
  confirmResolve: vi.fn(),
  archiveConfirm: vi.fn(),
}))
import { pluginAction } from '../bridge/api'
import PluginElement from './PluginElement.vue'
import Notices from '../components/Notices.vue'
import { clampNumber } from './props'
import type { ElementDto } from '../bridge/types'
import AgentAvatar from '../components/agents/AgentAvatar.vue'
import BrailleSpinner from '../components/BrailleSpinner.vue'
import { agentAvatars } from '../components/agents/useAgentAvatar'
import { agentIdentity } from '../components/agents/agent-icon'
import { STATIC_FRAME } from '../components/spinner'
import { activeTheme, fallbackTokens } from '../bridge/theme'
enableAutoUnmount(afterEach)
afterEach(() => {
  agentAvatars.value = {}
  activeTheme.value = null
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})
const element = (
  tag: string,
  props: Record<string, string | number | boolean> = {},
  children: ElementDto['children'] = [],
): ElementDto => ({ tag, props, children })

test.each([
  'row',
  'column',
  'text',
  'markdown',
  'button',
  'avatar',
  'badge',
  'divider',
  'spacer',
  'dot',
  'spinner',
])('renders the %s tag', (tag) => {
  const wrapper = mount(PluginElement, {
    props: {
      plugin: 'demo',
      element: element(tag, { name: 'Agent', count: 2 }, [{ Text: 'hello' }]),
    },
  })
  expect(wrapper.find(`[data-plugin-tag="${tag}"]`).exists()).toBe(true)
})
test('hostile text is inert and unknown properties are ignored', () => {
  const wrapper = mount(PluginElement, {
    props: {
      plugin: 'demo',
      element: element(
        'text',
        {
          style: 'display:none',
          class: 'evil',
          onclick: 'alert(1)',
          dangerouslySetInnerHTML: '<img>',
        },
        [{ Text: '<img src=x onerror=alert(1)>' }],
      ),
    },
  })
  expect(wrapper.text()).toContain('<img src=x onerror=alert(1)>')
  expect(wrapper.find('img').exists()).toBe(false)
  expect(wrapper.attributes('style')).toBeUndefined()
  expect(wrapper.classes()).not.toContain('evil')
  expect(wrapper.attributes('onclick')).toBeUndefined()
})
test('numeric props are clamped', () => {
  expect(clampNumber('gap', 1e9)).toBeLessThanOrEqual(400)
})
test('avatars use the shared catalog and update when an agent changes its icon', async () => {
  agentAvatars.value = { builder: 'lucide:bot:ansi-3' }
  const wrapper = mount(PluginElement, {
    props: { plugin: 'demo', element: element('avatar', { agent: 'builder', name: 'Builder' }) },
  })
  const avatar = wrapper.getComponent(AgentAvatar)
  expect(avatar.props()).toMatchObject({ agentId: 'builder', avatar: 'lucide:bot:ansi-3' })
  expect(avatar.get('[data-agent-mark]').attributes('style')).toContain(
    agentIdentity('lucide:bot:ansi-3', 'builder', {}).mark,
  )
  agentAvatars.value = { builder: 'lucide:bot:ansi-6' }
  await nextTick()
  expect(avatar.props('avatar')).toBe('lucide:bot:ansi-6')
  expect(avatar.get('[data-agent-mark]').attributes('style')).toContain(
    agentIdentity('lucide:bot:ansi-6', 'builder', {}).mark,
  )
})
test.each([
  { name: 'Alpha' },
  { name: 'Beta' },
  { name: 'Unknown Agent', agent: 'missing' },
  { name: 'Unknown Agent', agent: 'toString' },
  { name: 'Unknown Agent', agent: '__proto__' },
])('avatars without a stored icon use tinted initials: %j', (props) => {
  const wrapper = mount(PluginElement, {
    props: {
      plugin: 'demo',
      element: element('avatar', {
        name: props.name,
        ...(props.agent ? { agent: props.agent } : {}),
      }),
    },
  })
  const avatar = wrapper.getComponent(AgentAvatar)
  expect(avatar.props('agentId')).toBe(props.agent ?? props.name)
  expect(avatar.get('[data-agent-mark]').classes()).toContain('rounded-full')
  expect(avatar.get('[data-agent-mark]').attributes('style')).toContain('color-mix(')
  expect(avatar.get('[data-agent-mark]').attributes('style')).toContain(
    agentIdentity(null, props.agent ?? props.name, {}).mark,
  )
  expect(avatar.text()).toBe(
    props.name === 'Unknown Agent' ? 'UA' : props.name.slice(0, 2).toUpperCase(),
  )
})
test.each([
  ['label', ['section-label']],
  ['mono', ['font-mono', 'text-[0.92em]']],
  ['unknown', []],
])('text variant %s uses only its shared utility', (variant, classes) => {
  const wrapper = mount(PluginElement, {
    props: {
      plugin: 'demo',
      element: element('text', { variant: variant as string }, [{ Text: '<b>Agents</b>' }]),
    },
  })
  expect(wrapper.classes()).toEqual(classes)
  expect(wrapper.text()).toBe('<b>Agents</b>')
  expect(wrapper.find('b').exists()).toBe(false)
})
test.each(['row', 'column'])(
  'an agent %s supplies reactive identity colors to descendants',
  async (tag) => {
    agentAvatars.value = { builder: 'lucide:bot:ansi-3' }
    const wrapper = mount(PluginElement, {
      props: {
        plugin: 'demo',
        element: element(tag, { agent: 'builder', color: 'agent' }, [
          { Element: element('text', { color: 'agent' }, [{ Text: 'Builder' }]) },
          { Element: element('dot', { color: 'agent' }) },
          { Element: element('spinner') },
        ]),
      },
    })
    const identity = agentIdentity('lucide:bot:ansi-3', 'builder', {})
    expect(wrapper.attributes('style')).toContain(`--agent-color: ${identity.mark}`)
    expect(wrapper.attributes('style')).toContain(`--agent-text: ${identity.text}`)
    expect(wrapper.attributes('style')).toContain('color: var(--agent-text, var(--primary))')
    expect(wrapper.get('[data-plugin-tag="text"]').attributes('style')).toContain(
      'color: var(--agent-text, var(--primary))',
    )
    expect(wrapper.get('[data-plugin-tag="dot"]').attributes('style')).toContain(
      'background-color: var(--agent-color, var(--primary))',
    )
    expect(wrapper.getComponent(BrailleSpinner).classes()).toContain('text-agent')
    expect(wrapper.getComponent(BrailleSpinner).attributes('style')).toBeUndefined()
    activeTheme.value = {
      tokens: { ...fallbackTokens, ansi_3: fallbackTokens.ansi_6! },
      radius: null,
      fontScale: 1,
      spacingScale: 1,
      mode: 'dark',
      protectedBg: '',
      protectedFg: '',
      protectedBorder: '',
    }
    await nextTick()
    const updated = agentIdentity('lucide:bot:ansi-3', 'builder', activeTheme.value.tokens)
    expect(wrapper.attributes('style')).toContain(`--agent-color: ${updated.mark}`)
    expect(wrapper.attributes('style')).toContain(`--agent-text: ${updated.text}`)
  },
)
test('unknown agent ids get identity colors and agent colors have primary fallbacks', () => {
  const wrapper = mount(PluginElement, {
    props: { plugin: 'demo', element: element('row', { agent: 'missing' }) },
  })
  expect(wrapper.attributes('style')).toContain(
    `--agent-color: ${agentIdentity(null, 'missing', {}).mark}`,
  )
  for (const tag of ['text', 'dot']) {
    const view = mount(PluginElement, {
      props: { plugin: 'demo', element: element(tag, { color: 'agent' }) },
    })
    expect(view.attributes('style')).toContain('var(--primary)')
  }
})
test.each([undefined, 'Syncing'])(
  'spinner uses the shared accessible component with label %s',
  (label) => {
    const wrapper = mount(PluginElement, {
      props: { plugin: 'demo', element: element('spinner', label ? { label } : {}) },
    })
    expect(wrapper.findComponent(BrailleSpinner).exists()).toBe(true)
    expect(wrapper.get('[role="img"]').attributes('aria-label')).toBe(label ?? 'Working')
  },
)
test('plugin spinners stay static under reduced motion', () => {
  vi.stubGlobal('matchMedia', () => Object.assign(new EventTarget(), { matches: true }))
  const start = vi.spyOn(globalThis, 'setInterval')
  const wrapper = mount(PluginElement, {
    props: { plugin: 'demo', element: element('spinner') },
  })
  expect(wrapper.text()).toBe(STATIC_FRAME)
  expect(start).not.toHaveBeenCalled()
})
test('badges use mono tabular counts', () => {
  const wrapper = mount(PluginElement, {
    props: { plugin: 'demo', element: element('badge', { count: 12 }) },
  })
  expect(wrapper.classes()).toContain('font-mono')
  expect(wrapper.classes()).toContain('tabular-nums')
  expect(wrapper.text()).toBe('12')
})
test.each(['ansi_1', 'ansi_2', 'ansi_3', 'ansi_4', 'ansi_5', 'ansi_6'])(
  '%s is accepted as an element color',
  (color) => {
    const wrapper = mount(PluginElement, {
      props: { plugin: 'demo', element: element('text', { color }) },
    })
    expect(wrapper.attributes('style')).toBe(`color: var(--${color.replace('_', '-')});`)
  },
)
test('a large valid tree renders', () => {
  const children = Array.from({ length: 1999 }, () => ({
    Element: element('text', {}, [{ Text: 'x' }]),
  }))
  const wrapper = mount(PluginElement, {
    props: { plugin: 'demo', element: element('column', {}, children) },
  })
  expect(wrapper.findAll('[data-plugin-tag="text"]')).toHaveLength(1999)
})
test('unknown colors fall back to default', () => {
  const wrapper = mount(PluginElement, {
    props: { plugin: 'demo', element: element('dot', { color: 'evil' }) },
  })
  expect(wrapper.attributes('style') || '').not.toContain('evil')
})
test('button calls only a declared action', async () => {
  const wrapper = mount(PluginElement, {
    props: { plugin: 'demo', element: element('button', {}, [{ Text: 'Click' }]) },
  })
  await wrapper.trigger('click')
  expect(pluginAction).not.toHaveBeenCalled()
  await wrapper.setProps({ element: element('button', { on_click: 'go' }, [{ Text: 'Click' }]) })
  await wrapper.trigger('click')
  expect(pluginAction).toHaveBeenCalledWith('demo', 'go')
})
test('protected notices remain visible next to a hostile slot', () => {
  const node = document.createElement('div')
  const notices = mount(Notices, {
    attachTo: node,
    props: {
      banners: {
        reviewCount: 1,
        problemLines: ['bad'],
        confirm: 'Quit?',
        archive: { message: 'Archive?' },
        safeMode: false,
      },
      notice: null,
      archived: false,
    },
  })
  mount(PluginElement, {
    attachTo: node,
    props: {
      plugin: 'demo',
      element: element('text', { style: 'display:none' }, [{ Text: '<img src=x>' }]),
    },
  })
  expect(notices.findAll('[data-protected="true"]')).toHaveLength(4)
  expect(node.querySelector('img')).toBeNull()
})
