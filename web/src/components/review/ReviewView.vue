<script setup lang="ts">
import { GitPullRequest, RefreshCw } from '@lucide/vue'
import { computed, onMounted, ref, useTemplateRef, watch } from 'vue'
import {
  channelReview,
  reviewDecide,
  reviewRequestSummary,
  reviewRetryPost,
} from '../../bridge/api'
import type { ReviewDecisionDto, ReviewDto, ReviewStateDto } from '../../bridge/types'
import ChangesTree from '../changes/ChangesTree.vue'
import FileDiff from '../changes/FileDiff.vue'
import { preferenceKeys, readChoice, saveChoice } from '../changes/preferences'
import SegmentedControl from '../changes/SegmentedControl.vue'
import { treeOrder } from '../changes/tree'
import { useFilePane } from '../changes/useFilePane'
import { Badge } from '../ui/badge'
import { Button } from '../ui/button'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '../ui/empty'
import ReviewActions from './ReviewActions.vue'
import ReviewCommits from './ReviewCommits.vue'

const props = defineProps<{ channelId: string; turnRunning: boolean }>()
const emit = defineEmits<{ state: [state: ReviewStateDto | null] }>()

const ARCHIVED = 'This channel is archived.'
const MERGED = 'This branch is already merged.'
/** The backend starts a rejection with this text when the decision is saved and the note is not posted. */
const SAVED_NOT_POSTED = 'Your decision is saved'

const layouts = ['split', 'inline'] as const
const fileModes = ['all', 'selected'] as const
const layout = ref(readChoice(preferenceKeys.reviewLayout, layouts, 'inline'))
const fileMode = ref(readChoice(preferenceKeys.reviewFiles, fileModes, 'all'))
watch(layout, (value) => saveChoice(preferenceKeys.reviewLayout, value))
watch(fileMode, (value) => saveChoice(preferenceKeys.reviewFiles, value))
const layoutOptions = [
  { value: 'split', label: 'Split' },
  { value: 'inline', label: 'Inline' },
] as const
const fileOptions = [
  { value: 'all', label: 'All files' },
  { value: 'selected', label: 'Selected file' },
] as const

const data = ref<ReviewDto | null>(null)
const loading = ref(false)
const pending = ref(false)
const error = ref<string | null>(null)
let latest = 0

function messageOf(failure: unknown) {
  if (typeof failure === 'string') return failure
  return failure instanceof Error ? failure.message : String(failure)
}
const short = (id: string) => id.slice(0, 7)

/** Load the review. Only the newest request may set the result. A failed reload keeps the old result. */
async function load() {
  const request = ++latest
  loading.value = true
  try {
    const result = await channelReview(props.channelId)
    if (request !== latest) return
    data.value = result
    error.value = null
  } catch (failure) {
    if (request !== latest) return
    error.value = messageOf(failure)
  } finally {
    if (request === latest) loading.value = false
  }
}
function refresh() {
  error.value = null
  void load()
}

onMounted(load)
watch(
  () => props.channelId,
  () => {
    latest++
    data.value = null
    error.value = null
    loading.value = false
    pending.value = false
    reset()
    actions.value?.close()
    void load()
  },
)
watch(
  () => props.turnRunning,
  (running, was) => {
    if (was && !running) void load()
  },
)
watch(
  () => data.value?.state ?? null,
  (state) => emit('state', state),
  { immediate: true },
)

const snapshot = computed(() => data.value?.snapshot ?? null)
const state = computed(() => data.value?.state ?? null)
const unavailable = computed(() => snapshot.value?.reason ?? null)
const idle = computed(
  () => state.value === 'merged' || state.value === 'nothing_to_review' || !!unavailable.value,
)
/** Request changes, Retry, and the summary need a free channel. */
const blockedReason = computed(
  () =>
    data.value?.blocked ??
    (props.turnRunning ? 'A turn is running in this channel. Wait for it to end.' : null),
)
const approveOff = computed(() => data.value?.blocked === ARCHIVED)

