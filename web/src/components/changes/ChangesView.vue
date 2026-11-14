<script setup lang="ts">
import { GitPullRequest, RefreshCw } from '@lucide/vue'
import { computed, nextTick, onMounted, ref, useTemplateRef, watch } from 'vue'
import { channelChanges } from '../../bridge/api'
import type { ChannelChangesDto, MessageDto } from '../../bridge/types'
import ChangesCard from '../ChangesCard.vue'
import DiffBlock from '../DiffBlock.vue'
import { changeEntries } from '../messages'
import { Button } from '../ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '../ui/empty'
import ChangesTree from './ChangesTree.vue'
import FileDiff from './FileDiff.vue'
import { preferenceKeys, readChoice, saveChoice } from './preferences'
import SegmentedControl from './SegmentedControl.vue'
import { treeOrder } from './tree'

const props = defineProps<{ channelId: string; history: MessageDto[]; turnRunning: boolean }>()

/** With more files than this, or more diff text than `LARGE_DIFF`, sections after the first `OPEN_FILES` start closed. */
const MANY_FILES = 50
const LARGE_DIFF = 256 * 1024
const OPEN_FILES = 20

const scopes = ['uncommitted', 'branch', 'responses'] as const
const layouts = ['split', 'inline'] as const
const fileModes = ['all', 'selected'] as const
const scope = ref(readChoice(preferenceKeys.scope, scopes, 'uncommitted'))
const layout = ref(readChoice(preferenceKeys.layout, layouts, 'inline'))
const fileMode = ref(readChoice(preferenceKeys.files, fileModes, 'all'))
watch(scope, (value) => saveChoice(preferenceKeys.scope, value))
watch(layout, (value) => saveChoice(preferenceKeys.layout, value))
watch(fileMode, (value) => saveChoice(preferenceKeys.files, value))

const data = ref<ChannelChangesDto | null>(null)
const selectedPath = ref<string | null>(null)
/** The sections that the user opened or closed. The others follow the default. */
const overrides = ref<Record<string, boolean>>({})
const loading = ref(false)
const error = ref<string | null>(null)
let latest = 0

function messageOf(failure: unknown) {
  if (typeof failure === 'string') return failure
  return failure instanceof Error ? failure.message : String(failure)
}

/** Load the changes of the scope. Only the newest request may set the result. */
async function load() {
  const requested = scope.value
  if (requested === 'responses') return
  const request = ++latest
  loading.value = true
  error.value = null
  try {
    const result = await channelChanges(props.channelId, requested)
    if (request !== latest) return
    data.value = result
  } catch (failure) {
    if (request !== latest) return
    data.value = null
    error.value = messageOf(failure)
  } finally {
    if (request === latest) loading.value = false
  }
}

onMounted(load)
watch([scope, () => props.channelId], () => {
  // The old result belongs to another scope or channel. A request in flight is now stale.
  latest++
  data.value = null
  error.value = null
  loading.value = false
  overrides.value = {}
  selectedPath.value = null
  void load()
})
watch(
  () => props.turnRunning,
  (running, was) => {
    if (was && !running) void load()
  },
)

const files = computed(() => treeOrder(data.value?.files ?? []))
const totals = computed(() => ({
  additions: files.value.reduce((sum, file) => sum + file.additions, 0),
  deletions: files.value.reduce((sum, file) => sum + file.deletions, 0),
}))
const entries = computed(() => changeEntries(props.history))

// The file that the Selected file mode shows: the chosen one, else the first.
const shown = computed(() => {
  const chosen = files.value.find((file) => file.path === selectedPath.value)
  return chosen ?? files.value[0]
})
// All files mode marks only a file that the user chose. Selected file mode always has a file.
const highlighted = computed(() =>
  fileMode.value === 'selected' ? (shown.value?.path ?? null) : selectedPath.value,
)
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

const pane = useTemplateRef<HTMLElement>('pane')
async function select(path: string) {
  selectedPath.value = path
  if (fileMode.value !== 'all') return
  overrides.value = { ...overrides.value, [path]: true }
  await nextTick()
  const section = [...(pane.value?.querySelectorAll<HTMLElement>('[data-file-path]') ?? [])].find(
    (element) => element.dataset.filePath === path,
  )
  section?.scrollIntoView?.({ block: 'start' })
}

const compared = computed(() => {
  if (scope.value === 'uncommitted') return 'The working tree matches the last commit.'
  return `The working tree matches the base commit${data.value?.base ? ` (${data.value.base})` : ''}.`
})
const scopeOptions = [
  { value: 'uncommitted', label: 'Uncommitted' },
  { value: 'branch', label: 'Branch' },
  { value: 'responses', label: 'Responses' },
] as const
const layoutOptions = [
  { value: 'split', label: 'Split' },
  { value: 'inline', label: 'Inline' },
] as const
const fileOptions = [
  { value: 'all', label: 'All files' },
  { value: 'selected', label: 'Selected file' },
] as const
</script>

