import { describe, expect, it } from 'vite-plus/test'
import { commandLine, filterCommands, parseSlash, slashQuery, unescapeSlash } from '../slash'

describe('parseSlash', () => {
  it('reads the name and the trimmed arguments', () => {
    expect(parseSlash('/help')).toEqual({ name: 'help', args: '' })
    expect(parseSlash('/echo  a b \n c ')).toEqual({ name: 'echo', args: 'a b \n c' })
    expect(parseSlash('/stop\nnow')).toEqual({ name: 'stop', args: 'now' })
  })
  it('accepts the longest name', () => {
    expect(parseSlash('/' + 'a'.repeat(32))).toEqual({ name: 'a'.repeat(32), args: '' })
  })
  it.each([
    '',
    '/',
    '//help',
    ' /help',
    '/Help',
    '/usr/bin/env',
    '/1x',
    'hello /help',
    '/' + 'a'.repeat(33),
  ])('does not treat %j as a command', (text) => expect(parseSlash(text)).toBeNull())
})
describe('unescapeSlash', () => {
  it('removes the first slash when the rest has the command form', () => {
    expect(unescapeSlash('//help me')).toBe('/help me')
    expect(unescapeSlash('//help')).toBe('/help')
  })
  it.each(['// this fails', '///x', '//usr/bin', '/usr/bin', 'a // b'])(
    'leaves %j unchanged',
    (text) => expect(unescapeSlash(text)).toBe(text),
  )
})
describe('slashQuery', () => {
  it('matches only a draft that is a partial command name', () => {
    expect(slashQuery('/')).toBe('')
    expect(slashQuery('/he')).toBe('he')
    expect(slashQuery('/he ')).toBeNull()
    expect(slashQuery('//')).toBeNull()
    expect(slashQuery('/He')).toBeNull()
    expect(slashQuery('x /he')).toBeNull()
  })
  it('opens for every name character, including a digit or dash first', () => {
    expect(slashQuery('/1')).toBe('1')
    expect(slashQuery('/-')).toBe('-')
  })
  it('returns a name longer than the limit as typed', () => {
    const long = 'a'.repeat(33)
    expect(slashQuery('/' + long)).toBe(long)
  })
})
describe('filterCommands', () => {
  const entry = (name: string) => ({
    name,
    description: '',
    args: null,
    source: 'builtin',
    planned: false,
  })
  it('keeps registry order and filters by prefix', () => {
    const all = [entry('help'), entry('stop'), entry('settings')]
    expect(filterCommands(all, 's').map((e) => e.name)).toEqual(['stop', 'settings'])
    expect(filterCommands(all, '')).toHaveLength(3)
    expect(filterCommands(all, 'x')).toEqual([])
  })
})
describe('commandLine', () => {
  it('joins the name and the arguments on one line', () => {
    expect(commandLine({ name: 'echo', args: 'a\nb' })).toBe('/echo a b')
    expect(commandLine({ name: 'help', args: '' })).toBe('/help')
  })
})