const stateLabel = computed(() => {
  const current = data.value
  if (!current) return null
  const newest = current.decisions[0]
  switch (current.state) {
    case 'approved':
      return 'Approved'
    case 'approval_out_of_date':
      return `Approval out of date: approved ${short(newest?.head ?? '')}, the branch is now ${
        current.snapshot.headShort ?? ''
      }`
    case 'changes_requested':
      return 'Changes requested'
    case 'changes_requested_new_commits':
      return 'Changes requested · new commits since'
    case 'merged':
      return MERGED
    case 'nothing_to_review':
      return `Nothing to review: no commits beyond ${current.snapshot.target ?? 'the target branch'}`
    default:
      return null
  }
})

const files = computed(() => treeOrder(snapshot.value?.files ?? []))
const pane = useTemplateRef<HTMLElement>('pane')
const { highlighted, visible, isOpen, setOpen, select, trackActive, reset } = useFilePane(
  files,
  fileMode,
  pane,
)

const actions = useTemplateRef<InstanceType<typeof ReviewActions>>('actions')

/** Run an action. After a rejection, reload so the rows and the retry show. The error stays on screen. */
async function act(run: () => Promise<ReviewDto | void>, reloadOnFailure: boolean) {
  if (pending.value) return
  const channel = props.channelId
  // The channel switched while the call ran. The result belongs to the old channel.
  const stale = () => props.channelId !== channel
  pending.value = true
  error.value = null
  try {
    const result = await run()
    if (stale()) return false
    if (result) data.value = result
    return true
  } catch (failure) {
    if (stale()) return false
    const message = messageOf(failure)
    if (reloadOnFailure) await load()
    if (stale()) return false
    error.value = message
    if (message.startsWith(SAVED_NOT_POSTED)) actions.value?.close()
    return false
  } finally {
    if (!stale()) pending.value = false
  }
}

function decide(kind: 'approved' | 'changes_requested', note: string | null) {
  const head = snapshot.value?.head
  if (!head) return
  void act(() => reviewDecide(props.channelId, kind, head, note), true).then((ok) => {
    if (ok && kind === 'changes_requested') actions.value?.close()
  })
}
function retry(row: ReviewDecisionDto) {
  void act(() => reviewRetryPost(props.channelId, row.id), true)
}
function summary() {
  void act(() => reviewRequestSummary(props.channelId), false)
}

const decisionLabel = (row: ReviewDecisionDto) =>
  row.decision === 'approved' ? 'Approved' : 'Changes requested'
const date = (seconds: number) => new Date(seconds * 1000).toLocaleString()
</script>

