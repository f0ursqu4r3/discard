import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import fixture from '@/bridge/fixtures/thread.json'
import subAgentFixture from '@/bridge/fixtures/subagent.json'
import type {
  MessageDto,
  OpenThreadDto,
  PromptDto,
  SnapshotDto,
  ThreadHistoryDto,
} from '@/bridge/types'
import ThreadPanel from './ThreadPanel.vue'
import Composer from '../Composer.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: () => false }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('vue-sonner', () => ({ toast: { error: vi.fn() } }))
const views: ReturnType<typeof mount>[] = []
beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockResolvedValue(null)
})
afterEach(() => {
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
})
const thread = (fixture.snapshot as SnapshotDto).openThread as OpenThreadDto
const history = fixture.threadHistory as ThreadHistoryDto
function panel(props: Record<string, unknown> = {}) {
  const view = mount(ThreadPanel, {
    attachTo: document.body,
    props: { thread, history, parentChannelId: 'channel', parentName: 'general', ...props },
  })
  views.push(view)
  return view
}
const anchor: MessageDto = {
  id: 'anchor',
  role: 'user',
  continuesGroup: false,
  blocks: [{ type: 'markdown', text: 'Why is the build slow?' }],
}
const draft = () => panel({ thread: null, history: null, draftAnchor: anchor })

test('the header shows the thread name, the parent channel and a close button', async () => {
  const view = panel()
  const header = view.get('[data-ui-id="thread-header"]')
  expect(header.text()).toContain('Why a thread?')
  expect(header.text()).toContain('general')
  await header.get('[aria-label="Close thread"]').trigger('click')
  expect(view.emitted('close')).toHaveLength(1)
})

test('the transcript shows the anchor first, then the thread messages and the read-only note', () => {
  const view = panel()
  const ids = view
    .findAll('article[data-ui-id^="message-"]')
    .map((node) => node.attributes('data-ui-id'))
  expect(ids).toEqual(['message-answer', 'message-ask', 'message-reply'])
  expect(view.get('[data-testid="thread-divider"]').text()).toBe('2 messages')
  expect(view.get('[data-ui-id="thread-read-only-note"]').text()).toBe(thread.readOnlyNote)
})

test('the thread does not load channel memory', () => {
  panel()
  expect(invoke).not.toHaveBeenCalledWith('memory_get', expect.anything())
})

test('a followed thread offers Leave and a thread that is not followed offers Follow', async () => {
  const followed = panel()
  await followed.get('[aria-label="Leave thread"]').trigger('click')
  expect(invoke).toHaveBeenCalledWith('leave_thread', { threadId: 'thread' })
  const other = panel({ thread: { ...thread, followed: false } })
  expect(other.find('[aria-label="Leave thread"]').exists()).toBe(false)
  await other.get('[aria-label="Follow thread"]').trigger('click')
  expect(invoke).toHaveBeenCalledWith('follow_thread', { threadId: 'thread' })
})

test('the thread menu shows Rename and Delete as planned items', async () => {
  const view = panel()
  await view.get('[aria-label="Thread menu"]').trigger('keydown', { key: 'Enter' })
  await flushPromises()
  const items = [...document.querySelectorAll('[role="menuitem"]')]
  expect(items.map((item) => item.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
    'Rename thread Soon',
    'Delete thread Soon',
  ])
  for (const item of items) expect(item.getAttribute('aria-disabled')).toBe('true')
})

test('thread messages can post to the channel and show posted ones as done', () => {
  const view = panel()
  const ask = view.get('[data-ui-id="message-ask"]')
  expect(ask.find('[aria-label="Post to channel"]').exists()).toBe(true)
  const reply = view.get('[data-ui-id="message-reply"]')
  expect(reply.get('[aria-label="Posted to channel"]').attributes('disabled')).toBeDefined()
  // The anchor lives in the parent channel and has no Post action or thread action.
  const top = view.get('[data-ui-id="message-answer"]')
  expect(top.find('[aria-label="Post to channel"]').exists()).toBe(false)
  expect(top.find('[aria-label="Open thread"]').exists()).toBe(false)
  expect(top.find('[data-ui-id="thread-chip"]').exists()).toBe(false)
})

