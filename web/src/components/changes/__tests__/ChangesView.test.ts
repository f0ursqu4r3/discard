import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import type {
  BlockDto,
  ChangeFileDto,
  ChannelChangesDto,
  ChangesScope,
  MessageDto,
} from '../../../bridge/types'
import ChangesView from '../ChangesView.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const file = (path: string, patch: Partial<ChangeFileDto> = {}): ChangeFileDto => ({
  path,
  oldPath: null,
  status: 'M',
  additions: 1,
  deletions: 1,
  binary: false,
  diff: `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1,2 +1,2 @@\n keep\n-old ${path}\n+new ${path}\n`,
  truncated: false,
  ...patch,
})
const result = (
  files: ChangeFileDto[],
  patch: Partial<ChannelChangesDto> = {},
): ChannelChangesDto => ({
  root: '/repo',
  scope: 'uncommitted',
  base: 'abc1234',
  reason: null,
  files,
  truncated: false,
  ...patch,
})
const calls = () => vi.mocked(invoke).mock.calls.filter(([name]) => name === 'channel_changes')
const history: MessageDto[] = [
  {
    id: 'm1',
    role: 'agent',
    continuesGroup: false,
    blocks: [
      {
        type: 'changes',
        root: '/repo',
        state: 'completed',
        commits: [],
        files: [
          { path: 'reported.rs', status: 'M', diff: '+x', truncated: false, preexisting: false },
        ],
        truncated: false,
      },
      { type: 'diff', text: '--- a/p\n+++ b/p\n@@ -1 +1 @@\n-a\n+b' } as BlockDto,
    ],
  },
]
const views: VueWrapper[] = []
async function open(
  load: (scope: ChangesScope) => Promise<ChannelChangesDto> | ChannelChangesDto,
  props: { turnRunning?: boolean; history?: MessageDto[]; branchless?: boolean } = {},
) {
  vi.mocked(invoke).mockImplementation(async (name, args) => {
    if (name !== 'channel_changes') throw new Error(`unexpected ${name}`)
    return load((args as { scope: ChangesScope }).scope)
  })
  const view = mount(ChangesView, {
    attachTo: document.body,
    props: {
      channelId: 'c1',
      history: props.history ?? [],
      turnRunning: props.turnRunning ?? false,
      ...(props.branchless === undefined ? {} : { branchless: props.branchless }),
    },
  })
  views.push(view)
  await flushPromises()
  return view
}
const press = async (view: VueWrapper, label: string) => {
  const button = view.findAll('button').find((item) => item.text() === label)
  if (!button) throw new Error(`no button ${label}`)
  await button.trigger('click')
  await flushPromises()
}
const sections = (view: VueWrapper) =>
  view.findAll('[data-ui-id="file-diff"]').map((item) => item.attributes('data-file-path'))

beforeEach(() => {
  localStorage.clear()
  vi.mocked(invoke).mockReset()
})
afterEach(() => {
  views.splice(0).forEach((view) => view.unmount())
  document.body.innerHTML = ''
})

test('loads the uncommitted changes and shows the tree, the diffs, and the summary', async () => {
  const view = await open(() => result([file('src/a.ts'), file('src/b.ts'), file('top.md')]))
  expect(calls()).toEqual([['channel_changes', { channelId: 'c1', scope: 'uncommitted' }]])
  expect(view.findAll('[data-tree-file]').map((row) => row.attributes('data-path'))).toEqual([
    'src/a.ts',
    'src/b.ts',
    'top.md',
  ])
  expect(view.get('[data-tree-folder]').text()).toBe('src')
  expect(sections(view)).toEqual(['src/a.ts', 'src/b.ts', 'top.md'])
  expect(view.findAll('[data-ui-id="changes-summary"] > span').map((item) => item.text())).toEqual([
    '3 files',
    '+3',
    '−3',
  ])
  expect(view.text()).toContain('+new src/a.ts')
})