<template>
  <div data-ui-id="changes-view" class="flex min-h-0 flex-1 flex-col">
    <div
      data-ui-id="changes-toolbar"
      class="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-border/60 px-3 py-2 sm:px-5"
    >
      <SegmentedControl v-model="scope" label="Compare" :options="scopeOptions" />
      <template v-if="scope !== 'responses'">
        <SegmentedControl v-model="layout" label="Layout" :options="layoutOptions" />
        <SegmentedControl v-model="fileMode" label="Files" :options="fileOptions" />
        <span
          v-if="data && !data.reason"
          data-ui-id="changes-summary"
          class="flex items-center gap-2 font-mono text-xs text-muted-foreground"
        >
          <span>{{ files.length }} {{ files.length === 1 ? 'file' : 'files' }}</span>
          <span class="text-success">+{{ totals.additions }}</span>
          <span class="text-destructive">−{{ totals.deletions }}</span>
          <span v-if="scope === 'branch' && data.base" data-ui-id="changes-base">
            base {{ data.base }}
          </span>
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          class="ml-auto"
          aria-label="Refresh changes"
          title="Refresh changes"
          :disabled="loading"
          @click="load"
        >
          <RefreshCw
            aria-hidden="true"
            class="size-3.5"
            :class="loading ? 'animate-spin motion-reduce:animate-none' : ''"
          />
        </Button>
      </template>
    </div>

    <!-- Responses: what the agents reported in this conversation. -->
    <div
      v-if="scope === 'responses'"
      data-ui-id="changes-responses"
      class="min-h-0 flex-1 overflow-auto overscroll-contain p-5"
    >
      <div v-if="entries.length" class="flex flex-col gap-4">
        <template v-for="(block, index) in entries" :key="index">
          <ChangesCard v-if="block.type === 'changes'" class="my-0!" :block="block" />
          <DiffBlock v-else :text="block.text" />
        </template>
      </div>
      <Empty v-else>
        <EmptyHeader>
          <EmptyMedia variant="icon"><GitPullRequest /></EmptyMedia>
          <EmptyTitle>No changes to display</EmptyTitle>
          <EmptyDescription>
            File changes and diffs from this conversation appear here.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    </div>

    <div
      v-else-if="error"
      data-ui-id="changes-error"
      role="alert"
      class="flex min-h-0 flex-1 items-center justify-center p-5"
    >
      <Empty>
        <EmptyHeader>
          <EmptyTitle>Changes are not available</EmptyTitle>
          <EmptyDescription>{{ error }}</EmptyDescription>
        </EmptyHeader>
        <Button type="button" variant="outline" size="sm" @click="load">Retry</Button>
      </Empty>
    </div>

    <div
      v-else-if="!data"
      data-ui-id="changes-loading"
      role="status"
      class="flex min-h-0 flex-1 items-center justify-center p-5 text-sm text-muted-foreground"
    >
      Loading changes…
    </div>

    <div
      v-else-if="data.reason"
      data-ui-id="changes-unavailable"
      class="flex min-h-0 flex-1 items-center justify-center p-5"
    >
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon"><GitPullRequest /></EmptyMedia>
          <EmptyTitle>No branch comparison</EmptyTitle>
          <EmptyDescription>{{ data.reason }}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    </div>

    <div
      v-else-if="!files.length"
      data-ui-id="changes-empty"
      class="flex min-h-0 flex-1 items-center justify-center p-5"
    >
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon"><GitPullRequest /></EmptyMedia>
          <EmptyTitle>No changes</EmptyTitle>
          <EmptyDescription>{{ compared }}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    </div>

    <div v-else class="flex min-h-0 flex-1">
      <aside data-ui-id="changes-files" class="w-[260px] shrink-0 border-r border-border/60">
        <ChangesTree :files="files" :selected="highlighted" @select="select" />
      </aside>
      <div
        ref="pane"
        data-ui-id="changes-diffs"
        class="min-w-0 flex-1 overflow-auto overscroll-contain p-3"
      >
        <p v-if="data.truncated" class="mb-3 text-xs text-muted-foreground">
          Some changes are not shown.
        </p>
        <div class="flex flex-col gap-3">
          <FileDiff
            v-for="file in visible"
            :key="file.path"
            :file="file"
            :layout="layout"
            :open="isOpen(file.path)"
            :selected="highlighted === file.path"
            @update:open="overrides = { ...overrides, [file.path]: $event }"
          />
        </div>
      </div>
    </div>
  </div>
</template>