test('the composer sends the also-post choice and replies go to the thread', async () => {
  const view = panel()
  const composer = view.getComponent(Composer)
  expect(composer.props('alsoPostLabel')).toBe('#general')
  await view.get('textarea').setValue('More?')
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')?.[0]).toEqual(['More?', [], false])
  await view.get('[data-ui-id="composer-also-post"]').trigger('click')
  await view.get('textarea').setValue('Again')
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')?.[1]).toEqual(['Again', [], true])
  await view.get('[data-ui-id="message-reply"] [aria-label="Reply to message"]').trigger('click')
  expect(view.emitted('reply')?.[0]?.[0]).toMatchObject({ id: 'reply' })
})

test('a direct message parent names the conversation in the checkbox', () => {
  const view = panel({ directMessage: true, parentName: 'Agent' })
  expect(view.get('label').text()).toBe('Also post to the conversation')
})

test('a read-only thread disables the composer and the post action', () => {
  const view = panel({ thread: { ...thread, composerDisabled: true } })
  expect(view.get('textarea').attributes('disabled')).toBeDefined()
  expect(view.text()).toContain('Read only.')
  expect(
    view.get('[data-ui-id="message-ask"] [aria-label="Post to channel"]').attributes('disabled'),
  ).toBeDefined()
})

const pending: PromptDto = {
  kind: 'provider_tool',
  alwaysAllow: 'unavailable',
  approvalId: 'approval-1',
  tool: 'Bash',
  summary: 'ls',
  view: 'pending',
}

test('an approval in the thread is decided against the thread channel', async () => {
  const view = panel({
    thread: {
      ...thread,
      turnRunning: true,
      pendingApprovals: 1,
      prompts: [pending],
      liveBlocks: [{ type: 'approval', approval_id: 'approval-1', tool: 'Bash', summary: 'ls' }],
    },
  })
  const approve = view
    .findAll('[data-ui-id="approval-card"] button')
    .find((b) => b.text() === 'Approve')!
  await approve.trigger('click')
  expect(invoke).toHaveBeenCalledWith('decide_approval', {
    channelId: 'thread',
    approvalId: 'approval-1',
    decision: 'approve',
    ruleCut: undefined,
  })
})

test('Stop replaces Send while the thread turn runs and asks the app to stop', async () => {
  const view = panel({ thread: { ...thread, turnRunning: true } })
  await view.get('[data-ui-id="composer-stop"]').trigger('click')
  expect(view.emitted('stop')).toHaveLength(1)
  expect(view.text()).toContain('is thinking...')
})

test('the thread notice shows', () => {
  const view = panel({ thread: { ...thread, notice: 'The provider stopped.' } })
  expect(view.text()).toContain('The provider stopped.')
})

test('a transcript of another thread does not show while the open thread loads', () => {
  const view = panel({ history: { ...history, channelId: 'older' } })
  expect(view.text()).toContain('Opening thread')
  expect(view.find('[data-ui-id="message-ask"]').exists()).toBe(false)
  expect(view.find('textarea').exists()).toBe(true)
})

test('a missing anchor leaves the thread messages alone', () => {
  const view = panel({ history: { ...history, anchor: null } })
  expect(view.find('[data-ui-id="message-answer"]').exists()).toBe(false)
  expect(view.find('[data-ui-id="message-ask"]').exists()).toBe(true)
})

test('a draft shows the anchor and an empty composer without a thread', () => {
  const view = draft()
  expect(view.get('[data-ui-id="thread-header"]').text()).toContain('New thread')
  expect(view.find('[data-ui-id="message-anchor"]').exists()).toBe(true)
  expect(view.text()).toContain('Why is the build slow?')
  expect(view.text()).toContain('Send a message to start the thread')
  expect(view.find('[aria-label="Follow thread"]').exists()).toBe(false)
  expect(view.find('[aria-label="Thread menu"]').exists()).toBe(false)
  expect(view.get('textarea').attributes('placeholder')).toBe('Ask about this message')
  expect(invoke).not.toHaveBeenCalled()
})