test('switches between the inline and the split layout', async () => {
  const view = await open(() => result([file('a.ts')]))
  expect(view.find('[data-layout="inline"]').exists()).toBe(true)
  expect(view.find('[data-layout="split"]').exists()).toBe(false)
  await press(view, 'Split')
  expect(view.find('[data-layout="split"]').exists()).toBe(true)
  // Each side is its own pane. The same row index is the same line pair.
  expect(view.findAll('[data-side="left"]')[1]!.text()).toContain('old a.ts')
  expect(view.findAll('[data-side="right"]')[1]!.text()).toContain('new a.ts')
  await press(view, 'Inline')
  expect(view.find('[data-layout="inline"]').exists()).toBe(true)
})

test('shows every file, or only the selected file', async () => {
  const view = await open(() => result([file('a.ts'), file('b.ts')]))
  expect(sections(view)).toEqual(['a.ts', 'b.ts'])
  await press(view, 'Selected file')
  expect(sections(view)).toEqual(['a.ts'])
  await view.get('[data-path="b.ts"]').trigger('click')
  expect(sections(view)).toEqual(['b.ts'])
  expect(view.get('[data-path="b.ts"]').attributes('aria-current')).toBe('true')
  await press(view, 'All files')
  expect(sections(view)).toEqual(['a.ts', 'b.ts'])
})

test('a click on a file scrolls its section into view in All files mode', async () => {
  const scrolled: (string | undefined)[] = []
  const original = Element.prototype.scrollIntoView
  Element.prototype.scrollIntoView = function (this: HTMLElement) {
    if (this.dataset.filePath !== undefined) scrolled.push(this.dataset.filePath)
  }
  try {
    const view = await open(() => result([file('a.ts'), file('b.ts')]))
    await view.get('[data-path="b.ts"]').trigger('click')
    expect(scrolled).toEqual(['b.ts'])
    expect(view.get('[data-path="b.ts"]').attributes('aria-current')).toBe('true')
    expect(view.get('[data-file-path="b.ts"]').attributes('data-selected')).toBe('true')
  } finally {
    Element.prototype.scrollIntoView = original
  }
})

test('the first file is marked in the tree in All files mode before any click', async () => {
  const view = await open(() => result([file('a.ts'), file('b.ts')]))
  expect(view.get('[data-path="a.ts"]').attributes('aria-current')).toBe('true')
  expect(view.get('[data-path="b.ts"]').attributes('aria-current')).toBeUndefined()
})

test('a scroll marks the file at the top of the pane in the tree', async () => {
  const view = await open(() => result([file('a.ts'), file('b.ts')]))
  const tops: Record<string, number> = { 'a.ts': -200, 'b.ts': 5 }
  const original = Element.prototype.getBoundingClientRect
  Element.prototype.getBoundingClientRect = function (this: HTMLElement) {
    const top = this.dataset.filePath ? (tops[this.dataset.filePath] ?? 0) : 0
    return { top } as DOMRect
  }
  try {
    await view.get('[data-ui-id="changes-diffs"]').trigger('scroll')
    expect(view.get('[data-path="b.ts"]').attributes('aria-current')).toBe('true')
    expect(view.get('[data-path="a.ts"]').attributes('aria-current')).toBeUndefined()
  } finally {
    Element.prototype.getBoundingClientRect = original
  }
})

test('the tree filter narrows the tree and keeps the diffs', async () => {
  const view = await open(() => result([file('src/a.ts'), file('docs/b.md')]))
  await view.get('input[aria-label="Filter files"]').setValue('docs')
  expect(view.findAll('[data-tree-file]').map((row) => row.attributes('data-path'))).toEqual([
    'docs/b.md',
  ])
  expect(sections(view)).toEqual(['docs/b.md', 'src/a.ts'])
  await view.get('input[aria-label="Filter files"]').setValue('zzz')
  expect(view.text()).toContain('No file matches the filter.')
})

