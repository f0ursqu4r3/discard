import { computed, nextTick, ref, watch, type Ref, type ShallowRef } from 'vue'
import type { ChangeFileDto } from '../../bridge/types'
import { activeFilePath } from './activeFile'
import { LARGE_DIFF, MANY_FILES, OPEN_FILES } from './largeDiff'

/**
 * The state of a file tree beside a diff pane: the chosen file, the file at the top of the pane,
 * and which sections are open. `files` is in tree order.
 */
export function useFilePane(
  files: Ref<ChangeFileDto[]>,
  fileMode: Ref<'all' | 'selected'>,
  pane: Readonly<ShallowRef<HTMLElement | null>> | Ref<HTMLElement | null>,
) {
  const selectedPath = ref<string | null>(null)
  /** The file at the top of the diff pane in All files mode. */
  const activePath = ref<string | null>(null)
  /** The pane scroll position after a click on a file. The clicked file stays active until the user scrolls. */
  let pinnedTop: number | null = null
  /** The sections that the user opened or closed. The others follow the default. */
  const overrides = ref<Record<string, boolean>>({})

  // The file that the Selected file mode shows: the chosen one, else the first.
  const shown = computed(() => {
    const chosen = files.value.find((file) => file.path === selectedPath.value)
    return chosen ?? files.value[0]
  })
  // Selected file mode marks the shown file. All files mode marks the file at the top of the pane, else the first file.
  const highlighted = computed(() => {
    if (fileMode.value === 'selected') return shown.value?.path ?? null
    const active = files.value.find((file) => file.path === activePath.value)
    return active?.path ?? files.value[0]?.path ?? null
  })
  const visible = computed(() =>
    fileMode.value === 'selected' ? (shown.value ? [shown.value] : []) : files.value,
  )
  const startsClosed = computed(
    () =>
      files.value.length > MANY_FILES ||
      files.value.reduce((sum, file) => sum + file.diff.length, 0) > LARGE_DIFF,
  )
  function isOpen(path: string) {
    const chosen = overrides.value[path]
    if (chosen !== undefined) return chosen
    if (fileMode.value === 'selected' || !startsClosed.value) return true
    return files.value.findIndex((file) => file.path === path) < OPEN_FILES
  }
  function setOpen(path: string, open: boolean) {
    overrides.value = { ...overrides.value, [path]: open }
  }

  async function select(path: string) {
    selectedPath.value = path
    if (fileMode.value !== 'all') return
    setOpen(path, true)
    await nextTick()
    const section = [...(pane.value?.querySelectorAll<HTMLElement>('[data-file-path]') ?? [])].find(
      (element) => element.dataset.filePath === path,
    )
    section?.scrollIntoView?.({ block: 'start' })
    activePath.value = path
    pinnedTop = pane.value?.scrollTop ?? null
  }

  /** Find the file at the top of the pane. A clicked file stays active until the user scrolls. */
  function trackActive() {
    const element = pane.value
    if (!element || fileMode.value !== 'all') return
    if (pinnedTop !== null && Math.abs(element.scrollTop - pinnedTop) < 1) return
    pinnedTop = null
    const found = [...element.querySelectorAll<HTMLElement>('[data-file-path]')].map((section) => ({
      path: section.dataset.filePath ?? '',
      top: section.getBoundingClientRect().top,
    }))
    activePath.value = activeFilePath(element.getBoundingClientRect().top, found)
  }

  watch(fileMode, async (value) => {
    if (value !== 'all') return
    pinnedTop = null
    await nextTick()
    trackActive()
  })

  /** Forget the choice and the open overrides. Call it when the files belong to another source. */
  function reset() {
    overrides.value = {}
    selectedPath.value = null
    activePath.value = null
    pinnedTop = null
  }

  return { highlighted, visible, isOpen, setOpen, select, trackActive, reset }
}
