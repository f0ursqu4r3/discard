import { expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import {
  createReply,
  joinReply,
  messageReply,
  messageSourceLink,
  parseMessageSource,
  parseReply,
  replyPrefix,
} from '../reply'
import type { MessageDto } from '../../bridge/types'
import type { PendingReply } from '../reply'
import SafeMarkdown from '../SafeMarkdown.vue'
import MessageList from '../MessageList.vue'
import Composer from '../Composer.vue'
import { defineComponent, h, ref } from 'vue'
import { ContextMenuItem } from '../ui/context-menu'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (name: string) =>
    name === 'context_menu'
      ? []
      : {
          items: [],
          scopes: [],
          proposals: [],
          responses: [],
          paused: false,
        },
  ),
}))

const message: MessageDto = {
  id: 'source',
  role: 'agent',
  continuesGroup: false,
  author: { agentId: 'a', name: 'Quinn [test]', provider: 'test', avatar: null },
  blocks: [{ type: 'markdown', text: 'First line\n\nSecond line' }],
}

test('the reply prefix preserves paragraphs and identifies the saved author and source', () => {
  const prefix = replyPrefix(message, 'channel', 'Other agent')
  expect(prefix).toContain('> First line\n> \n> Second line\n\n')
  const view = mount(SafeMarkdown, { props: { text: prefix } })
  expect(view.get('blockquote a').text()).toBe('Message from Quinn [test]')
  expect(view.get('a').attributes('href')).toBe('#discard-message/channel/source')
  expect(view.text()).not.toContain('Other agent')
})

test('the reply prefix preserves code and diff blocks, including embedded fences', () => {
  const prefix = replyPrefix(
    {
      ...message,
      blocks: [
        { type: 'code', language: 'md', text: '```js\nconst x = 1\n```' },
        { type: 'diff', text: '-old\n+new' },
      ],
    },
    'channel',
    'Agent',
  )
  expect(prefix).toContain('> ````md\n> ```js')
  expect(prefix).toContain('> ```diff\n> -old\n> +new\n> ```')
})

test('source references round-trip encoded IDs and reject malformed links', () => {
  const source = { channelId: 'channel/a)', messageId: 'message/%)' }
  expect(parseMessageSource(messageSourceLink(source))).toEqual(source)
  for (const href of [
    'https://example.com',
    '#discard-message/a',
    '#discard-message/a/b/c',
    '#discard-message/a/%FF',
  ]) {
    expect(parseMessageSource(href)).toBeNull()
  }
})

test('the prefix of a message without readable text is empty', () => {
  expect(replyPrefix({ ...message, blocks: [] }, 'channel', 'Agent')).toBe('')
  expect(createReply({ ...message, blocks: [] }, 'channel', 'Agent')).toBeNull()
})

test('a reply prefix keeps the exact quote wire format', () => {
  expect(replyPrefix(message, 'channel', 'Agent')).toBe(
    '> [Message from Quinn \\[test\\]](#discard-message/channel/source)\n>\n> First line\n> \n> Second line\n\n',
  )
})

test('parseReply reads the source, the author, the excerpt, and the typed text', () => {
  const prefix = replyPrefix(message, 'channel', 'Agent')
  expect(parseReply(prefix + 'Thanks\nAnd more')).toEqual({
    source: { channelId: 'channel', messageId: 'source' },
    author: 'Quinn [test]',
    excerpt: 'First line Second line',
    body: 'Thanks\nAnd more',
  })
  expect(parseReply(prefix.trimEnd())).toMatchObject({
    body: '',
    excerpt: 'First line Second line',
  })
})

test('parseReply unescapes every escaped author character', () => {
  const author = 'A\\`*_[]<>b'
  const prefix = replyPrefix(
    { ...message, author: { agentId: 'a', name: author, provider: 'test', avatar: null } },
    'channel',
    'Agent',
  )
  expect(parseReply(prefix + 'Hi')?.author).toBe(author)
})

test('parseReply skips code fences in the excerpt', () => {
  const prefix = replyPrefix(
    { ...message, blocks: [{ type: 'code', language: 'ts', text: 'const x = 1' }] },
    'channel',
    'Agent',
  )
  expect(parseReply(prefix)?.excerpt).toBe('const x = 1')
})

test.each([
  'Plain text',
  'Before\n\n> [Message from Quinn](#discard-message/channel/source)\n>\n> Text\n\nAfter',
  '> A quote\n\nThanks',
  '> [Message from Quinn](https://example.com)\n>\n> Text\n\nThanks',
  '> [Message from Quinn](#discard-message/channel/source)\n>\n> Text\nlazy line',
])('parseReply rejects text that has no reply prefix: %j', (text) => {
  expect(parseReply(text)).toBeNull()
})

