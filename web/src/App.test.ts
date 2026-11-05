import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vitest'
vi.mock('./bridge/store', () => ({ useStore: () => ({ snapshot: null, history: [] }) }))
import App from './App.vue'

test('renders the app root', () => {
  expect(mount(App).find('[data-testid="app-root"]').exists()).toBe(true)
})
