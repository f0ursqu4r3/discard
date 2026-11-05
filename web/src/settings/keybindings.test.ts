import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import Keybindings from './pages/Keybindings.vue'
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async (name: string) => {
  if (name === 'settings_keymap_rows') return [{ id: 'open_settings', label: 'Settings', chord: '⌘,', bound: true, custom: false, rule: 'required' }]
  throw 'Shortcut is reserved'
}) }))
describe('keybindings', () => { it('shows a conflict and keeps the previous shortcut', async () => {
  const view = mount(Keybindings); await flushPromises(); await view.get('button').trigger('click')
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'q', metaKey: true, bubbles: true }))
  await flushPromises(); expect(view.text()).toContain('Shortcut is reserved'); expect(view.text()).toContain('⌘,')
  view.unmount()
}) })
