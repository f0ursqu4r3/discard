import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import type {
  ChangeFileDto,
  ReviewDecisionDto,
  ReviewDto,
  ReviewSnapshotDto,
  ReviewStateDto,
} from '../../../bridge/types'
import ReviewView from '../ReviewView.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const HEAD = 'a'.repeat(40)
const file = (path: string): ChangeFileDto => ({
  path,
  oldPath: null,
  status: 'M',
  additions: 1,
  deletions: 1,
  binary: false,
  diff: `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1,2 +1,2 @@\n keep\n-old ${path}\n+new ${path}\n`,
  truncated: false,
})
const snapshot = (patch: Partial<ReviewSnapshotDto> = {}): ReviewSnapshotDto => ({
  source: 'feature',
  target: 'main',
  head: HEAD,
  headShort: 'aaaaaaa',
  base: 'b'.repeat(40),
  baseShort: 'bbbbbbb',
  ahead: 2,
  behind: 1,
  nothingToReview: false,
  merged: false,
  dirty: 0,
  commits: [
    { id: 'c1c1c1c', subject: 'Add the thing', author: 'Ana', time: 1700000000 },
    { id: 'd2d2d2d', subject: 'Fix the thing', author: 'Bo', time: 1700000100 },
  ],
  files: [file('src/a.ts'), file('b.md')],
  truncated: false,
  reason: null,
  ...patch,
})
const decision = (patch: Partial<ReviewDecisionDto> = {}): ReviewDecisionDto => ({
  id: 'r1',
  channelId: 'c1',
  decision: 'approved',
  head: HEAD,
  base: 'b'.repeat(40),
  target: 'main',
  note: null,
  decidedAt: 1700000000,
  postedAt: null,
  ...patch,
})
const review = (patch: Partial<ReviewDto> = {}): ReviewDto => ({
  snapshot: snapshot(),
  state: 'not_reviewed',
  decisions: [],
  blocked: null,
  unposted: null,
  ...patch,
})
const calls = (name: string) => vi.mocked(invoke).mock.calls.filter(([n]) => n === name)
const views: VueWrapper[] = []
async function open(
  handlers: Record<string, (args: Record<string, unknown>) => unknown>,
  props: { turnRunning?: boolean } = {},
) {
  vi.mocked(invoke).mockImplementation(async (name, args) => {
    const handler = handlers[name]
    if (!handler) throw new Error(`unexpected ${name}`)
    return handler((args ?? {}) as Record<string, unknown>)
  })
  const view = mount(ReviewView, {
    attachTo: document.body,
    props: { channelId: 'c1', turnRunning: props.turnRunning ?? false },
  })
  views.push(view)
  await flushPromises()
  return view
}
const show = (dto: ReviewDto, props: { turnRunning?: boolean } = {}) =>
  open({ channel_review: () => dto }, props)
const button = (view: VueWrapper, label: string) => {
  const found = view.findAll('button').find((item) => item.text() === label)
  if (!found) throw new Error(`no button ${label}`)
  return found
}
const press = async (view: VueWrapper, label: string) => {
  await button(view, label).trigger('click')
  await flushPromises()
}
const hasButton = (view: VueWrapper, label: string) =>
  view.findAll('button').some((item) => item.text() === label)

beforeEach(() => {
  localStorage.clear()
  vi.mocked(invoke).mockReset()
})
afterEach(() => {
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
})

test('loads the review when it mounts and shows source → target, counts, commits, files', async () => {
  const view = await show(review())
  expect(calls('channel_review')).toEqual([['channel_review', { channelId: 'c1' }]])
  const header = view.get('[data-ui-id="review-header"]').text()
  expect(header).toContain('feature')
  expect(header).toContain('main')
  expect(header).toContain('2 ahead')
  expect(header).toContain('1 behind')
  const commits = view.get('[data-ui-id="review-commits"]').text()
  expect(commits).toContain('Add the thing')
  expect(commits).toContain('c1c1c1c')
  expect(commits).toContain('Fix the thing')
  expect(view.findAll('[data-tree-file]').map((row) => row.attributes('data-path'))).toEqual([
    'src/a.ts',
    'b.md',
  ])
})

test.each<[ReviewStateDto, Partial<ReviewDto>, string | null]>([
  ['not_reviewed', {}, null],
  ['approved', {}, 'Approved'],
  [
    'approval_out_of_date',
    { decisions: [decision({ head: 'f'.repeat(40) })] },
    'Approval out of date',
  ],
  ['changes_requested', {}, 'Changes requested'],
  ['changes_requested_new_commits', {}, 'Changes requested · new commits since'],
])('state %s shows its label', async (state, patch, label) => {
  const view = await show(review({ state, ...patch }))
  const line = view.get('[data-ui-id="review-state"]')
  if (label === null) expect(line.text()).not.toMatch(/Approved|Changes requested|out of date/)
  else expect(line.text()).toContain(label)
})

