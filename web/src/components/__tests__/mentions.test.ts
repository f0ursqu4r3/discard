import { expect, test } from 'vite-plus/test'
import { mentionQuery, mentionToken } from '../mentions'

test('finds a partial name at the caret and replaces the complete token', () => {
  expect(mentionQuery('Ask @Jordan please', 7)).toEqual({ start: 4, end: 11, query: 'Jo' })
  expect(mentionQuery('@"Jordan L', 10)?.query).toBe('Jordan L')
  expect(mentionToken('Jordan Lee')).toBe('@"Jordan Lee"')
  expect(mentionToken('Jordan')).toBe('@Jordan')
})
test.each(['mail@Jo', '> @Jo', '    @Jo', '\t@Jo', '`@Jo', '```sh\n@Jo', '~~~\n@Jo', '\\@Jo'])(
  'ignores literal text: %s',
  (text) => {
    expect(mentionQuery(text, text.length)).toBeNull()
  },
)
test('allows a mention after closed code', () => {
  const text = '```\n@Ignored\n```\n@Jo'
  expect(mentionQuery(text, text.length)?.query).toBe('Jo')
})
