import { expect, test } from 'vite-plus/test'
import { parseLogLine } from '../log_line'

test('splits a backend line into its parts', () => {
  const raw = '2026-11-10T05:20:07.788482Z Info Other user plugins loaded'
  expect(parseLogLine(raw)).toEqual({
    date: '2026-11-10',
    time: '05:20:07.788',
    level: 'Info',
    component: 'Other',
    message: 'user plugins loaded',
    raw,
  })
})

test('reads every level and the MCP component', () => {
  for (const level of ['Trace', 'Debug', 'Info', 'Warn', 'Error']) {
    const parsed = parseLogLine(`2026-11-10T00:00:00.000Z ${level} MCP started`)
    expect(parsed.level).toBe(level)
    expect(parsed.component).toBe('MCP')
  }
})

test('keeps extra spaces and newlines in the message', () => {
  const parsed = parseLogLine('2026-11-10T05:20:07.1Z Warn Provider  two  spaces\nsecond line ')
  expect(parsed.message).toBe(' two  spaces\nsecond line ')
  expect(parsed.time).toBe('05:20:07.100')
})

test('accepts a time without a fraction and an empty message', () => {
  const parsed = parseLogLine('2026-11-10T05:20:07Z Error Storage')
  expect(parsed.time).toBe('05:20:07.000')
  expect(parsed.message).toBe('')
})

test('keeps a truncated message that ends with an ellipsis', () => {
  expect(parseLogLine('2026-11-10T05:20:07.5Z Info Other long…').message).toBe('long…')
})

test('falls back to the raw line when the format does not match', () => {
  expect(parseLogLine('plain text')).toEqual({ raw: 'plain text' })
  expect(parseLogLine(' Info Other bad')).toEqual({ raw: ' Info Other bad' })
  expect(parseLogLine('2026-11-10T05:20:07.5Z Fatal Other nope')).toEqual({
    raw: '2026-11-10T05:20:07.5Z Fatal Other nope',
  })
  expect(parseLogLine('')).toEqual({ raw: '' })
})