test('an out of date approval shows the old and the new short id', async () => {
  const view = await show(
    review({
      state: 'approval_out_of_date',
      decisions: [decision({ head: 'f'.repeat(40) })],
    }),
  )
  const text = view.get('[data-ui-id="review-state"]').text()
  expect(text).toContain('fffffff')
  expect(text).toContain('aaaaaaa')
})

test('merged shows its text and no actions', async () => {
  const view = await show(
    review({ state: 'merged', snapshot: snapshot({ merged: true, ahead: 0 }) }),
  )
  expect(view.text()).toContain('This branch is already merged.')
  expect(hasButton(view, 'Approve')).toBe(false)
  expect(hasButton(view, 'Request changes')).toBe(false)
  expect(hasButton(view, 'Ask for a summary')).toBe(false)
})

test('nothing to review shows "Nothing to review: no commits beyond main" and no actions', async () => {
  const view = await show(
    review({
      state: 'nothing_to_review',
      snapshot: snapshot({ nothingToReview: true, ahead: 0, commits: [], files: [] }),
    }),
  )
  expect(view.text()).toContain('Nothing to review: no commits beyond main')
  expect(hasButton(view, 'Approve')).toBe(false)
  expect(hasButton(view, 'Ask for a summary')).toBe(false)
})

test('a reason shows an empty state with the reason and no actions', async () => {
  const view = await show(
    review({
      snapshot: snapshot({
        reason: 'This channel is on the default branch.',
        commits: [],
        files: [],
        head: null,
      }),
    }),
  )
  expect(view.get('[data-ui-id="review-unavailable"]').text()).toContain(
    'This channel is on the default branch.',
  )
  expect(hasButton(view, 'Approve')).toBe(false)
  expect(hasButton(view, 'Request changes')).toBe(false)
})

test('the uncommitted notice shows the count and says those files are not reviewed', async () => {
  const view = await show(review({ snapshot: snapshot({ dirty: 3 }) }))
  const notice = view.get('[data-ui-id="review-dirty"]').text()
  expect(notice).toContain('3')
  expect(notice).toContain('not reviewed')
})

test('approve calls reviewDecide with the pinned head and a null note', async () => {
  const view = await open({
    channel_review: () => review(),
    review_decide: () => review({ state: 'approved', decisions: [decision()] }),
  })
  await press(view, 'Approve')
  expect(calls('review_decide')).toEqual([
    ['review_decide', { channelId: 'c1', decision: 'approved', head: HEAD, note: null }],
  ])
  expect(view.get('[data-ui-id="review-state"]').text()).toContain('Approved')
})

test('request changes needs a non-empty note and sends it with the pinned head', async () => {
  const view = await open({
    channel_review: () => review(),
    review_decide: () => review({ state: 'changes_requested', decisions: [decision()] }),
  })
  await press(view, 'Request changes')
  const field = view.get('textarea')
  expect(field.attributes('maxlength')).toBe('4000')
  const send = button(view, 'Send request')
  expect(send.attributes('disabled')).toBeDefined()
  await field.setValue('   ')
  expect(button(view, 'Send request').attributes('disabled')).toBeDefined()
  await field.setValue('  Please rename it.  ')
  await press(view, 'Send request')
  expect(calls('review_decide')).toEqual([
    [
      'review_decide',
      { channelId: 'c1', decision: 'changes_requested', head: HEAD, note: 'Please rename it.' },
    ],
  ])
  expect(view.find('textarea').exists()).toBe(false)
})

test('actions are disabled with the blocked reason shown, not hidden', async () => {
  const view = await show(review({ blocked: 'A turn is running in this channel.' }))
  expect(view.text()).toContain('A turn is running in this channel.')
  expect(button(view, 'Request changes').attributes('disabled')).toBeDefined()
  expect(button(view, 'Ask for a summary').attributes('disabled')).toBeDefined()
  // Approve stays allowed while the channel is busy.
  expect(button(view, 'Approve').attributes('disabled')).toBeUndefined()
})

test('a running turn disables request changes and the summary', async () => {
  const view = await show(review(), { turnRunning: true })
  expect(button(view, 'Request changes').attributes('disabled')).toBeDefined()
  expect(button(view, 'Ask for a summary').attributes('disabled')).toBeDefined()
  expect(button(view, 'Approve').attributes('disabled')).toBeUndefined()
})

test('an archived channel disables every action and shows the reason', async () => {
  const view = await show(review({ blocked: 'This channel is archived.' }))
  expect(view.text()).toContain('This channel is archived.')
  expect(button(view, 'Approve').attributes('disabled')).toBeDefined()
  expect(button(view, 'Request changes').attributes('disabled')).toBeDefined()
})

