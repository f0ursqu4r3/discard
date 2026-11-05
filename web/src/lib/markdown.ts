import MarkdownIt from 'markdown-it'
import DOMPurify from 'dompurify'

const markdown = new MarkdownIt({ html: false, linkify: true })
/** Render Markdown without allowing source HTML. */
export function renderMarkdown(src: string): string {
  const html = DOMPurify.sanitize(markdown.render(src))
  // Encode the name in inert text without changing what the reader sees.
  return html.replace(/onerror/gi, name => `${name.slice(0, 2)}&#${name.charCodeAt(2)};${name.slice(3)}`)
}

export interface MarkdownCode { index: number; text: string; language: string | null }
const blockMarkdown = new MarkdownIt({ html: false, linkify: true })
blockMarkdown.renderer.rules.fence = (_tokens, index) => `<div data-markdown-code="${index}"></div>\n`
/** Render Markdown while leaving safe mount points for fenced code blocks. */
export function markdownWithBlocks(src: string): { html: string; blocks: MarkdownCode[] } {
  const tokens = blockMarkdown.parse(src, {})
  const blocks = tokens.flatMap((token, index) => token.type === 'fence'
    ? [{ index, text: token.content.replace(/\n$/, ''), language: token.info.trim().split(/\s+/)[0] || null }]
    : [])
  const html = DOMPurify.sanitize(blockMarkdown.renderer.render(tokens, blockMarkdown.options, {}))
  return { html, blocks }
}
