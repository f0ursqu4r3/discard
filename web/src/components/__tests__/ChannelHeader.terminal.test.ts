import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import ChannelHeader from '../ChannelHeader.vue'
import { NO_PROJECT_REASON } from '../terminal/terminal-state'

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
const views: ReturnType<typeof mount>[] = []
beforeEach(() => vi.stubGlobal('ResizeObserver', ResizeObserverStub))
afterEach(() => {
  vi.unstubAllGlobals()
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
})
function header(props: Record<string, unknown> = {}) {
  const view = mount(ChannelHeader, {
    attachTo: document.body,
    props: { channel: 'general', memberListVisible: false, ...props },
  })
  views.push(view)
  return view
}
const toggle = (view: ReturnType<typeof header>) => view.get('[data-testid="terminal-toggle"]')

test('the toggle sits in the header, shows the state, and emits a toggle', async () => {
  const view = header({ terminalVisible: false })
  expect(toggle(view).attributes('aria-label')).toBe('Show terminal')
  expect(toggle(view).attributes('aria-pressed')).toBe('false')
  expect(toggle(view).attributes('disabled')).toBeUndefined()
  expect(toggle(view).text()).not.toContain('Soon')
  await toggle(view).trigger('click')
  expect(view.emitted('toggleTerminal')).toHaveLength(1)
  await view.setProps({ terminalVisible: true })
  expect(toggle(view).attributes('aria-label')).toBe('Hide terminal')
  expect(toggle(view).attributes('aria-pressed')).toBe('true')
})

test('a direct message disables the toggle with its reason and emits nothing', async () => {
  const view = header({ directMessage: true, terminalDisabledReason: NO_PROJECT_REASON })
  expect(toggle(view).attributes('disabled')).toBeDefined()
  expect(toggle(view).attributes('aria-label')).toContain(NO_PROJECT_REASON)
  // This is a blocked state, not a planned feature. It has no Soon label.
  expect(view.text()).not.toContain('Soon')
  await toggle(view).trigger('click')
  expect(view.emitted('toggleTerminal')).toBeUndefined()
})

test('a page that is not a conversation has no terminal toggle', () => {
  const view = header({ conversation: false })
  expect(view.find('[data-testid="terminal-toggle"]').exists()).toBe(false)
})

test('the tooltip shows the key binding like the label of the control', async () => {
  const view = header({ terminalShortcut: 'Ctrl+`' })
  await view.get('span.inline-flex').trigger('focus')
  await vi.waitFor(() => expect(document.body.textContent).toContain('Show terminal (Ctrl+`)'))
})