test('a reply to a reply quotes only the typed text', () => {
  const stored: MessageDto = {
    id: 'mine',
    role: 'user',
    continuesGroup: false,
    blocks: [
      {
        type: 'markdown',
        text: replyPrefix(message, 'channel', 'Agent') + 'My answer',
      },
    ],
  }
  expect(messageReply(stored)).toMatchObject({ index: 0, reply: { body: 'My answer' } })
  const prefix = replyPrefix(stored, 'channel', 'Agent')
  expect(prefix).toBe('> [Message from You](#discard-message/channel/mine)\n>\n> My answer\n\n')
  // A reply that holds only the prefix leaves nothing to quote.
  expect(
    replyPrefix(
      { ...stored, blocks: [{ type: 'markdown', text: replyPrefix(message, 'c', 'A').trimEnd() }] },
      'channel',
      'Agent',
    ),
  ).toBe('')
})

test('only a user message with no author holds a reply', () => {
  const text = replyPrefix(message, 'channel', 'Agent') + 'Hi'
  const user: MessageDto = {
    ...message,
    role: 'user',
    author: null,
    blocks: [{ type: 'markdown', text }],
  }
  expect(messageReply(user)).not.toBeNull()
  expect(messageReply({ ...user, role: 'agent' })).toBeNull()
  expect(
    messageReply({ ...user, author: { agentId: 'a', name: 'Quinn', provider: 'x', avatar: null } }),
  ).toBeNull()
  expect(
    messageReply({ ...user, blocks: [{ type: 'markdown', text: 'Hi\n\n' + text }] }),
  ).toBeNull()
})

test('joining a reply puts the typed text after the prefix', () => {
  const pending = createReply(message, 'channel', 'Agent')!
  expect(pending).toMatchObject({
    author: 'Quinn [test]',
    excerpt: 'First line Second line',
    source: { channelId: 'channel', messageId: 'source' },
  })
  expect(joinReply(pending.prefix, 'Thanks')).toBe(pending.prefix + 'Thanks')
  expect(joinReply(pending.prefix, '')).toBe(pending.prefix.trimEnd())
})

test('the reply icon emits the selected message through the transcript', async () => {
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [message],
      live: [],
      running: false,
      newAnchorId: null,
    },
  })
  await list.get('button[aria-label="Reply to message"]').trigger('click')
  expect(list.emitted('reply')?.[0]?.[0]).toMatchObject({ id: 'source', blocks: message.blocks })
  await list.setProps({ replyDisabled: true })
  expect(list.get('button[aria-label="Reply to message"]').attributes('disabled')).toBeDefined()
  await list.get('button[aria-label="Reply to message"]').trigger('click')
  expect(list.emitted('reply')).toHaveLength(1)
  expect(list.get('button[aria-label="Start thread"]').attributes('disabled')).toBeUndefined()
  await list.setProps({ threadDisabled: true })
  expect(list.get('button[aria-label="Start thread"]').attributes('disabled')).toBeDefined()
  list.unmount()
})

test('replying from the context menu closes the menu before focusing the composer', async () => {
  const send = vi.fn()
  const focused = vi.fn()
  const Host = defineComponent({
    setup() {
      const composer = ref<InstanceType<typeof Composer> | null>(null)
      const reply = ref<PendingReply | null>(null)
      return () =>
        h('div', [
          h(MessageList, {
            conversationId: 'channel',
            messages: [message],
            live: [],
            running: false,
            newAnchorId: null,
            onReply: (source: MessageDto) => {
              reply.value = createReply(source, 'channel', 'Quinn')
              void composer.value?.focusEnd().then(focused)
            },
          }),
          h(Composer, {
            ref: composer,
            channelId: 'channel',
            archived: false,
            busy: false,
            running: false,
            channel: 'general',
            draft: 'Draft',
            reply: reply.value,
            onSend: send,
            onCancelReply: () => {
              reply.value = null
            },
          }),
        ])
    },
  })
  const view = mount(Host, { attachTo: document.body })
  try {
    await view
      .get('[data-ui-id="message-source"]')
      .trigger('contextmenu', { button: 2, clientX: 20, clientY: 20 })
    const item = view.findAllComponents(ContextMenuItem).find((item) => item.text() === 'Reply')!
    await item.trigger('click')
    await vi.waitFor(() => {
      const input = view.get('textarea').element
      expect(input.value).toBe('Draft')
      expect(document.activeElement).toBe(input)
      expect(view.get('[data-ui-id="reply-chip"]').text()).toContain('Replying to Quinn [test]')
    })
    expect(send).not.toHaveBeenCalled()
    // Escape in the input cancels the reply.
    await view.get('textarea').trigger('keydown', { key: 'Escape' })
    expect(view.find('[data-ui-id="reply-chip"]').exists()).toBe(false)
  } finally {
    view.unmount()
  }
})