test('a head-changed error reloads the review and shows the message', async () => {
  let loads = 0
  const view = await open({
    channel_review: () => {
      loads++
      return review()
    },
    review_decide: () => {
      throw 'The branch changed. Reload the review.'
    },
  })
  await press(view, 'Approve')
  expect(loads).toBe(2)
  expect(view.get('[role="alert"]').text()).toContain('The branch changed. Reload the review.')
})

test('an unposted request shows its error and a Retry that calls reviewRetryPost', async () => {
  const failed = decision({
    id: 'r9',
    decision: 'changes_requested',
    note: 'Fix it',
    postedAt: null,
  })
  const posted = 'Your decision is saved, but the note was not posted: the channel is busy'
  let reviewed = review()
  const view = await open({
    channel_review: () => reviewed,
    review_decide: () => {
      reviewed = review({ state: 'changes_requested', decisions: [failed], unposted: 'r9' })
      throw posted
    },
    review_retry_post: () =>
      review({ state: 'changes_requested', decisions: [{ ...failed, postedAt: 5 }] }),
  })
  await press(view, 'Request changes')
  await view.get('textarea').setValue('Fix it')
  await press(view, 'Send request')
  expect(view.get('[role="alert"]').text()).toContain(posted)
  expect(view.get('[data-ui-id="review-history"]').text()).toContain('Fix it')
  await press(view, 'Retry')
  expect(calls('review_retry_post')).toEqual([
    ['review_retry_post', { channelId: 'c1', decisionId: 'r9' }],
  ])
  expect(hasButton(view, 'Retry')).toBe(false)
  expect(view.find('[role="alert"]').exists()).toBe(false)
})

test('Retry is disabled while a turn runs', async () => {
  const failed = decision({ id: 'r9', decision: 'changes_requested', note: 'Fix it' })
  const view = await show(
    review({ state: 'changes_requested', decisions: [failed], unposted: 'r9' }),
    { turnRunning: true },
  )
  expect(button(view, 'Retry').attributes('disabled')).toBeDefined()
})

test('the summary button is labelled "Ask for a summary" and calls reviewRequestSummary', async () => {
  const view = await open({
    channel_review: () => review(),
    review_request_summary: () => undefined,
  })
  await press(view, 'Ask for a summary')
  expect(calls('review_request_summary')).toEqual([['review_request_summary', { channelId: 'c1' }]])
})

test('earlier decisions show in a short history list, newest first, with decision, short commit, and note', async () => {
  const view = await show(
    review({
      state: 'changes_requested',
      decisions: [
        decision({ id: 'r2', decision: 'changes_requested', head: 'e'.repeat(40), note: 'Second' }),
        decision({ id: 'r1', decision: 'approved', head: 'c'.repeat(40) }),
      ],
    }),
  )
  const rows = view.findAll('[data-ui-id="review-history"] li')
  expect(rows).toHaveLength(2)
  expect(rows[0]!.text()).toContain('Changes requested')
  expect(rows[0]!.text()).toContain('eeeeeee')
  expect(rows[0]!.text()).toContain('Second')
  expect(rows[1]!.text()).toContain('Approved')
  expect(rows[1]!.text()).toContain('ccccccc')
})

test('refresh reloads, and a turn ending reloads', async () => {
  const view = await show(review())
  expect(calls('channel_review')).toHaveLength(1)
  await view.get('[aria-label="Refresh review"]').trigger('click')
  await flushPromises()
  expect(calls('channel_review')).toHaveLength(2)
  await view.setProps({ turnRunning: true })
  await flushPromises()
  expect(calls('channel_review')).toHaveLength(2)
  await view.setProps({ turnRunning: false })
  await flushPromises()
  expect(calls('channel_review')).toHaveLength(3)
})

test('emits the state for the tab badge', async () => {
  const view = await show(review({ state: 'approved', decisions: [decision()] }))
  expect(view.emitted('state')?.at(-1)).toEqual(['approved'])
})

test('emits null when the review cannot load', async () => {
  const view = await open({
    channel_review: () => {
      throw 'Direct messages have no merge review.'
    },
  })
  expect(view.emitted('state')?.at(-1)).toEqual([null])
  expect(view.text()).toContain('Direct messages have no merge review.')
})

test('files render through the same tree and diff components as the Changes tab', async () => {
  const view = await show(review())
  expect(
    view.findAll('[data-ui-id="file-diff"]').map((item) => item.attributes('data-file-path')),
  ).toEqual(['src/a.ts', 'b.md'])
  expect(view.text()).toContain('+new src/a.ts')
  expect(view.find('[data-layout="inline"]').exists()).toBe(true)
  await press(view, 'Split')
  expect(view.find('[data-layout="split"]').exists()).toBe(true)
  expect(localStorage.getItem('discard:review-layout')).toBe('split')
})