test('a section closes and opens again, and a closed section draws no lines', async () => {
  const view = await open(() => result([file('a.ts')]))
  expect(view.find('[data-file-body]').exists()).toBe(true)
  await view.get('[data-file-toggle]').trigger('click')
  expect(view.find('[data-file-body]').exists()).toBe(false)
  expect(view.get('[data-file-toggle]').attributes('aria-expanded')).toBe('false')
  await view.get('[data-file-toggle]').trigger('click')
  expect(view.find('[data-file-body]').exists()).toBe(true)
})

test('a large change starts with only the first twenty sections open', async () => {
  const many = Array.from({ length: 60 }, (_, i) => file(`f${String(i).padStart(2, '0')}.ts`))
  const view = await open(() => result(many))
  expect(view.findAll('[data-ui-id="file-diff"]')).toHaveLength(60)
  expect(view.findAll('[data-file-body]')).toHaveLength(20)
  // A click on a closed file opens it.
  await view.get('[data-path="f30.ts"]').trigger('click')
  expect(view.find('[data-file-path="f30.ts"] [data-file-body]').exists()).toBe(true)
})

test('binary, truncated, renamed, and empty files say so', async () => {
  const view = await open(() =>
    result([
      file('image.png', { binary: true, diff: '', additions: 0, deletions: 0 }),
      file('big.txt', { truncated: true }),
      file('new-name.ts', {
        status: 'R',
        oldPath: 'old-name.ts',
        diff: 'diff --git a/x b/y\nrename from old-name.ts\n',
      }),
    ]),
  )
  const text = (path: string) => view.get(`[data-file-path="${path}"]`).text()
  expect(text('image.png')).toContain('Binary file')
  expect(text('big.txt')).toContain('Some changes are not shown.')
  expect(text('new-name.ts')).toContain('old-name.ts → new-name.ts')
  expect(text('new-name.ts')).toContain('No line changes to show.')
})

test('the branch scope shows the base commit', async () => {
  const view = await open((scope) =>
    result([file('a.ts')], { scope, base: scope === 'branch' ? 'def5678' : 'abc1234' }),
  )
  await press(view, 'Branch')
  expect(calls().at(-1)).toEqual(['channel_changes', { channelId: 'c1', scope: 'branch' }])
  expect(view.get('[data-ui-id="changes-base"]').text()).toBe('base def5678')
})

test('an unavailable branch scope shows the reason from the backend', async () => {
  const reason = 'This is the default branch, main. Use Uncommitted to see its changes.'
  const view = await open((scope) =>
    scope === 'branch' ? result([], { scope, base: null, reason }) : result([file('a.ts')]),
  )
  await press(view, 'Branch')
  expect(view.get('[data-ui-id="changes-unavailable"]').text()).toContain(reason)
  expect(view.text()).not.toContain('Soon')
  // The scope stays selectable.
  expect(
    view
      .findAll('button')
      .find((b) => b.text() === 'Branch')!
      .attributes('aria-pressed'),
  ).toBe('true')
})

test('a clean tree says that there are no changes and what it compares', async () => {
  const view = await open(() => result([]))
  expect(view.get('[data-ui-id="changes-empty"]').text()).toContain('No changes')
  expect(view.text()).toContain('The working tree matches the last commit.')
})

test('an error shows the message and Retry loads again', async () => {
  let fail = true
  const view = await open(() => {
    if (fail) throw 'This folder is not a Git repository.'
    return result([file('a.ts')])
  })
  expect(view.get('[data-ui-id="changes-error"]').text()).toContain(
    'This folder is not a Git repository.',
  )
  fail = false
  await press(view, 'Retry')
  expect(calls()).toHaveLength(2)
  expect(view.find('[data-ui-id="changes-error"]').exists()).toBe(false)
  expect(sections(view)).toEqual(['a.ts'])
})

