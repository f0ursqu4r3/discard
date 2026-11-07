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
