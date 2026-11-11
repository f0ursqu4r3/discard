import { mount, flushPromises } from '@vue/test-utils'
import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import type { Highlighter } from '../ui/code-block/highlighter'
const { load } = vi.hoisted(() => ({
  load: vi.fn(),
}))
vi.mock('../ui/code-block/highlighter', () => ({
  loadHighlighter: load,
  highlightCode: vi.fn(),
}))
import CodeBlock from '../CodeBlock.vue'
import { CodeBlockCopyButton } from '../ui/code-block'

const wrappers: ReturnType<typeof mount>[] = []
const originalCopyCommand = Object.getOwnPropertyDescriptor(document, 'execCommand')
beforeEach(() => {
  load.mockReset().mockResolvedValue(null)
})
afterEach(() => {
  wrappers.splice(0).forEach((wrapper) => wrapper.unmount())
  if (originalCopyCommand) Object.defineProperty(document, 'execCommand', originalCopyCommand)
  else Reflect.deleteProperty(document, 'execCommand')
})
function highlighter(): Highlighter {
  return {
    highlight: vi.fn((text: string) => ({
      lines: text.split('\n').map((content) => (content ? [{ content }] : [])),
      state: undefined,
    })),
  }
}

test('a late language load cannot replace newer streaming text', async () => {
  const pending = new Map<string, (value: Highlighter) => void>()
  load.mockImplementation(
    (language: string) => new Promise((resolve) => pending.set(language, resolve)),
  )
  const block = mount(CodeBlock, { props: { text: 'old', language: 'rust' } })
  wrappers.push(block)
  await block.setProps({ text: 'new', language: 'typescript' })
  const current = highlighter()
  pending.get('typescript')!(current)
  await flushPromises()
  const stale = highlighter()
  pending.get('rust')!(stale)
  await flushPromises()
  expect(block.text()).toContain('new')
  expect(block.text()).not.toContain('old')
  expect(stale.highlight).not.toHaveBeenCalled()
  expect(current.highlight).toHaveBeenCalled()
})

test('streamed code reuses completed lines instead of highlighting the whole block', async () => {
  const syntax = highlighter()
  load.mockResolvedValue(syntax)
  const block = mount(CodeBlock, { props: { text: 'first\nsec', language: 'rust' } })
  wrappers.push(block)
  await flushPromises()
  vi.mocked(syntax.highlight).mockClear()
  await block.setProps({ text: 'first\nsecond\nthird' })
  await flushPromises()
  expect(vi.mocked(syntax.highlight).mock.calls.map(([text]) => text)).toEqual(['second', 'third'])
})

test('official code-block markup preserves plain text and escapes markup for unknown languages', async () => {
  const block = mount(CodeBlock, {
    props: { text: '<script>alert(1)</script>\n\nlast', language: 'unknown' },
  })
  wrappers.push(block)
  await flushPromises()
  expect(block.find('[data-slot="code-block"]').exists()).toBe(true)
  expect(block.get('[data-slot="code-block-content"]').attributes('tabindex')).toBe('0')
  expect(block.findAll('[data-slot="code-block-line"]')).toHaveLength(3)
  expect(block.find('script').exists()).toBe(false)
  expect(block.text()).toContain('<script>alert(1)</script>')
})

test('the official copy button copies the exact displayed text including blank lines', async () => {
  let copiedText: string | undefined
  const copy = vi.fn(() => {
    copiedText = document.querySelector('textarea')?.value
    return true
  })
  Object.defineProperty(document, 'execCommand', { configurable: true, value: copy })
  const text = 'git status\n\ngit log\n'
  const block = mount(CodeBlock, { props: { text, language: 'sh', copyText: true } })
  wrappers.push(block)
  await flushPromises()
  expect(block.getComponent(CodeBlockCopyButton).props('value')).toBe(text)
  await block.get('[data-slot="code-block-copy-button"]').trigger('click')
  await flushPromises()
  expect(copy).toHaveBeenCalledWith('copy')
  expect(copiedText).toBe(text)
  expect(block.get('[data-slot="code-block-copy-button"]').attributes('aria-label')).toBe('Copied')
})