test('the refresh button loads again', async () => {
  const view = await open(() => result([file('a.ts')]))
  await view.get('button[aria-label="Refresh changes"]').trigger('click')
  await flushPromises()
  expect(calls()).toHaveLength(2)
})

test('shows a loading state until the first answer', async () => {
  let release: (value: ChannelChangesDto) => void = () => {}
  const view = await open(() => new Promise<ChannelChangesDto>((resolve) => (release = resolve)))
  expect(view.get('[data-ui-id="changes-loading"]').text()).toContain('Loading changes')
  release(result([file('a.ts')]))
  await flushPromises()
  expect(view.find('[data-ui-id="changes-loading"]').exists()).toBe(false)
})

test('loads again when a turn ends, and not when it starts', async () => {
  const view = await open(() => result([file('a.ts')]), { turnRunning: true })
  expect(calls()).toHaveLength(1)
  await view.setProps({ turnRunning: false })
  await flushPromises()
  expect(calls()).toHaveLength(2)
  await view.setProps({ turnRunning: true })
  await flushPromises()
  expect(calls()).toHaveLength(2)
})

test('an older answer for the same scope does not replace a newer one', async () => {
  const pending: ((value: ChannelChangesDto) => void)[] = []
  const view = await open(
    () => new Promise<ChannelChangesDto>((resolve) => pending.push(resolve)),
    {
      turnRunning: true,
    },
  )
  await view.setProps({ turnRunning: false })
  expect(pending).toHaveLength(2)
  pending[1]!(result([file('new.ts')]))
  await flushPromises()
  pending[0]!(result([file('old.ts')]))
  await flushPromises()
  expect(sections(view)).toEqual(['new.ts'])
})

test('the newest answer wins when the older answer arrives last', async () => {
  const pending: ((value: ChannelChangesDto) => void)[] = []
  const view = await open((scope) =>
    scope === 'uncommitted'
      ? new Promise<ChannelChangesDto>((resolve) => pending.push(resolve))
      : result([file('branch.ts')], { scope }),
  )
  expect(pending).toHaveLength(1)
  await press(view, 'Branch')
  expect(sections(view)).toEqual(['branch.ts'])
  pending[0]!(result([file('late.ts')]))
  await flushPromises()
  expect(sections(view)).toEqual(['branch.ts'])
})

test('Responses shows the cards and the diffs that agents posted, without a tree', async () => {
  const view = await open(() => result([file('a.ts')]), { history })
  await press(view, 'Responses')
  expect(view.find('[data-ui-id="changes-card"]').exists()).toBe(true)
  expect(view.text()).toContain('reported.rs')
  expect(view.find('[data-diff]').exists()).toBe(true)
  expect(view.find('[data-ui-id="changes-tree"]').exists()).toBe(false)
  expect(view.find('[data-ui-id="changes-summary"]').exists()).toBe(false)
  expect(calls()).toHaveLength(1)
})

test('Responses without entries keeps the old empty state', async () => {
  const view = await open(() => result([]))
  await press(view, 'Responses')
  expect(view.text()).toContain('No changes to display')
})

test('remembers the scope, the layout, and the files mode', async () => {
  const view = await open(() => result([file('a.ts')]))
  await press(view, 'Split')
  await press(view, 'Selected file')
  await press(view, 'Branch')
  expect(localStorage.getItem('discard:changes-layout')).toBe('split')
  expect(localStorage.getItem('discard:changes-files')).toBe('selected')
  expect(localStorage.getItem('discard:changes-scope')).toBe('branch')
  view.unmount()
  const again = await open(() => result([file('a.ts')]))
  expect(calls().at(-1)).toEqual(['channel_changes', { channelId: 'c1', scope: 'branch' }])
  expect(again.find('[data-layout="split"]').exists()).toBe(true)
})

test('a saved value that is not a choice falls back to the default', async () => {
  localStorage.setItem('discard:changes-scope', 'everything')
  const view = await open(() => result([file('a.ts')]))
  expect(calls()[0]![1]).toMatchObject({ scope: 'uncommitted' })
  expect(view.find('[data-layout="inline"]').exists()).toBe(true)
})

