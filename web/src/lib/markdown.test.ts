import { expect, test } from 'vitest'
import { renderMarkdown } from './markdown'

test('raw HTML stays inert', () => {
  const html = renderMarkdown('<img src=x onerror=alert(1)>')
  expect(html).not.toContain('<img')
  expect(html).not.toContain('onerror=')
})

test('fenced code has a language class', () => {
  expect(renderMarkdown('```rust\nlet x = 1;\n```')).toContain('language-rust')
})

test('fenced code preserves markup as text', () => {
  const html = renderMarkdown('```html\n<div>hello</div>\n```')
  expect(html).toContain('&lt;div&gt;hello&lt;/div&gt;')
})

test('plain angle brackets and entities remain readable text', () => {
  expect(renderMarkdown('Use <Widget> here')).toContain('&lt;Widget&gt;')
  expect(renderMarkdown('Use &lt;Widget&gt; here')).toContain('&lt;Widget&gt;')
})