test('a draft sends its first message and the choice to post it', async () => {
  const view = draft()
  await view.get('[data-ui-id="composer-also-post"]').trigger('click')
  await view.get('textarea').setValue('Why?')
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')).toEqual([['Why?', [], true]])
  expect(invoke).not.toHaveBeenCalled()
})

test('a draft names an agent anchor without a saved author after the parent agent', () => {
  const view = panel({
    thread: null,
    history: null,
    draftAnchor: { ...anchor, role: 'agent' },
    parentAgent: 'Quinn',
    parentAgentId: 'agent-1',
  })
  expect(view.get('[data-testid="message-identity"]').text()).toBe('Quinn')
})

const subThread = (subAgentFixture.snapshot as SnapshotDto).openThread as OpenThreadDto
const subHistory = subAgentFixture.threadHistory as ThreadHistoryDto
const briefAuthor = { agentId: 'parent', name: 'Quinn', provider: 'claude', avatar: null }
function subPanel(patch: Partial<OpenThreadDto> = {}, props: Record<string, unknown> = {}) {
  return panel({
    thread: { ...subThread, ...patch },
    history: {
      ...subHistory,
      messages: subHistory.messages.map((message) => ({ ...message, author: briefAuthor })),
    },
    parentAgent: 'Quinn',
    ...props,
  })
}

test('a sub-agent thread header shows the title, the sub-agent and the state', () => {
  const view = subPanel()
  const header = view.get('[data-ui-id="thread-header"]')
  expect(header.text()).toContain('Find callers')
  expect(header.get('[data-ui-id="thread-agent"]').text()).toBe('Reviewer')
  expect(header.get('[data-ui-id="thread-state"]').text()).toContain('Running')
  const done = subPanel({ state: 'done', turnRunning: false })
  expect(done.get('[data-ui-id="thread-state"]').text()).toBe('Done')
  // A question thread shows neither.
  expect(panel().find('[data-ui-id="thread-state"]').exists()).toBe(false)
})

test('a sub-agent thread with no anchor shows no anchor row and no divider', () => {
  const view = subPanel()
  expect(subHistory.anchor).toBeNull()
  const ids = view
    .findAll('article[data-ui-id^="message-"]')
    .map((node) => node.attributes('data-ui-id'))
  expect(ids).toEqual(['message-brief'])
  expect(view.find('[data-testid="thread-divider"]').exists()).toBe(false)
})

test('a sub-agent thread with an anchor still shows it above the brief', () => {
  const view = subPanel(
    { anchorMessageId: 'answer' },
    {
      history: {
        ...subHistory,
        anchor: (history as ThreadHistoryDto).anchor,
      },
    },
  )
  const ids = view
    .findAll('article[data-ui-id^="message-"]')
    .map((node) => node.attributes('data-ui-id'))
  expect(ids).toEqual(['message-answer', 'message-brief'])
  expect(view.get('[data-testid="thread-divider"]').text()).toBe('1 message')
})

test('the brief shows the parent agent as its author', () => {
  const view = subPanel()
  const brief = view.get('[data-ui-id="message-brief"]')
  expect(brief.get('[data-testid="message-identity"]').text()).toBe('Quinn')
  expect(brief.text()).toContain('Find the callers of foo.')
  expect(brief.text()).not.toContain('You')
})

test('the sub-agent composer is on and offers Also post like a question thread', () => {
  const view = subPanel()
  expect(view.get('textarea').attributes('disabled')).toBeUndefined()
  expect(view.find('[data-ui-id="composer-soon"]').exists()).toBe(false)
  expect(view.text()).not.toContain('Soon')
  expect(view.find('[data-ui-id="composer-also-post"]').exists()).toBe(true)
  expect(view.getComponent(Composer).props('alsoPostLabel')).toBe('#general')
  expect(view.text()).not.toContain('Channel archived')
  expect(view.text()).not.toContain('Read only.')
  // The read-only note about file edits still shows.
  expect(view.get('[data-ui-id="thread-read-only-note"]').text()).toBe(subThread.readOnlyNote)
})