<template>
  <div data-ui-id="review-view" class="flex min-h-0 flex-1 flex-col">
    <div
      v-if="!data && error"
      data-ui-id="review-error"
      role="alert"
      class="flex min-h-0 flex-1 items-center justify-center p-5"
    >
      <Empty>
        <EmptyHeader>
          <EmptyTitle>Merge review is not available</EmptyTitle>
          <EmptyDescription>{{ error }}</EmptyDescription>
        </EmptyHeader>
        <Button type="button" variant="outline" size="sm" @click="refresh">Retry</Button>
      </Empty>
    </div>

    <div
      v-else-if="!data || !snapshot"
      data-ui-id="review-loading"
      role="status"
      class="flex min-h-0 flex-1 items-center justify-center p-5 text-sm text-muted-foreground"
    >
      Loading review…
    </div>

    <div
      v-else-if="unavailable"
      data-ui-id="review-unavailable"
      class="flex min-h-0 flex-1 items-center justify-center p-5"
    >
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon"><GitPullRequest /></EmptyMedia>
          <EmptyTitle>No merge review</EmptyTitle>
          <EmptyDescription>{{ unavailable }}</EmptyDescription>
        </EmptyHeader>
        <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
        <Button type="button" variant="outline" size="sm" :disabled="loading" @click="refresh">
          Refresh
        </Button>
      </Empty>
    </div>

    <template v-else>
      <header
        data-ui-id="review-header"
        class="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-border/60 px-3 py-2 sm:px-5"
      >
        <span class="font-mono text-sm">{{ snapshot.source }} → {{ snapshot.target }}</span>
        <span class="font-mono text-xs text-muted-foreground">
          {{ snapshot.ahead }} ahead · {{ snapshot.behind }} behind
        </span>
        <span v-if="snapshot.headShort" class="font-mono text-xs text-muted-foreground">
          head {{ snapshot.headShort }}
        </span>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          class="ml-auto"
          aria-label="Refresh review"
          title="Refresh review"
          :disabled="loading"
          @click="refresh"
        >
          <RefreshCw
            aria-hidden="true"
            class="size-3.5"
            :class="loading ? 'animate-spin motion-reduce:animate-none' : ''"
          />
        </Button>
      </header>

      <div class="flex min-h-0 flex-1 flex-col overflow-auto">
        <section
          data-ui-id="review-state"
          class="flex shrink-0 flex-col gap-2 border-b border-border/60 px-3 py-3 sm:px-5"
        >
          <p v-if="stateLabel" class="flex flex-wrap items-center gap-2 text-sm font-medium">
            <Badge v-if="state === 'merged'" variant="secondary">Merged</Badge>
            <span>{{ stateLabel }}</span>
          </p>
          <template v-if="!idle">
            <ReviewActions
              ref="actions"
              :approve-off="approveOff"
              :busy="!!blockedReason"
              :pending="pending"
              @approve="decide('approved', null)"
              @request="decide('changes_requested', $event)"
              @summary="summary"
            />
            <p
              v-if="blockedReason"
              data-ui-id="review-blocked"
              class="text-xs text-muted-foreground"
            >
              {{ blockedReason }}
            </p>
          </template>
          <p v-if="error" role="alert" class="text-sm text-destructive">{{ error }}</p>
        </section>

        <p
          v-if="snapshot.dirty > 0"
          data-ui-id="review-dirty"
          class="shrink-0 border-b border-border/60 px-3 py-2 text-xs text-muted-foreground sm:px-5"
        >
          {{ snapshot.dirty }} uncommitted {{ snapshot.dirty === 1 ? 'file is' : 'files are' }}
          not reviewed. Only committed work is reviewed.
        </p>

        <section
          v-if="data.decisions.length"
          data-ui-id="review-history"
          class="shrink-0 border-b border-border/60 px-3 py-3 sm:px-5"
        >
          <h3 class="mb-2 text-xs font-medium text-muted-foreground">Decisions</h3>
          <ul class="flex flex-col gap-2 text-sm">
            <li v-for="row in data.decisions" :key="row.id" class="flex flex-col gap-0.5">
              <div class="flex flex-wrap items-baseline gap-2">
                <span>{{ decisionLabel(row) }}</span>
                <span class="font-mono text-xs text-muted-foreground">{{ short(row.head) }}</span>
                <span class="text-xs text-muted-foreground">{{ date(row.decidedAt) }}</span>
                <Button
                  v-if="row.id === data.unposted"
                  type="button"
                  variant="outline"
                  size="xs"
                  :disabled="!!blockedReason || pending"
                  @click="retry(row)"
                >
                  Retry
                </Button>
              </div>
              <p v-if="row.note" class="whitespace-pre-wrap text-muted-foreground">
                {{ row.note }}
              </p>
              <p v-if="row.id === data.unposted" class="text-xs text-destructive">
                The note was not posted to the conversation.
              </p>
            </li>
          </ul>
        </section>

        <template v-if="state !== 'nothing_to_review'">
          <ReviewCommits :commits="snapshot.commits" />

          <div
            data-ui-id="review-toolbar"
            class="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-y border-border/60 px-3 py-2 sm:px-5"
          >
            <SegmentedControl v-model="layout" label="Layout" :options="layoutOptions" />
            <SegmentedControl v-model="fileMode" label="Files" :options="fileOptions" />
            <span class="font-mono text-xs text-muted-foreground">
              {{ files.length }} {{ files.length === 1 ? 'file' : 'files' }}
            </span>
          </div>

          <div v-if="files.length" class="flex min-h-96 flex-1">
            <aside data-ui-id="review-files" class="w-[260px] shrink-0 border-r border-border/60">
              <ChangesTree :files="files" :selected="highlighted" @select="select" />
            </aside>
            <div
              ref="pane"
              data-ui-id="review-diffs"
              class="min-w-0 flex-1 overflow-auto overscroll-contain px-3"
              @scroll.passive="trackActive"
            >
              <div class="py-3">
                <p v-if="snapshot.truncated" class="mb-3 text-xs text-muted-foreground">
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
                    @update:open="setOpen(file.path, $event)"
                  />
                </div>
              </div>
            </div>
          </div>
          <p v-else class="px-5 py-4 text-sm text-muted-foreground">The commits change no files.</p>
        </template>
      </div>
    </template>
  </div>
</template>
