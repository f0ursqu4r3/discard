import { mount, flushPromises } from '@vue/test-utils'
import { expect, test } from 'vite-plus/test'
import MessageRow from './MessageRow.vue'
import DiffBlock from './DiffBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import MessageList from './MessageList.vue'
import { hasLiveRow, thinkingLine, messagePresentation } from './messages'
import type { MessageDto } from '../bridge/types'

test('continued messages repeat their identity and leave the timestamp gutter empty', () => {
  const row = mount(MessageRow, {
    props: {
      channelId: 'channel',
      message: { id: 'a', role: 'agent', blocks: [], continuesGroup: true },
    },
  })
  expect(row.get('[data-testid="message-identity"]').text()).toBe('Agent')
  expect(row.get('[data-testid="message-time-gutter"]').text()).toBe('')
})
test('timestamps restart for a new author, minute, or local date', () => {
  const message: MessageDto = {
    id: 'a',
    role: 'agent',
    blocks: [],
    continuesGroup: true,
    createdAt: new Date(2026, 10, 6, 23, 59, 10).getTime(),
    author: { agentId: 'one', name: 'One', provider: 'test', avatar: null },
  }
  expect(messagePresentation(message)).toMatchObject({ startsDate: true, continuesGroup: false })
  expect(
    messagePresentation({ ...message, createdAt: message.createdAt! + 10_000 }, message),
  ).toMatchObject({ startsDate: false, continuesGroup: true })
  expect(
    messagePresentation({ ...message, author: { ...message.author!, agentId: 'two' } }, message)
      .continuesGroup,
  ).toBe(false)
  expect(
    messagePresentation({ ...message, createdAt: message.createdAt! + 60_000 }, message),
  ).toMatchObject({ startsDate: true, continuesGroup: false })
  expect(
    messagePresentation({ ...message, createdAt: message.createdAt! - 60_000 }, message),
  ).toMatchObject({ startsDate: false, continuesGroup: false })
  expect(messagePresentation({ ...message, createdAt: null }, message).date).toBeNull()
  expect(messagePresentation({ ...message, createdAt: NaN }, message).date).toBeNull()
})

test('date dividers precede messages and the unread marker across midnight', () => {
  const before = new Date(2026, 10, 6, 23, 59, 10).getTime()
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      live: [],
      running: false,
      newAnchorId: 'c',
      messages: [
        {
          id: 'a',
          role: 'user',
          blocks: [{ type: 'markdown', text: 'First' }],
          continuesGroup: false,
          createdAt: before,
        },
        {
          id: 'b',
          role: 'user',
          blocks: [{ type: 'markdown', text: 'Second' }],
          continuesGroup: true,
          createdAt: before + 10_000,
        },
        {
          id: 'c',
          role: 'user',
          blocks: [{ type: 'markdown', text: 'Next day' }],
          continuesGroup: true,
          createdAt: before + 60_000,
        },
      ],
    },
  })
  expect(list.findAll('[data-testid="date-divider"]')).toHaveLength(2)
  expect(list.findAll('[data-testid="message-timestamp"]')).toHaveLength(2)
  expect(list.findAll('[data-testid="message-identity"]').map((item) => item.text())).toEqual([
    'You',
    'You',
    'You',
  ])
  const last = list.get('[data-message-id="c"]')
  const markers = last.findAll('[data-testid="date-divider"], [data-testid="new-divider"], article')
  expect(markers.map((item) => item.attributes('data-testid') ?? item.element.tagName)).toEqual([
    'date-divider',
    'new-divider',
    'ARTICLE',
  ])
})

test('diff lines have their matching types', () => {
  const diff = mount(DiffBlock, { props: { text: '+added\n-removed\n@@ change' } })
  expect(diff.find('[data-diff="added"]').text()).toBe('+added')
  expect(diff.find('[data-diff="removed"]').text()).toBe('-removed')
  expect(diff.find('[data-diff="hunk"]').text()).toBe('@@ change')
})
test('long tool output can be expanded', async () => {
  const output = Array.from({ length: 21 }, (_, i) => `line ${i}`).join('\n')
  const block = mount(ToolCallBlock, { props: { name: 'shell', input: {}, output } })
  expect(block.text()).toContain('Shell')
  expect(block.text()).not.toContain('line 20')
  await block.get('button').trigger('click')
  expect(block.text()).toContain('line 20')
})
test('tool details hide raw data until opened and preserve the complete input', async () => {
  const name = 'mcp__discard-memory__memory_search'
  const input = { limit: 10, query: 'wikijs', scope: 'project' }
  const block = mount(ToolCallBlock, { props: { name, input, output: 'no matches' } })
  const trigger = block.get('button')
  expect(trigger.text()).toContain('Memory search')
  expect(trigger.text()).toContain('wikijs')
  expect(trigger.text()).not.toContain('mcp__')
  expect(trigger.attributes('aria-expanded')).toBe('false')
  expect(block.text()).not.toContain('no matches')
  await trigger.trigger('click')
  expect(trigger.attributes('aria-expanded')).toBe('true')
  expect(block.text()).toContain(name)
  expect(block.get('[aria-label="Tool input"]').text()).toBe(JSON.stringify(input, null, 2))
  expect(block.get('[aria-label="Tool output"]').text()).toBe('no matches')
  await trigger.trigger('click')
  expect(trigger.attributes('aria-expanded')).toBe('false')
})

