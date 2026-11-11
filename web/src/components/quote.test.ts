import { expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import { appendQuote, messageSourceLink, parseMessageSource, quoteMessage } from './quote'
import type { MessageDto } from '../bridge/types'
import SafeMarkdown from './SafeMarkdown.vue'
import MessageList from './MessageList.vue'
import Composer from './Composer.vue'
import { defineComponent, h, ref } from 'vue'
import { ContextMenuItem } from './ui/context-menu'

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

test('quote preserves paragraphs and identifies the saved author and source', () => {
  const quote = quoteMessage(message, 'channel', 'Other agent')
  expect(quote).toContain('> First line\n> \n> Second line\n\n')
  const view = mount(SafeMarkdown, { props: { text: quote } })
  expect(view.get('blockquote a').text()).toBe('Message from Quinn [test]')
  expect(view.get('a').attributes('href')).toBe('#discard-message/channel/source')
  expect(view.text()).not.toContain('Other agent')
})

test('quote preserves code and diff blocks, including embedded fences', () => {
  const quote = quoteMessage(
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
  expect(quote).toContain('> ````md\n> ```js')
  expect(quote).toContain('> ```diff\n> -old\n> +new\n> ```')
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

test('appending a quote preserves the existing draft and leaves room for a reply', () => {
  const quote = quoteMessage(message, 'channel', 'Agent')
  expect(appendQuote('Unsent draft  ', quote)).toBe('Unsent draft  \n\n' + quote)
  expect(appendQuote('', quote)).toBe(quote)
  expect(appendQuote('Draft\n', quote)).toBe('Draft\n\n' + quote)
  expect(appendQuote('Draft', '')).toBe('Draft')
  expect(quoteMessage({ ...message, blocks: [] }, 'channel', 'Agent')).toBe('')
})

test('the quote icon emits the selected message through the transcript', async () => {
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [message],
      live: [],
      running: false,
      newAnchorId: null,
    },
  })
  await list.get('button[aria-label="Quote message"]').trigger('click')
  expect(list.emitted('quote')?.[0]?.[0]).toMatchObject({ id: 'source', blocks: message.blocks })
  await list.setProps({ quoteDisabled: true })
  expect(list.get('button[aria-label="Quote message"]').attributes('disabled')).toBeDefined()
  await list.get('button[aria-label="Quote message"]').trigger('click')
  expect(list.emitted('quote')).toHaveLength(1)
  expect(list.get('button[aria-label="Start thread"]').attributes('disabled')).toBeUndefined()
  await list.setProps({ threadDisabled: true })
  expect(list.get('button[aria-label="Start thread"]').attributes('disabled')).toBeDefined()
  list.unmount()
})

test('quoting from the context menu closes the menu before focusing the composer', async () => {
  const send = vi.fn()
  const Host = defineComponent({
    setup() {
      const draft = ref('Draft')
      const composer = ref<InstanceType<typeof Composer> | null>(null)
      return () =>
        h('div', [
          h(MessageList, {
            conversationId: 'channel',
            messages: [message],
            live: [],
            running: false,
            newAnchorId: null,
            onQuote: (source) => {
              draft.value = appendQuote(draft.value, quoteMessage(source, 'channel', 'Quinn'))
              void composer.value?.focusEnd()
            },
          }),
          h(Composer, {
            ref: composer,
            archived: false,
            busy: false,
            running: false,
            channel: 'general',
            draft: draft.value,
            'onUpdate:draft': (value) => {
              draft.value = value
            },
            onSend: send,
          }),
        ])
    },
  })
  const view = mount(Host, { attachTo: document.body })
  try {
    await view
      .get('[data-ui-id="message-source"]')
      .trigger('contextmenu', { button: 2, clientX: 20, clientY: 20 })
    const item = view.findAllComponents(ContextMenuItem).find((item) => item.text() === 'Quote')!
    await item.trigger('click')
    await vi.waitFor(() => {
      const input = view.get('textarea').element
      expect(input.value).toContain('Draft\n\n> [Message from Quinn')
      expect(document.activeElement).toBe(input)
      expect(input.selectionStart).toBe(input.value.length)
    })
    expect(send).not.toHaveBeenCalled()
  } finally {
    view.unmount()
  }
})
