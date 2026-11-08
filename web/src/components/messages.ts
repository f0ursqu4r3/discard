import type { MessageDto } from '../bridge/types'

/** Display times and date boundaries in the viewer's local time zone. */
export function messageTime(message: MessageDto): Date | null {
  if (message.createdAt == null) return null
  const date = new Date(message.createdAt)
  return Number.isNaN(date.getTime()) ? null : date
}

export function messageDateLabel(date: Date): string {
  return date.toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' })
}

export function messagePresentation(message: MessageDto, previous?: MessageDto) {
  const date = messageTime(message)
  const previousDate = previous && messageTime(previous)
  const startsDate =
    !!date && (!previousDate || date.toDateString() !== previousDate.toDateString())
  const continuesGroup =
    !!previous &&
    message.continuesGroup &&
    message.role === previous.role &&
    message.author?.agentId === previous.author?.agentId &&
    !startsDate &&
    !!date &&
    !!previousDate &&
    Math.floor(date.getTime() / 60_000) === Math.floor(previousDate.getTime() / 60_000)
  return { date, startsDate, continuesGroup }
}

/** Whether a streaming reply has a visible row. */
export function hasLiveRow(_running: boolean, blocks: number): boolean {
  return blocks > 0
}
/** Show the agents that are thinking. */
export function thinkingLine(running: boolean, agents: string[]): string | null {
  if (!running || !agents.length) return null
  if (agents.length === 1) return `${agents[0]} is thinking...`
  if (agents.length === 2) return `${agents[0]} and ${agents[1]} are thinking...`
  return `${agents[0]}, ${agents[1]} and ${agents.length - 2} more are thinking...`
}
/** Count history rows and the optional streaming row. */
export function rowCount(history: number, live: boolean): number {
  return history + Number(live)
}
/** Classify a line of unified diff output. */
export function classifyLine(line: string): 'added' | 'removed' | 'hunk' | 'context' {
  if (line.startsWith('+++') || line.startsWith('---')) return 'context'
  if (line.startsWith('+')) return 'added'
  if (line.startsWith('-')) return 'removed'
  if (line.startsWith('@@')) return 'hunk'
  return 'context'
}
/** Detect a unified diff without treating ordinary lists as diffs. */
export function looksLikeUnifiedDiff(text: string): boolean {
  let oldHeader = false
  for (const line of text.split('\n')) {
    if (
      (line.startsWith('@@ ') && line.slice(3).includes(' @@')) ||
      (oldHeader && line.startsWith('+++ '))
    )
      return true
    oldHeader = line.startsWith('--- ')
  }
  return false
}
/** Summarize the most useful field of a tool call. */
export function summarizeInput(input: unknown): string {
  let text: string | undefined
  if (input !== null && typeof input === 'object' && !Array.isArray(input)) {
    const fields = input as Record<string, unknown>
    for (const key of ['command', 'file_path', 'path', 'url', 'pattern']) {
      if (typeof fields[key] === 'string') {
        text = fields[key]
        break
      }
    }
  }
  text ??= JSON.stringify(input) ?? 'null'
  const chars = Array.from(text)
  return chars.length > 120 ? `${chars.slice(0, 120).join('')}…` : text
}

/** Read unified diff content and keep old and new line counters separate. */
export function parseDiff(text: string): {
  path: string | null
  added: number
  removed: number
  lines: { text: string; kind: 'added' | 'removed' | 'hunk' | 'context'; number: number | null }[]
} {
  let path: string | null = null
  let oldPath: string | null = null
  let oldNumber: number | null = null,
    newNumber: number | null = null
  let oldRemaining = 0,
    newRemaining = 0
  let inHunk = false,
    files = 0
  let added = 0,
    removed = 0
  const lines: ReturnType<typeof parseDiff>['lines'] = []
  const headers = new Set<(typeof lines)[number]>()
  const input = text.split('\n')
  if (input.at(-1) === '') input.pop()
  for (const textLine of input) {
    if (textLine.startsWith('diff --git ')) {
      inHunk = false
      oldNumber = newNumber = null
      continue
    }
    if (!inHunk && textLine.startsWith('index ')) continue
    if (!inHunk && (textLine.startsWith('--- ') || textLine.startsWith('+++ '))) {
      const header = { text: textLine, kind: 'context' as const, number: null }
      headers.add(header)
      lines.push(header)
      if (textLine.startsWith('--- ')) {
        oldPath = textLine.slice(4).replace(/^a\//, '')
        oldNumber = newNumber = null
      } else {
        const nextPath = textLine.slice(4)
        path =
          nextPath === '/dev/null' ? oldPath : nextPath.startsWith('b/') ? nextPath.slice(2) : null
        files++
      }
      continue
    }
    const hunk = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(textLine)
    if (hunk) {
      oldNumber = Number(hunk[1])
      newNumber = Number(hunk[3])
      oldRemaining = Number(hunk[2] ?? 1)
      newRemaining = Number(hunk[4] ?? 1)
      inHunk = true
    }
    const kind = textLine.startsWith('@@')
      ? 'hunk'
      : textLine.startsWith('+')
        ? 'added'
        : textLine.startsWith('-')
          ? 'removed'
          : 'context'
    const number =
      kind === 'hunk' || (kind === 'context' && !textLine.startsWith(' '))
        ? null
        : kind === 'removed'
          ? oldNumber
          : newNumber
    lines.push({ text: textLine, kind, number })
    if (kind === 'added') {
      added++
      if (newNumber !== null) newNumber++
      newRemaining--
    } else if (kind === 'removed') {
      removed++
      if (oldNumber !== null) oldNumber++
      oldRemaining--
    } else if (kind === 'context' && textLine.startsWith(' ')) {
      if (oldNumber !== null) oldNumber++
      if (newNumber !== null) newNumber++
      oldRemaining--
      newRemaining--
    }
    if (oldRemaining <= 0 && newRemaining <= 0) inHunk = false
  }
  // A multi-file patch keeps its headers so each hunk still identifies its file.
  return {
    path: files > 1 ? null : path,
    added,
    removed,
    lines: files > 1 ? lines : lines.filter((line) => !headers.has(line)),
  }
}