test('the sub-agent placeholder says steer while the task runs and reply after', () => {
  const running = subPanel({ state: 'running', turnRunning: true })
  expect(running.get('textarea').attributes('placeholder')).toBe('Steer this sub-agent')
  for (const state of ['done', 'failed', 'cancelled', 'timed_out'] as const) {
    const view = subPanel({ state, turnRunning: false })
    expect(view.get('textarea').attributes('placeholder')).toBe('Reply in thread')
  }
  // A question thread keeps its own placeholder.
  expect(panel().get('textarea').attributes('placeholder')).toBe('Reply in thread')
})

test('a read-only reason turns the sub-agent composer off with the read-only line', () => {
  const view = subPanel({ composerDisabled: true, composerDisabledReason: 'read_only' })
  expect(view.get('textarea').attributes('disabled')).toBeDefined()
  expect(view.find('[data-ui-id="composer-soon"]').exists()).toBe(false)
  expect(view.text()).toContain('Read only.')
})

test('a running sub-agent thread shows Stop and Send, and Send sends a steer', async () => {
  const view = subPanel()
  expect(view.find('[data-ui-id="composer-stop"]').exists()).toBe(true)
  const send = view.get('[data-ui-id="composer-send"]')
  expect(send.attributes('disabled')).toBeDefined()
  await view.get('textarea').setValue('Only look in tests.')
  expect(send.attributes('disabled')).toBeUndefined()
  await send.trigger('click')
  expect(view.emitted('send')?.[0]).toEqual(['Only look in tests.', [], false])
  await view.get('[data-ui-id="composer-stop"]').trigger('click')
  expect(view.emitted('stop')).toHaveLength(1)
})

test('Enter sends a steer while the sub-agent works', async () => {
  const view = subPanel()
  await view.get('textarea').setValue('Stop checking docs.')
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')?.[0]).toEqual(['Stop checking docs.', [], false])
})

test('after the task ended the thread shows Send only, and a question thread cannot send while it runs', async () => {
  const done = subPanel({ turnRunning: false, state: 'done' })
  expect(done.find('[data-ui-id="composer-stop"]').exists()).toBe(false)
  expect(done.get('[data-ui-id="composer-send"]').attributes('disabled')).toBeDefined()
  // A follow-up turn runs in a finished thread: Stop shows, Send does not.
  const followUp = subPanel({ turnRunning: true, state: 'done' })
  expect(followUp.find('[data-ui-id="composer-stop"]').exists()).toBe(true)
  expect(followUp.find('[data-ui-id="composer-send"]').exists()).toBe(false)
  const question = panel({ thread: { ...thread, turnRunning: true } })
  expect(question.find('[data-ui-id="composer-stop"]').exists()).toBe(true)
  expect(question.find('[data-ui-id="composer-send"]').exists()).toBe(false)
})

test('Post to channel, Follow and Leave keep working in a sub-agent thread', async () => {
  const view = subPanel()
  const post = view.get('[data-ui-id="message-brief"] [aria-label="Post to channel"]')
  expect(post.attributes('disabled')).toBeUndefined()
  await post.trigger('click')
  expect(invoke).toHaveBeenCalledWith('post_thread_message', { messageId: 'brief' })
  expect(
    view.get('[data-ui-id="message-brief"] [aria-label="Reply to message"]').attributes('disabled'),
  ).toBeUndefined()
  await view.get('[aria-label="Leave thread"]').trigger('click')
  expect(invoke).toHaveBeenCalledWith('leave_thread', { threadId: 'find-callers' })
  const other = subPanel({ followed: false })
  await other.get('[aria-label="Follow thread"]').trigger('click')
  expect(invoke).toHaveBeenCalledWith('follow_thread', { threadId: 'find-callers' })
})

test('a sub-agent thread offers no way to start another thread and keeps Rename and Delete planned', async () => {
  const view = subPanel()
  expect(view.find('[aria-label="Start thread"]').exists()).toBe(false)
  await view.get('[aria-label="Thread menu"]').trigger('keydown', { key: 'Enter' })
  await flushPromises()
  const items = [...document.querySelectorAll('[role="menuitem"]')]
  expect(items.map((item) => item.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
    'Rename thread Soon',
    'Delete thread Soon',
  ])
})