test('loads again for another channel', async () => {
  const view = await open(() => result([file('a.ts')]))
  await view.setProps({ channelId: 'c2' })
  await flushPromises()
  expect(calls().at(-1)).toEqual(['channel_changes', { channelId: 'c2', scope: 'uncommitted' }])
})

const labels = (view: VueWrapper) =>
  view
    .get('[role="group"][aria-label="Compare"]')
    .findAll('button')
    .map((item) => item.text())

test('a branchless channel offers Responses and Main checkout, and opens on Responses', async () => {
  const view = await open(() => result([file('a.ts')]), { branchless: true, history })
  expect(labels(view)).toEqual(['Responses', 'Main checkout'])
  expect(calls()).toHaveLength(0)
  expect(view.find('[data-ui-id="changes-responses"]').exists()).toBe(true)
  await press(view, 'Main checkout')
  expect(calls()).toEqual([['channel_changes', { channelId: 'c1', scope: 'uncommitted' }]])
})

test('a branchless channel saves the scope under its own key', async () => {
  localStorage.setItem('discard:changes-scope', 'branch')
  const view = await open(() => result([file('a.ts')]), { branchless: true })
  expect(view.find('[data-ui-id="changes-responses"]').exists()).toBe(true)
  await press(view, 'Main checkout')
  expect(localStorage.getItem('discard:changes-text-scope')).toBe('uncommitted')
  expect(localStorage.getItem('discard:changes-scope')).toBe('branch')
  view.unmount()
  await open(() => result([file('a.ts')]), { branchless: true })
  expect(calls().at(-1)).toEqual(['channel_changes', { channelId: 'c1', scope: 'uncommitted' }])
})

test('a branchless channel ignores a saved branch scope in its own key', async () => {
  localStorage.setItem('discard:changes-text-scope', 'branch')
  const view = await open(() => result([file('a.ts')]), { branchless: true })
  expect(view.find('[data-ui-id="changes-responses"]').exists()).toBe(true)
  expect(calls()).toHaveLength(0)
})

test('a clean main checkout says what it compares', async () => {
  const view = await open(() => result([]), { branchless: true })
  await press(view, 'Main checkout')
  expect(view.text()).toContain('The main checkout matches the last commit.')
})

test('a change of the branchless prop reads the scope from the matching key', async () => {
  localStorage.setItem('discard:changes-scope', 'branch')
  localStorage.setItem('discard:changes-text-scope', 'uncommitted')
  const view = await open((scope) => result([file('a.ts')], { scope }), { branchless: false })
  expect(calls().at(-1)).toEqual(['channel_changes', { channelId: 'c1', scope: 'branch' }])
  await view.setProps({ branchless: true })
  await flushPromises()
  expect(labels(view)).toEqual(['Responses', 'Main checkout'])
  expect(calls().at(-1)).toEqual(['channel_changes', { channelId: 'c1', scope: 'uncommitted' }])
  expect(localStorage.getItem('discard:changes-scope')).toBe('branch')
  await view.setProps({ branchless: false })
  await flushPromises()
  expect(labels(view)).toEqual(['Uncommitted', 'Branch', 'Responses'])
  expect(calls().at(-1)).toEqual(['channel_changes', { channelId: 'c1', scope: 'branch' }])
  expect(localStorage.getItem('discard:changes-text-scope')).toBe('uncommitted')
})

test('a channel that is not branchless keeps the three scopes and the old default', async () => {
  const view = await open(() => result([]), { branchless: false })
  expect(labels(view)).toEqual(['Uncommitted', 'Branch', 'Responses'])
  expect(calls()[0]![1]).toMatchObject({ scope: 'uncommitted' })
  expect(view.text()).toContain('The working tree matches the last commit.')
})
