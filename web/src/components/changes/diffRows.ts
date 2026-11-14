import type { parseDiff } from '../messages'

/** One parsed line of a unified diff, as `parseDiff` returns it. */
export type DiffLine = ReturnType<typeof parseDiff>['lines'][number]

/** One side of a split row. `text` has no `+`, `-`, or space prefix. */
export interface SplitCell {
  number: number | null
  text: string
  kind: 'context' | 'added' | 'removed'
}
/** A hunk header spans both sides. A note is a line such as `\ No newline at end of file`. */
export type SplitRow =
  | { type: 'hunk'; text: string }
  | { type: 'note'; text: string }
  | { type: 'line'; left: SplitCell | null; right: SplitCell | null }

export type InlineRow =
  | { type: 'hunk'; text: string }
  | { type: 'note'; text: string }
  | {
      type: 'line'
      kind: 'context' | 'added' | 'removed'
      oldNumber: number | null
      newNumber: number | null
      text: string
    }

const body = (text: string) => text.slice(1)

/** The lines of the hunks. Header lines before the first hunk (`rename from`, `new file mode`) are dropped. */
function hunkLines(lines: DiffLine[]): DiffLine[] {
  const first = lines.findIndex((line) => line.kind === 'hunk')
  return first < 0 ? [] : lines.slice(first)
}

/** A line that has no number inside a hunk is a note, such as `\ No newline at end of file`. */
const isNote = (line: DiffLine) =>
  line.kind === 'context' && line.number === null && line.text !== ''

/**
 * Turn the lines of a parsed unified diff into rows of two sides.
 * A context line fills both sides. A run of removed lines followed by a run of added lines pairs row by row,
 * and the longer run leaves the opposite cell empty. A hunk header is one row for both sides.
 */
export function splitRows(lines: DiffLine[]): SplitRow[] {
  const rows: SplitRow[] = []
  let removed: DiffLine[] = []
  let added: DiffLine[] = []
  const flush = () => {
    for (let i = 0; i < Math.max(removed.length, added.length); i++) {
      const old = removed[i]
      const next = added[i]
      rows.push({
        type: 'line',
        left: old ? { number: old.oldNumber, text: body(old.text), kind: 'removed' } : null,
        right: next ? { number: next.newNumber, text: body(next.text), kind: 'added' } : null,
      })
    }
    removed = []
    added = []
  }
  for (const line of hunkLines(lines)) {
    if (line.kind === 'removed') {
      // A removed line after an added line starts a new run.
      if (added.length) flush()
      removed.push(line)
    } else if (line.kind === 'added') {
      added.push(line)
    } else if (line.kind === 'hunk') {
      flush()
      rows.push({ type: 'hunk', text: line.text })
    } else if (isNote(line)) {
      flush()
      rows.push({ type: 'note', text: line.text })
    } else if (line.number !== null) {
      flush()
      rows.push({
        type: 'line',
        left: { number: line.oldNumber, text: body(line.text), kind: 'context' },
        right: { number: line.newNumber, text: body(line.text), kind: 'context' },
      })
    }
  }
  flush()
  return rows
}

/** Turn the lines of a parsed unified diff into one-column rows with an old and a new line number. */
export function inlineRows(lines: DiffLine[]): InlineRow[] {
  const rows: InlineRow[] = []
  for (const line of hunkLines(lines)) {
    if (line.kind === 'hunk') rows.push({ type: 'hunk', text: line.text })
    else if (isNote(line)) rows.push({ type: 'note', text: line.text })
    else if (line.kind !== 'context' || line.number !== null)
      rows.push({
        type: 'line',
        kind: line.kind as 'context' | 'added' | 'removed',
        oldNumber: line.oldNumber,
        newNumber: line.newNumber,
        text: body(line.text),
      })
  }
  return rows
}
