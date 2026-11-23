import { beforeEach, describe, expect, it, vi } from 'vite-plus/test'
import { mount, flushPromises } from '@vue/test-utils'
import General from '../General.vue'

const LABEL = 'Keep running in the background when all windows are closed'
const page = vi.hoisted(() => ({
  state: {
    defaultProvider: 'claude',
    confirmWhileWorking: true,
    keepRunningInBackground: true,
    desktopNotifications: true,
    backgroundUnavailable: null as string | null,
    providers: ['claude', 'codex'],
    issues: [] as string[],
    themeProblems: [] as string[],
  },
}))
const invoke = vi.hoisted(() =>
  vi.fn(async (name: string, args?: { patch?: Record<string, unknown> }) => {
    if (name === 'settings_get') return { ...page.state }
    if (name === 'settings_set_general') {
      Object.assign(page.state, args?.patch)
      return undefined
    }
    throw new Error(`unexpected command ${name}`)
  }),
)
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))

beforeEach(() => {
  invoke.mockClear()
  page.state.keepRunningInBackground = true
  page.state.backgroundUnavailable = null
  page.state.desktopNotifications = true
})

describe('general settings: desktop notifications', () => {
  const NOTIFY = '[role="switch"][aria-label="Desktop notifications"]'

  it('shows the Desktop notifications switch on by default', async () => {
    const view = mount(General)
    await flushPromises()
    expect(view.get(NOTIFY).attributes('aria-checked')).toBe('true')
  })

  it('sends a patch when the user turns it off', async () => {
    const view = mount(General)
    await flushPromises()
    await view.get(NOTIFY).trigger('click')
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('settings_set_general', {
      patch: { desktopNotifications: false },
    })
  })
})

describe('general settings: background mode', () => {
  it('shows the switch with its label, on by default', async () => {
    const view = mount(General)
    await flushPromises()
    expect(view.text()).toContain(LABEL)
    const toggle = view.get(`[role="switch"][aria-label="${LABEL}"]`)
    expect(toggle.attributes('aria-checked')).toBe('true')
  })

  it('sends a patch with the new value when the user turns the switch off', async () => {
    const view = mount(General)
    await flushPromises()
    await view.get(`[role="switch"][aria-label="${LABEL}"]`).trigger('click')
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('settings_set_general', {
      patch: { keepRunningInBackground: false },
    })
    expect(view.get(`[role="switch"][aria-label="${LABEL}"]`).attributes('aria-checked')).toBe(
      'false',
    )
  })

  it('shows the switch as off when the setting is off', async () => {
    page.state.keepRunningInBackground = false
    const view = mount(General)
    await flushPromises()
    expect(view.get(`[role="switch"][aria-label="${LABEL}"]`).attributes('aria-checked')).toBe(
      'false',
    )
  })

  it('shows the line of the tray problem under the switch when the tray icon is not available', async () => {
    const line =
      'The tray icon is not available: no appindicator. Discard quits when the last window closes.'
    page.state.backgroundUnavailable = line
    const view = mount(General)
    await flushPromises()
    const toggle = view.get(`[role="switch"][aria-label="${LABEL}"]`)
    const described = toggle.attributes('aria-describedby')
    expect(described).toBeTruthy()
    expect(view.get(`#${described}`).text()).toBe(line)
  })

  it('shows no tray problem when the tray icon works', async () => {
    const view = mount(General)
    await flushPromises()
    expect(view.text()).not.toContain('The tray icon is not available')
  })
})