test('tool results distinguish missing and empty output without claiming success', async () => {
  const block = mount(ToolCallBlock, {
    props: { name: 'shell', input: {}, output: null },
  })
  expect(block.get('button').text()).toContain('Awaiting result')
  expect(block.get('button').text()).not.toContain('{}')
  await block.get('button').trigger('click')
  expect(block.text()).toContain('No result received yet.')
  await block.setProps({ output: '' })
  expect(block.get('button').text()).toContain('Returned')
  expect(block.text()).toContain('No output.')
  await block.setProps({ output: 'Error: permission denied' })
  expect(block.get('[aria-label="Tool output"]').text()).toBe('Error: permission denied')
  expect(block.get('button').text()).not.toContain('Success')
})

test('expanded tool output retains diff rendering', async () => {
  const block = mount(ToolCallBlock, {
    props: { name: 'apply_patch', input: {}, output: '@@ -1 +1 @@\n-old\n+new' },
  })
  expect(block.findComponent(DiffBlock).exists()).toBe(false)
  await block.get('button').trigger('click')
  expect(block.find('[data-diff="added"]').text()).toBe('+new')
  expect(block.find('[data-diff="removed"]').text()).toBe('-old')
})

test('new divider is before its message', () => {
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [{ id: 'a', role: 'user', blocks: [], continuesGroup: false }],
      live: [],
      running: false,
      newAnchorId: 'a',
    },
  })
  expect(list.text()).toContain('New')
  expect(list.find('[data-testid="new-divider"]').exists()).toBe(true)
})
test('live row and thinking line follow running state', () => {
  expect(hasLiveRow(false, 0)).toBe(false)
  expect(hasLiveRow(false, 2)).toBe(true)
  expect(hasLiveRow(true, 0)).toBe(false)
  expect(hasLiveRow(true, 1)).toBe(true)
  expect(thinkingLine(false, ['Claude'])).toBeNull()
  expect(thinkingLine(true, [])).toBeNull()
  expect(thinkingLine(true, ['Claude'])).toBe('Claude is thinking...')
  expect(thinkingLine(true, ['Claude', 'Codex'])).toBe('Claude and Codex are thinking...')
  expect(thinkingLine(true, ['A', 'B', 'C', 'D'])).toBe('A, B and 2 more are thinking...')
})
test('stream cursor appears only when running', async () => {
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      messages: [],
      live: [{ type: 'markdown', text: 'Hi' }],
      running: true,
      newAnchorId: null,
    },
  })
  expect(list.find('[data-testid="stream-cursor"]').exists()).toBe(true)
  await list.setProps({ running: false })
  expect(list.find('[data-testid="stream-cursor"]').exists()).toBe(false)
})

test('row count includes a live row only when present', async () => {
  const { rowCount } = await import('./messages')
  expect(rowCount(0, false)).toBe(0)
  expect(rowCount(3, false)).toBe(3)
  expect(rowCount(3, true)).toBe(4)
})

test('diff file headers remain context lines', () => {
  const diff = mount(DiffBlock, { props: { text: '--- a/file\n+++ b/file\n+added\n-removed' } })
  expect(diff.findAll('[data-diff="context"]')).toHaveLength(2)
})
test('tool output identifies unified diffs without treating lists as diffs', async () => {
  const { looksLikeUnifiedDiff } = await import('./messages')
  expect(looksLikeUnifiedDiff('@@ -1 +1 @@\n-old\n+new')).toBe(true)
  expect(looksLikeUnifiedDiff('--- a/file\n+++ b/file\n')).toBe(true)
  expect(looksLikeUnifiedDiff('- item\n- another')).toBe(false)
  expect(looksLikeUnifiedDiff('--- a/file\ncontext\n+++ b/file')).toBe(false)
  expect(looksLikeUnifiedDiff('@@ not a hunk')).toBe(false)
})

