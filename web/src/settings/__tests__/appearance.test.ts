import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import Appearance from './pages/Appearance.vue'
import AppearancePreview from './AppearancePreview.vue'
import ServerRail from '@/components/ServerRail.vue'
import MessageRow from '@/components/MessageRow.vue'
import { Select } from '@/components/ui/select'
import { defaultAppearance, useAppearanceSettings } from '@/bridge/appearance'

const { invoke, listen } = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen }))
let saved: Record<string, unknown>
beforeEach(() => {
  saved = {
    ...defaultAppearance,
    themeSource: 'default',
    themes: [],
    swatches: [],
    overrides: [],
    problems: [],
  }
  vi.clearAllMocks()
  listen.mockResolvedValue(() => {})
  invoke.mockImplementation(async (name: string, args?: { patch?: Record<string, unknown> }) => {
    if (name === 'settings_get') return { ...saved }
    if (name === 'settings_set_appearance') Object.assign(saved, args?.patch)
  })
})

test('keeps the current appearance by default and previews all three saved controls', async () => {
  const view = mount(Appearance)
  await flushPromises()
  const preview = () => view.get('[data-testid="appearance-preview"]')
  expect(preview().attributes('inert')).toBeDefined()
  expect(preview().get('[data-ui-id="project-rail"]').classes()).toContain('w-18')
  expect(preview().findAll('[data-testid="message-avatar"]')).toHaveLength(3)
  expect(preview().get('[data-ui-id="message-preview-agent"]').classes()).toContain('py-2')
  expect(preview().findAll('article button')).toHaveLength(0)

  await view.get('#mini-project-rail').trigger('click')
  await flushPromises()
  expect(invoke).toHaveBeenCalledWith('settings_set_appearance', {
    patch: { miniProjectRail: true },
  })
  expect(preview().get('[data-ui-id="project-rail"]').classes()).toContain('w-12')
  expect(preview().get('button[aria-label="Sample project"]').classes()).toContain('size-8')
  expect(preview().get('[data-testid="approval-badge"]').text()).toBe('1')

  view.findComponent(Select).vm.$emit('update:modelValue', 'compact')
  await flushPromises()
  expect(saved.messageDensity).toBe('compact')
  expect(preview().get('[data-ui-id="message-preview-agent"]').classes()).toContain('py-1')
  expect(preview().get('[data-ui-id="message-preview-agent"]').classes()).toContain('text-sm')

  await view.get('#show-message-avatars').trigger('click')
  await flushPromises()
  expect(saved.showMessageAvatars).toBe(false)
  expect(preview().find('[data-testid="message-avatar"]').exists()).toBe(false)
  expect(preview().text()).toContain('Assistant')
  expect(preview().text()).toContain('You')
  view.unmount()

  const reopened = mount(Appearance)
  await flushPromises()
  expect(reopened.get('#mini-project-rail').attributes('aria-checked')).toBe('true')
  expect(reopened.get('#show-message-avatars').attributes('aria-checked')).toBe('false')
  reopened.unmount()
})

test('keeps saved preview values and reports a failed write', async () => {
  const view = mount(Appearance)
  await flushPromises()
  invoke.mockRejectedValueOnce(new Error('Disk is read-only'))
  await view.get('#mini-project-rail').trigger('click')
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toContain('Disk is read-only')
  expect(view.getComponent(AppearancePreview).props('preferences').miniProjectRail).toBe(false)
  expect(view.get('#mini-project-rail').attributes('disabled')).toBeUndefined()
  view.unmount()
})

test('updates the real rail and live message from settings events and cleans up listeners', async () => {
  const stop = vi.fn()
  listen.mockResolvedValue(stop)
  const error = vi.fn()
  const view = mount(
    defineComponent({
      setup() {
        useAppearanceSettings(error)
        return () =>
          h('div', [
            h(ServerRail, {
              project: 'Project',
              unread: false,
              pendingApprovals: 0,
              compact: true,
            }),
            h(MessageRow, {
              channelId: 'channel',
              message: {
                id: 'live',
                role: 'agent',
                blocks: [{ type: 'markdown', text: 'Working' }],
                continuesGroup: false,
              },
              showCursor: true,
            }),
          ])
      },
    }),
  )
  await flushPromises()
  expect(view.get('[data-ui-id="project-rail"]').classes()).toContain('w-13')
  Object.assign(saved, {
    miniProjectRail: true,
    messageDensity: 'compact',
    showMessageAvatars: false,
  })
  await listen.mock.calls.find(([name]) => name === 'discard://settings')![1]()
  await flushPromises()
  expect(view.get('[data-ui-id="project-rail"]').classes()).toContain('w-12')
  expect(view.find('[data-testid="message-avatar"]').exists()).toBe(false)
  expect(view.get('[data-ui-id="message-live"]').classes()).toContain('py-1')
  expect(view.get('[data-testid="stream-cursor"]').classes()).toContain('stream-cursor')
  expect(error).not.toHaveBeenCalled()
  view.unmount()
  expect(stop).toHaveBeenCalledOnce()
})
