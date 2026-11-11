import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
vi.mock('./bridge/store', () => ({ useStore: () => ({ snapshot: null, history: [] }) }))
import App from './App.vue'

test('renders the app root', () => {
  expect(mount(App).find('[data-testid="app-root"]').exists()).toBe(true)
})
