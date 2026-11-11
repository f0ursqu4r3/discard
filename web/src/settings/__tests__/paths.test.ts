import { expect, test } from 'vite-plus/test'
import { shortPath } from '../paths'

test('infers the macOS home directory from the path', () => {
  expect(shortPath('/Users/ada/git/discard')).toBe('~/git/discard')
})

test('infers the Linux home directory from the path', () => {
  expect(shortPath('/home/ada/.config/ghostty/config')).toBe('~/.config/ghostty/config')
})

test('shortens the home directory itself to a bare tilde', () => {
  expect(shortPath('/Users/ada')).toBe('~')
  expect(shortPath('/Users/ada/')).toBe('~/')
})

test('keeps paths outside a home directory', () => {
  expect(shortPath('/opt/tools/bin')).toBe('/opt/tools/bin')
  expect(shortPath('/p')).toBe('/p')
  expect(shortPath('')).toBe('')
})

test('does not match a directory that only starts with the same letters', () => {
  expect(shortPath('/Usersx/ada/work')).toBe('/Usersx/ada/work')
  expect(shortPath('/homes/ada/work')).toBe('/homes/ada/work')
})

test('uses an explicit home directory when given', () => {
  expect(shortPath('/var/ada/work', '/var/ada')).toBe('~/work')
  expect(shortPath('/var/ada/work', '/var/ada/')).toBe('~/work')
  expect(shortPath('/var/adam/work', '/var/ada')).toBe('/var/adam/work')
})

test('leaves an already shortened path unchanged', () => {
  expect(shortPath('~/git/discard')).toBe('~/git/discard')
})