test('a fenced Markdown block uses the code component and offers copy', async () => {
  const { default: SafeMarkdown } = await import('./SafeMarkdown.vue')
  const { default: CodeBlock } = await import('./CodeBlock.vue')
  const markdown = mount(SafeMarkdown, {
    props: { text: 'Before\n\n```rust\nlet x = 1;\n```\n\nAfter' },
  })
  await flushPromises()
  expect(markdown.text()).toContain('Before')
  expect(markdown.text()).toContain('After')
  const code = markdown.findComponent(CodeBlock)
  expect(code.exists()).toBe(true)
  expect(code.text()).toContain('let x = 1;')
  expect(code.get('[data-slot="code-block-copy-button"]').attributes('aria-label')).toBe('Copy')
  expect(code.find('[data-slot="code-block-content"]').exists()).toBe(true)
})

test('a fenced block nested in a list uses the code component', async () => {
  const { default: SafeMarkdown } = await import('./SafeMarkdown.vue')
  const { default: CodeBlock } = await import('./CodeBlock.vue')
  const markdown = mount(SafeMarkdown, {
    props: { text: '- Item\n\n  ```rust\n  let x = 1;\n  ```' },
  })
  await flushPromises()
  expect(markdown.findComponent(CodeBlock).exists()).toBe(true)
  expect(markdown.find('li [data-markdown-code]').exists()).toBe(true)
})

test('long tool output uses the collapsible component', () => {
  const output = Array.from({ length: 21 }, (_, i) => `line ${i}`).join('\n')
  const block = mount(ToolCallBlock, { props: { name: 'shell', input: {}, output } })
  expect(block.find('[data-slot="collapsible"]').exists()).toBe(true)
})

test('tool call summary uses the preferred field and truncates long input', () => {
  const block = mount(ToolCallBlock, {
    props: { name: 'shell', input: { command: 'x'.repeat(121), extra: 'hidden' }, output: null },
  })
  expect(block.text()).toContain(`${'x'.repeat(120)}…`)
  expect(block.text()).not.toContain('hidden')
})

test('model-change and context-summary system notices stay visible in history', () => {
  const view = mount(MessageList, {
    props: {
      conversationId: 'direct',
      running: false,
      live: [],
      newAnchorId: null,
      messages: [
        {
          id: 'model',
          role: 'system',
          blocks: [{ type: 'markdown', text: 'Model changed from old to exact-model.' }],
          continuesGroup: false,
        },
        {
          id: 'summary',
          role: 'system',
          blocks: [{ type: 'markdown', text: 'Older messages use a saved context summary.' }],
          continuesGroup: true,
        },
      ],
    },
  })
  expect(view.findAll('[data-slot="marker"]')).toHaveLength(2)
  expect(view.find('[data-testid="message-identity"]').exists()).toBe(false)
  expect(view.find('[data-slot="avatar"]').exists()).toBe(false)
  expect(view.text()).toContain('Model changed from old to exact-model.')
  expect(view.text()).toContain('Older messages use a saved context summary.')
})
test('an unsaved reply remains visible and has no streaming cursor after the turn stops', () => {
  const view = mount(MessageList, {
    props: {
      conversationId: 'direct',
      messages: [],
      live: [{ type: 'markdown', text: 'Copy this unsaved reply.' }],
      running: false,
      newAnchorId: null,
    },
  })
  expect(view.text()).toContain('Copy this unsaved reply.')
  expect(view.find('[data-testid="stream-cursor"]').exists()).toBe(false)
})

test('saved and live replies use their own author, not the channel default', () => {
  const jordan = { agentId: 'jordan', name: 'Jordan', provider: 'test', avatar: null }
  const quinn = { agentId: 'quinn', name: 'Quinn', provider: 'test', avatar: null }
  const list = mount(MessageList, {
    props: {
      conversationId: 'channel',
      agent: 'Quinn',
      newAnchorId: null,
      messages: [{ id: 'old', role: 'agent', blocks: [], continuesGroup: false, author: quinn }],
      live: [{ type: 'markdown', text: 'Jordan reply' }],
      liveAuthor: jordan,
      running: true,
    },
  })
  expect(list.findAll('[data-testid="message-identity"]').map((row) => row.text())).toEqual([
    'Quinn',
    'Jordan',
  ])
})
