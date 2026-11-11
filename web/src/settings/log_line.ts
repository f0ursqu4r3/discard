// Parse a log line from `settings_logs`. The backend sends
// "<time> <Level> <Component> <message>", for example
// "2026-11-10T05:20:07.788482Z Info Other user plugins loaded".
// Time stays in UTC, as the backend wrote it.
export type LogLevel = 'Trace' | 'Debug' | 'Info' | 'Warn' | 'Error'

export type ParsedLogLine = {
  /** `YYYY-MM-DD` */
  date: string
  /** `HH:MM:SS.mmm` */
  time: string
  level: LogLevel
  component: string
  message: string
  raw: string
}

export type RawLogLine = {
  raw: string
  date?: undefined
  time?: undefined
  level?: undefined
  component?: undefined
  message?: undefined
}

const LINE =
  /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2}:\d{2})(?:\.(\d+))?\S* (Trace|Debug|Info|Warn|Error) (\S+)(?: ([\s\S]*))?$/

export function parseLogLine(line: string): ParsedLogLine | RawLogLine {
  const m = LINE.exec(line)
  if (!m) return { raw: line }
  const millis = (m[3] ?? '').slice(0, 3).padEnd(3, '0')
  return {
    date: m[1],
    time: `${m[2]}.${millis}`,
    level: m[4] as LogLevel,
    component: m[5],
    message: m[6] ?? '',
    raw: line,
  }
}
