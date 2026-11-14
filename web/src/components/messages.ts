import type { BlockDto, MessageDto, PromptDto } from '../bridge/types'

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
  lines: {
    text: string
    kind: 'added' | 'removed' | 'hunk' | 'context'
    /** The new number of an added or context line, the old number of a removed line. */
    number: number | null
    /** The line number in the old file. Null for an added line and for a line outside a hunk body. */
    oldNumber: number | null
    /** The line number in the new file. Null for a removed line and for a line outside a hunk body. */
    newNumber: number | null
  }[]
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
      const header = {
        text: textLine,
        kind: 'context' as const,
        number: null,
        oldNumber: null,
        newNumber: null,
      }
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
    const body = kind === 'added' || kind === 'removed' || (kind === 'context' && number !== null)
    lines.push({
      text: textLine,
      kind,
      number,
      oldNumber: body && kind !== 'added' ? oldNumber : null,
      newNumber: body && kind !== 'removed' ? newNumber : null,
    })
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

export type ApprovalBlock = Extract<BlockDto, { type: 'approval' }>
export type ToolCallItem = {
  block: Extract<BlockDto, { type: 'tool_call' }>
  index: number
  approval?: ApprovalBlock
}
export type BlockSegment =
  | { kind: 'block'; block: BlockDto; index: number; approval?: ApprovalBlock }
  | { kind: 'tools'; items: ToolCallItem[] }

/** Group consecutive calls without changing their stored block indexes. An approval joins the call with the same ID. */
export function groupBlocks(blocks: BlockDto[]): BlockSegment[] {
  // The approval block follows its call in saved replies. Match both orders to be safe.
  const callAt = new Map<string, number>()
  blocks.forEach((block, index) => {
    if (block.type === 'tool_call' && !callAt.has(block.call_id)) callAt.set(block.call_id, index)
  })
  const attached = new Map<number, ApprovalBlock>()
  const joined = new Set<number>()
  blocks.forEach((block, index) => {
    if (block.type !== 'approval') return
    let call = block.call_id ? callAt.get(block.call_id) : undefined
    // An old record has no call ID. Its approval block directly follows the call of the same tool.
    const before = blocks[index - 1]
    if (!block.call_id && before?.type === 'tool_call' && before.name === block.tool)
      call = index - 1
    if (call === undefined || attached.has(call)) return
    attached.set(call, block)
    joined.add(index)
  })
  const segments: BlockSegment[] = []
  let calls: ToolCallItem[] = []
  function flush() {
    if (calls.length === 1) segments.push({ kind: 'block', ...calls[0]! })
    else if (calls.length > 1) segments.push({ kind: 'tools', items: calls })
    calls = []
  }
  blocks.forEach((block, index) => {
    if (joined.has(index)) return
    if (block.type === 'tool_call') {
      const approval = attached.get(index)
      calls.push(approval ? { block, index, approval } : { block, index })
    } else {
      flush()
      segments.push({ kind: 'block', block, index })
    }
  })
  flush()
  return segments
}

export type ApprovalStatus =
  | 'pending'
  | 'approved'
  | 'auto_approved'
  | 'denied'
  | 'stale'
  | 'answered'

/** The live prompt wins. Otherwise the saved state. A record with neither is answered, outcome unknown. */
export function approvalStatus(
  block: ApprovalBlock | undefined,
  prompts: PromptDto[] | undefined,
): ApprovalStatus | undefined {
  if (!block) return undefined
  const view = prompts?.find((prompt) => prompt.approvalId === block.approval_id)?.view
  const state = view ?? block.state
  if (state === 'pending' || state === 'denied' || state === 'stale') return state
  if (state === 'approved') return block.decided_by === 'policy' ? 'auto_approved' : 'approved'
  return 'answered'
}

/** The text mark for an approval status. Old records have no mark. */
export function approvalMark(
  status: ApprovalStatus | undefined,
): { text: string; label: string; pending: boolean } | null {
  switch (status) {
    case 'pending':
      return { text: 'Needs approval', label: 'Approval needed', pending: true }
    case 'approved':
      return { text: 'Approved', label: 'Approved by you', pending: false }
    case 'auto_approved':
      return { text: 'Auto-approved', label: 'Approved by policy', pending: false }
    case 'denied':
      return { text: 'Denied', label: 'Denied', pending: false }
    case 'stale':
      return { text: 'Expired', label: 'Approval expired', pending: false }
    default:
      return null
  }
}

export function toolCallLabel(name: string): string {
  const action = name.split('__').filter(Boolean).at(-1) ?? name
  const words = action
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[_-]+/g, ' ')
    .trim()
  return words ? words.toLowerCase() : 'tool call'
}

export function toolCallSummary(input: unknown): string {
  if (input == null) return ''
  if (typeof input === 'object' && !Array.isArray(input)) {
    const fields = input as Record<string, unknown>
    if (!Object.keys(fields).length) return ''
    if (typeof fields.query === 'string') return summarizeInput({ pattern: fields.query })
  }
  return summarizeInput(input)
}

export type ChangeEntry = Extract<BlockDto, { type: 'changes' | 'diff' | 'code' }>

/** What the Changes tab lists: the file changes each response made, and the diffs agents posted. */
export function changeEntries(history: MessageDto[]): ChangeEntry[] {
  return history.flatMap((message) =>
    message.blocks.filter(
      (block): block is ChangeEntry =>
        (block.type === 'changes' && block.files.length + block.commits.length > 0) ||
        block.type === 'diff' ||
        (block.type === 'code' && ['diff', 'patch'].includes(block.language || '')),
    ),
  )
}
