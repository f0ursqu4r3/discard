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

/** A reply split into its parts. `body` is the text the user typed. */
export interface ReplyParts {
  source: MessageSource
  author: string
  excerpt: string
  body: string
}

/** A reply that waits for its message. `prefix` is the text sent before the typed text. */
export interface PendingReply {
  prefix: string
  source: MessageSource
  author: string
  excerpt: string
}

const excerptLimit = 160
const labelEscape = /[\\`*_[\]<>]/g
const header =
  /^> \[Message from ((?:\\[\\`*_[\]<>]|[^\]\\\n])*)\]\((#discard-message\/[^)\s]*)\)\n>(?:\n|$)/

/**
 * Read a reply prefix at the start of `text`. Return null when the text does not start with one.
 * The prefix is a quote block and a blank line. The rest is the typed text.
 */
export function parseReply(text: string): ReplyParts | null {
  const matched = header.exec(text)
  if (!matched) return null
  const source = parseMessageSource(matched[2]!)
  if (!source) return null
  const lines = text.slice(matched[0].length).replace(/\r\n?/g, '\n').split('\n')
  const quoted: string[] = []
  let next = 0
  while (next < lines.length && (lines[next]!.startsWith('> ') || lines[next] === '>')) {
    quoted.push(lines[next]!.slice(2))
    next++
  }
  // A quote block ends at a blank line or at the end of the text. A line after it belongs to no reply.
  if (next < lines.length && lines[next] !== '') return null
  const excerpt = quoted
    .filter((line) => !/^\s*(?:`{3,}|~{3,})/.test(line))
    .join(' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, excerptLimit)
  return {
    source,
    author: matched[1]!.replace(/\\([\\`*_[\]<>])/g, '$1'),
    excerpt,
    body: lines.slice(next + 1).join('\n'),
  }
}

/** The reply that a saved user message holds, with the index of the block that holds it. */
export function messageReply(message: MessageDto): { index: number; reply: ReplyParts } | null {
  if (message.role !== 'user' || message.author) return null
  const index = message.blocks.findIndex((block) => block.type === 'markdown')
  const block = message.blocks[index]
  if (!block || block.type !== 'markdown') return null
  const reply = parseReply(block.text)
  return reply ? { index, reply } : null
}

/** The reply prefix of readable message blocks. Attachments and tool actions are not duplicated. */
export function replyPrefix(message: MessageDto, channelId: string, fallbackAgent: string): string {
  // The prefix of a reply to a reply quotes only the typed text of that reply.
  const own = messageReply(message)
  const content = message.blocks
    .flatMap((block, index) => {
      if (!('text' in block) || !block.text) return []
      if (block.type === 'markdown') {
        const text = own?.index === index ? own.reply.body : block.text
        return text.trim() ? [text] : []
      }
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
  const label = author.replace(/[\r\n]+/g, ' ').replace(labelEscape, '\\$&')
  const source = messageSourceLink({ channelId, messageId: message.id })
  return `> [Message from ${label}](${source})\n>\n${content
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => `> ${line}`)
    .join('\n')}\n\n`
}

/** Build the pending reply to a message. Return null when the message has no readable text. */
export function createReply(
  message: MessageDto,
  channelId: string,
  fallbackAgent: string,
): PendingReply | null {
  const prefix = replyPrefix(message, channelId, fallbackAgent)
  const parts = prefix ? parseReply(prefix) : null
  return parts
    ? { prefix, source: parts.source, author: parts.author, excerpt: parts.excerpt }
    : null
}

/** Join the prefix and the typed text. Without typed text, only the quote block remains. */
export function joinReply(prefix: string, typed: string): string {
  return typed ? prefix + typed : prefix.trimEnd()
}
