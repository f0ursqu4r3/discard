import type { CommandDto } from '../bridge/types'

export interface SlashCommand {
  name: string
  args: string
}

// Keep aligned with discard_core::slash::parse.
// A command is `/name` at the start of the draft. The arguments are the rest, trimmed.
export function parseSlash(text: string): SlashCommand | null {
  const match = /^\/([a-z][a-z0-9_-]{0,31})(\s|$)/.exec(text)
  if (!match) return null
  return { name: match[1]!, args: text.slice(1 + match[1]!.length).trim() }
}

/** `//help` becomes `/help`. Other text, such as `// note`, is unchanged. */
export function unescapeSlash(text: string): string {
  return text.startsWith('//') && parseSlash(text.slice(1)) ? text.slice(1) : text
}

/** The typed name part when the whole draft is `/` plus name characters; else null. */
export function slashQuery(text: string): string | null {
  const match = /^\/([a-z0-9_-]*)$/.exec(text)
  return match ? match[1]! : null
}

/** Entries whose name starts with `query`, in registry order. */
export function filterCommands(entries: CommandDto[], query: string): CommandDto[] {
  return entries.filter((entry) => entry.name.startsWith(query))
}

/** `/name args` on one line for the result panel. */
export function commandLine(command: SlashCommand): string {
  return `/${command.name} ${command.args.replace(/\s+/g, ' ')}`.trimEnd()
}
