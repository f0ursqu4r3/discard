import type { MessageDto } from '../bridge/types'

export interface MessageSource {
  channelId: string
  messageId: string
}

const sourcePrefix = '#discard-message/'
const encodeId = (id: string) =>
  encodeURIComponent(id).replace(/[!'()*]/g, (char) => `%${char.charCodeAt(0).toString(16)}`)

export function messageSourceLink(source: MessageSource): string {
  return `${sourcePrefix}${encodeId(source.channelId)}/${encodeId(source.messageId)}`
}

export function parseMessageSource(href: string): MessageSource | null {
  if (!href.startsWith(sourcePrefix)) return null
  const parts = href.slice(sourcePrefix.length).split('/')
  if (parts.length !== 2 || !parts.every(Boolean)) return null
  try {
    return { channelId: decodeURIComponent(parts[0]!), messageId: decodeURIComponent(parts[1]!) }
  } catch {
    return null
  }
}

/** Quote readable message blocks. Attachments and tool actions are not duplicated. */
export function quoteMessage(
  message: MessageDto,
  channelId: string,
  fallbackAgent: string,
): string {
  const content = message.blocks
    .flatMap((block) => {
      if (!('text' in block) || !block.text) return []
      if (block.type === 'markdown') return [block.text]
      if (block.type !== 'code' && block.type !== 'diff') return []
      const longest = Math.max(0, ...(block.text.match(/`+/g) ?? []).map((run) => run.length))
      const fence = '`'.repeat(Math.max(3, longest + 1))
      const language =
        block.type === 'code' ? (block.language ?? '').replace(/[^\w+-]/g, '') : 'diff'
      return [`${fence}${language}\n${block.text}\n${fence}`]
    })
    .join('\n\n')
  if (!content.trim()) return ''
  const author =
    message.role === 'user'
      ? 'You'
      : message.role === 'agent'
        ? (message.author?.name ?? fallbackAgent)
        : 'System'
  const label = author.replace(/[\r\n]+/g, ' ').replace(/[\\`*_[\]<>]/g, '\\$&')
  const source = messageSourceLink({ channelId, messageId: message.id })
  return `> [Message from ${label}](${source})\n>\n${content
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => `> ${line}`)
    .join('\n')}\n\n`
}

export function appendQuote(draft: string, quote: string): string {
  if (!quote) return draft
  const separator = !draft || draft.endsWith('\n\n') ? '' : draft.endsWith('\n') ? '\n' : '\n\n'
  return draft + separator + quote
}
