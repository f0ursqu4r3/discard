import { mount, flushPromises } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
const { pending, engine } = vi.hoisted(() => ({
  pending: [] as ((value: unknown) => void)[],
  engine: vi.fn(() => ({})),
}))
vi.mock('shiki', () => ({
  createJavaScriptRegexEngine: engine,
  getSingletonHighlighter: vi.fn(async () => ({
    loadLanguage: vi.fn(async () => {}),
    codeToTokens: vi.fn(() => new Promise((resolve) => pending.push(resolve))),
  })),
}))
import CodeBlock from './CodeBlock.vue'
test('a late highlight cannot replace newer streaming text', async () => {
  const block = mount(CodeBlock, { props: { text: 'old', language: 'rust' } })
  await new Promise((resolve) => setTimeout(resolve, 100))
  await flushPromises()
  expect(engine).toHaveBeenCalled()
  await block.setProps({ text: 'new' })
  await new Promise((resolve) => setTimeout(resolve, 100))
  await flushPromises()
  pending[1]!({ tokens: [[{ content: 'new', type: 0 }]] })
  await flushPromises()
  pending[0]!({ tokens: [[{ content: 'old', type: 0 }]] })
  await flushPromises()
  expect(block.text()).toContain('new')
  expect(block.text()).not.toContain('old')
})

test('rapid code updates defer expensive highlighting until the stream pauses', async () => {
  engine.mockClear()
  pending.length = 0
  const block = mount(CodeBlock, { props: { text: 'one', language: 'rust' } })
  await block.setProps({ text: 'two' })
  await block.setProps({ text: 'three' })
  await flushPromises()
  expect(engine).not.toHaveBeenCalled()
  await new Promise((resolve) => setTimeout(resolve, 120))
  expect(engine).toHaveBeenCalledTimes(1)
  block.unmount()
})