test('the reply chip links to the source and Cancel reply clears it', async () => {
  const pending = createReply(message, 'channel', 'Agent')!
  const composer = mount(Composer, {
    props: {
      channelId: 'c1',
      archived: false,
      busy: false,
      running: false,
      channel: 'general',
      reply: pending,
    },
  })
  const chip = composer.get('[data-ui-id="reply-chip"]')
  expect(chip.text()).toContain('Replying to Quinn [test]')
  expect(chip.text()).toContain('First line Second line')
  expect(chip.get('a').attributes('href')).toBe('#discard-message/channel/source')
  await chip.get('button[aria-label="Cancel reply"]').trigger('click')
  expect(composer.emitted('cancelReply')).toHaveLength(1)
  // The mention list takes Escape first. Escape closes the list and keeps the reply.
  await composer.get('textarea').setValue('@')
  await composer.get('textarea').trigger('keydown', { key: 'Escape' })
  expect(composer.emitted('cancelReply')).toHaveLength(1)
  await composer.get('textarea').trigger('keydown', { key: 'Escape' })
  expect(composer.emitted('cancelReply')).toHaveLength(2)
  await composer.setProps({ reply: null })
  expect(composer.find('[data-ui-id="reply-chip"]').exists()).toBe(false)
})

test('a reply with an empty draft cannot send', async () => {
  const composer = mount(Composer, {
    props: {
      channelId: 'channel',
      archived: false,
      busy: false,
      running: false,
      channel: 'general',
      reply: createReply(message, 'channel', 'Agent')!,
    },
  })
  expect(composer.get('[data-ui-id="composer-send"]').attributes('disabled')).toBeDefined()
  await composer.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(composer.emitted('send')).toBeUndefined()
})

const stored = (text: string): MessageDto => ({
  id: 'mine',
  role: 'user',
  continuesGroup: false,
  blocks: [{ type: 'markdown', text }],
})

test('a reply shows a link row and only the typed text, and keeps block indexes', async () => {
  const text = replyPrefix(message, 'channel', 'Agent') + 'Thanks, **that** helps'
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [
        {
          ...stored(text),
          blocks: [
            { type: 'markdown', text },
            { type: 'code', language: 'ts', text: 'const x = 1' },
          ],
        },
      ],
      live: [],
      running: false,
      newAnchorId: null,
    },
  })
  const row = list.get('[data-ui-id="reply-source"]')
  expect(row.attributes('href')).toBe('#discard-message/channel/source')
  expect(row.text()).toContain('Quinn [test]')
  expect(row.text()).toContain('First line Second line')
  const article = list.get('[data-ui-id="message-mine"]')
  expect(article.text()).toContain('Thanks, that helps')
  expect(article.text()).not.toContain('Message from')
  expect(article.find('blockquote').exists()).toBe(false)
  // The code block keeps index 1, so its menu target still points at the stored block.
  await list.get('[data-ui-id="message-mine"] [data-slot="code-block"]').trigger('contextmenu', {
    button: 2,
  })
  expect(invoke).toHaveBeenCalledWith('context_menu', {
    target: { type: 'code_block', messageId: 'mine', block: 1 },
  })
  list.unmount()
})

test('a reply row follows the markdown block, and a later quote shows as before', () => {
  const prefix = replyPrefix(message, 'channel', 'Agent')
  const later = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [stored('Before\n\n' + prefix + 'After')],
      live: [],
      running: false,
      newAnchorId: null,
    },
  })
  expect(later.find('[data-ui-id="reply-source"]').exists()).toBe(false)
  expect(later.get('[data-ui-id="message-mine"]').find('blockquote').exists()).toBe(true)
  later.unmount()
  const agentReply = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [{ ...stored(prefix + 'Hi'), role: 'agent' }],
      live: [],
      running: false,
      newAnchorId: null,
    },
  })
  expect(agentReply.find('[data-ui-id="reply-source"]').exists()).toBe(false)
  agentReply.unmount()
})

test('a reply with no typed text shows the row and no empty block', () => {
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [stored(replyPrefix(message, 'channel', 'Agent').trimEnd())],
      live: [],
      running: false,
      newAnchorId: null,
    },
  })
  expect(list.find('[data-ui-id="reply-source"]').exists()).toBe(true)
  expect(list.get('[data-ui-id="message-mine"]').find('blockquote').exists()).toBe(false)
  list.unmount()
})

test('message text can be selected and its toolbar cannot', () => {
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [message],
      live: [],
      running: false,
      newAnchorId: null,
    },
  })
  const trigger = list.get('[data-slot="context-menu-trigger"]')
  expect(trigger.classes()).toContain('select-text')
  expect(trigger.classes()).not.toContain('select-none')
  expect(list.get('[aria-label="Message actions"]').classes()).toContain('select-none')
  expect(list.get('button[aria-label="Reply to message"]').classes()).toContain('select-none')
  list.unmount()
})
