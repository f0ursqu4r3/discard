import { mount } from '@vue/test-utils'
import { defineComponent, h, nextTick, ref } from 'vue'
import { expect, test } from 'vite-plus/test'
import type { ChangeFileDto } from '../../../bridge/types'
import { LARGE_DIFF, OPEN_FILES } from '../largeDiff'
import { useFilePane } from '../useFilePane'

const file = (path: string, diff = ''): ChangeFileDto => ({
  path,
  oldPath: null,
  status: 'M',
  additions: 0,
  deletions: 0,
  binary: false,
  diff,
  truncated: false,
})

function setup(files: ChangeFileDto[], mode: 'all' | 'selected' = 'all') {
  const list = ref(files)
  const fileMode = ref(mode)
  let pane!: ReturnType<typeof useFilePane>
  mount(
    defineComponent({
      setup() {
        pane = useFilePane(list, fileMode, ref(null))
        return () => h('div')
      },
    }),
  )
  return { pane, list, fileMode }
}

test('marks the first file, and shows all files in All files mode', () => {
  const { pane } = setup([file('a'), file('b')])
  expect(pane.highlighted.value).toBe('a')
  expect(pane.visible.value.map((item) => item.path)).toEqual(['a', 'b'])
})

test('Selected file mode shows the chosen file only', async () => {
  const { pane, fileMode } = setup([file('a'), file('b')], 'selected')
  expect(pane.visible.value.map((item) => item.path)).toEqual(['a'])
  await pane.select('b')
  expect(pane.highlighted.value).toBe('b')
  expect(pane.visible.value.map((item) => item.path)).toEqual(['b'])
  fileMode.value = 'all'
  await nextTick()
  expect(pane.visible.value).toHaveLength(2)
})

test('many files start closed after the first OPEN_FILES, and a click opens one', async () => {
  const many = Array.from({ length: 51 }, (_, index) => file(`f${index}`))
  const { pane } = setup(many)
  expect(pane.isOpen('f0')).toBe(true)
  expect(pane.isOpen(`f${OPEN_FILES}`)).toBe(false)
  await pane.select(`f${OPEN_FILES}`)
  expect(pane.isOpen(`f${OPEN_FILES}`)).toBe(true)
})

test('a large diff starts closed after the first OPEN_FILES', () => {
  const files = Array.from({ length: 22 }, (_, index) =>
    file(`f${index}`, index === 0 ? 'x'.repeat(LARGE_DIFF + 1) : ''),
  )
  const { pane } = setup(files)
  expect(pane.isOpen('f19')).toBe(true)
  expect(pane.isOpen('f20')).toBe(false)
})

test('an override beats the default, and reset clears the choice and the overrides', async () => {
  const { pane } = setup(Array.from({ length: 51 }, (_, index) => file(`f${index}`)))
  pane.setOpen('f0', false)
  expect(pane.isOpen('f0')).toBe(false)
  await pane.select('f5')
  pane.reset()
  expect(pane.isOpen('f0')).toBe(true)
  expect(pane.highlighted.value).toBe('f0')
})
