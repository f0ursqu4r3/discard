<script setup lang="ts">
import { ChevronDown, ChevronRight } from '@lucide/vue'
import { computed } from 'vue'
import type { ChangeFileDto } from '../../bridge/types'
import { CodeBlockCopyButton } from '../ui/code-block'
import { parseDiff } from '../messages'
import { inlineRows, splitRows } from './diffRows'
import { statusTone } from './tree'

const props = defineProps<{
  file: ChangeFileDto
  layout: 'split' | 'inline'
  /** The body shows only while the section is open. A closed section parses nothing. */
  open: boolean
  selected?: boolean
}>()
defineEmits<{ 'update:open': [open: boolean] }>()

const lines = computed(() =>
  props.open && !props.file.binary ? parseDiff(props.file.diff).lines : [],
)
const split = computed(() => (props.layout === 'split' ? splitRows(lines.value) : []))
const inline = computed(() => (props.layout === 'inline' ? inlineRows(lines.value) : []))
const isEmpty = computed(() => (props.layout === 'split' ? split.value : inline.value).length === 0)

type Kind = 'added' | 'removed' | 'context'
/** The same tints as the diffs in the conversation. An empty side of a split row gets a quiet tint. */
function tint(kind: Kind | null) {
  if (kind === null) return { background: 'color-mix(in srgb, var(--muted) 30%, transparent)' }
  if (kind === 'context') return undefined
  const color = `var(--diff-${kind})`
  return { background: `color-mix(in srgb, ${color} 11%, transparent)`, '--diff-sign': color }
}
const hunkTint = { background: 'color-mix(in srgb, var(--ansi-6) 6%, transparent)' }
const sign = (kind: Kind) => (kind === 'added' ? '+' : kind === 'removed' ? '-' : '')

const sides = ['left', 'right'] as const
const panes: Record<(typeof sides)[number], HTMLElement | null> = { left: null, right: null }
/** Keep the two split panes at the same sideways position. */
function follow(side: (typeof sides)[number]) {
  const from = panes[side]
  const to = panes[side === 'left' ? 'right' : 'left']
  if (from && to && to.scrollLeft !== from.scrollLeft) to.scrollLeft = from.scrollLeft
}
</script>

<template>
  <section
    data-ui-id="file-diff"
    :data-file-path="file.path"
    :data-selected="selected || undefined"
    class="min-w-0 rounded-md border border-border/60 bg-background"
  >
    <header
      class="sticky top-0 z-10 flex min-w-0 items-center gap-2 border-border/60 bg-background px-2 py-1 font-mono text-xs"
      :class="[open ? 'rounded-t-md border-b' : 'rounded-md', selected ? 'text-foreground' : '']"
    >
      <button
        type="button"
        data-file-toggle
        :aria-expanded="open"
        :aria-label="`${open ? 'Collapse' : 'Expand'} ${file.path}`"
        class="flex size-6 shrink-0 items-center justify-center rounded-sm text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
        @click="$emit('update:open', !open)"
      >
        <component :is="open ? ChevronDown : ChevronRight" aria-hidden="true" class="size-3.5" />
      </button>
      <span
        data-status
        class="w-3.5 shrink-0 text-center font-semibold"
        :class="statusTone(file.status)"
        :aria-label="`Status ${file.status}`"
      >
        {{ file.status }}
      </span>
      <span data-file-name class="min-w-0 flex-1 truncate" :title="file.path">
        <template v-if="file.oldPath">{{ file.oldPath }} →</template>
        {{ file.path }}
      </span>
      <span v-if="file.binary" class="shrink-0 text-muted-foreground">binary</span>
      <span v-else class="flex shrink-0 gap-1.5">
        <span class="text-success">+{{ file.additions }}</span>
        <span class="text-destructive">−{{ file.deletions }}</span>
      </span>
      <CodeBlockCopyButton :value="file.path" title="Copy path" class="shrink-0" />
    </header>
    <div v-if="open" data-file-body class="font-mono text-xs leading-5">
      <p v-if="file.binary" class="px-3 py-2 text-muted-foreground">Binary file</p>
      <template v-else>
        <!-- Split: one pane per side. No line wraps, so every row has the same height and the two sides stay
             aligned. Each pane scrolls sideways, and the other pane follows. -->
        <div v-if="layout === 'split'" data-layout="split" class="grid grid-cols-2">
          <div
            v-for="side in sides"
            :key="side"
            :ref="(element) => (panes[side] = element as HTMLElement | null)"
            :data-pane="side"
            class="min-w-0 overflow-x-auto"
            :class="side === 'right' ? 'border-l border-border/60' : ''"
            @scroll.passive="follow(side)"
          >
            <div class="w-max min-w-full">
              <template v-for="(row, i) in split" :key="i">
                <div
                  v-if="row.type === 'hunk'"
                  data-row="hunk"
                  class="h-5 px-3 whitespace-pre text-ansi-6"
                  :style="hunkTint"
                >
                  {{ side === 'left' ? row.text : '' }}
                </div>
                <div
                  v-else-if="row.type === 'note'"
                  data-row="note"
                  class="h-5 px-3 whitespace-pre text-muted-foreground"
                >
                  {{ side === 'left' ? row.text : '' }}
                </div>
                <div
                  v-else
                  data-row="line"
                  :data-side="side"
                  :data-diff="row[side]?.kind"
                  class="grid h-5 grid-cols-[5ch_1ch_minmax(0,1fr)]"
                  :style="tint(row[side]?.kind ?? null)"
                >
                  <span
                    aria-hidden="true"
                    class="select-none px-1 text-right text-muted-foreground"
                  >
                    {{ row[side]?.number }}
                  </span>
                  <span class="text-[color:var(--diff-sign)]">
                    {{ row[side] ? sign(row[side]!.kind) : '' }}
                  </span>
                  <span class="whitespace-pre pr-3">{{ row[side]?.text }}</span>
                </div>
              </template>
            </div>
          </div>
        </div>
        <!-- Inline: long lines scroll sideways inside the body. -->
        <div v-else data-layout="inline" class="overflow-x-auto">
          <div class="w-max min-w-full">
            <template v-for="(row, i) in inline" :key="i">
              <div
                v-if="row.type === 'hunk'"
                data-row="hunk"
                class="px-3 text-ansi-6 whitespace-pre"
                :style="hunkTint"
              >
                {{ row.text }}
              </div>
              <div
                v-else-if="row.type === 'note'"
                data-row="note"
                class="px-3 whitespace-pre text-muted-foreground"
              >
                {{ row.text }}
              </div>
              <div
                v-else
                data-row="line"
                :data-diff="row.kind"
                class="grid grid-cols-[5ch_5ch_1ch_minmax(0,1fr)]"
                :style="tint(row.kind)"
              >
                <span aria-hidden="true" class="select-none px-1 text-right text-muted-foreground">
                  {{ row.oldNumber }}
                </span>
                <span aria-hidden="true" class="select-none px-1 text-right text-muted-foreground">
                  {{ row.newNumber }}
                </span>
                <span class="text-[color:var(--diff-sign)]">{{ sign(row.kind) }}</span>
                <span class="whitespace-pre pr-3">{{ row.text }}</span>
              </div>
            </template>
          </div>
        </div>
        <p v-if="isEmpty && !file.truncated" class="px-3 py-2 text-muted-foreground">
          No line changes to show.
        </p>
        <p v-if="file.truncated" class="px-3 py-2 text-muted-foreground">
          Some changes are not shown.
        </p>
      </template>
    </div>
  </section>
</template>
