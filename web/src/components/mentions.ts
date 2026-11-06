export interface MentionQuery {
  start: number
  end: number
  query: string
}

// Keep these text exclusions aligned with discard_core::mentions::names.
export function mentionQuery(text: string, caret: number): MentionQuery | null {
  const before = text.slice(0, caret)
  const lines = before.split('\n')
  let fence: { marker: string; length: number } | null = null
  for (const line of lines.slice(0, -1)) {
    const match = /^ {0,3}(`{3,}|~{3,})/.exec(line)
    if (!match) continue
    if (!fence) fence = { marker: match[1]![0]!, length: match[1]!.length }
    else if (match[1]![0] === fence.marker && match[1]!.length >= fence.length) fence = null
  }
  const line = lines.at(-1) ?? ''
  if (fence || /^\s*>|^(?: {4}|\t)|^ {0,3}(?:`{3,}|~{3,})/.test(line)) return null
  const match = /(?:^|[\s(])@(?:(?:"([^"\n]*))|([\p{L}\p{N}_-]*))$/u.exec(line)
  if (!match) return null
  const start = caret - match[0].length + match[0].indexOf('@')
  const prefix = text.slice(caret - line.length, start)
  // Escaped markers and open inline-code spans are literal text.
  let ticks = 0
  for (let i = 0; i < prefix.length; i++) {
    if (prefix[i] === '\\') {
      i++
      continue
    }
    if (prefix[i] !== '`') continue
    let end = i
    while (prefix[end] === '`') end++
    const count = end - i
    if (!ticks) ticks = count
    else if (ticks === count) ticks = 0
    i = end - 1
  }
  if (ticks) return null
  const quoted = match[1] !== undefined
  const tail = text.slice(caret)
  const remainder = quoted ? /^[^"\n]*(?:"|$)/.exec(tail)?.[0] : /^[\p{L}\p{N}_-]*/u.exec(tail)?.[0]
  return { start, end: caret + (remainder?.length ?? 0), query: match[1] ?? match[2] ?? '' }
}

export function mentionToken(name: string): string {
  return /^[\p{L}\p{N}_-]+$/u.test(name) ? `@${name}` : `@"${name}"`
}
