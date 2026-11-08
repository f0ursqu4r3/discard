<script setup lang="ts">
import { computed } from 'vue'
import {
  CodeBlock,
  CodeBlockActions,
  CodeBlockContent,
  CodeBlockCopyButton,
  CodeBlockHeader,
  CodeBlockLine,
  CodeBlockTitle,
} from './ui/code-block'
import type { ContextTargetDto } from '../bridge/types'
import { parseDiff } from './messages'
const props = defineProps<{ text: string; target?: ContextTargetDto | null; inline?: boolean }>()
const diff = computed(() => parseDiff(props.text))
function lineStyle(kind: string) {
  if (kind === 'hunk') return { background: 'color-mix(in srgb, var(--ansi-6) 6%, transparent)' }
  if (kind !== 'added' && kind !== 'removed') return undefined
  const color = `var(--diff-${kind}${props.inline ? '-inline' : ''})`
  return { background: `color-mix(in srgb, ${color} 11%, transparent)`, '--diff-sign': color }
}
</script>
<template>
  <CodeBlock :size="inline ? 'sm' : 'default'">
    <CodeBlockHeader v-if="!inline">
      <CodeBlockTitle class="normal-case">{{ diff.path ?? 'diff' }}</CodeBlockTitle>
      <span class="text-xs text-success">+{{ diff.added }}</span>
      <span class="text-xs text-destructive">−{{ diff.removed }}</span>
      <CodeBlockActions><CodeBlockCopyButton :value="text" /></CodeBlockActions>
    </CodeBlockHeader>
    <CodeBlockContent>
      <CodeBlockLine
        v-for="(line, i) in diff.lines"
        :key="i"
        :data-diff="line.kind"
        :style="lineStyle(line.kind)"
        :class="{ 'text-ansi-6': line.kind === 'hunk' }"
      >
        <span
          class="grid"
          :class="inline ? 'grid-cols-[1ch_minmax(0,1fr)]' : 'grid-cols-[3ch_1ch_minmax(0,1fr)]'"
        >
          <span
            v-if="!inline"
            data-testid="diff-number"
            aria-hidden="true"
            class="select-none pr-2 text-right text-muted-foreground"
          >
            {{ line.number }}
          </span>
          <span class="text-[color:var(--diff-sign)]">
            {{ line.kind === 'added' ? '+' : line.kind === 'removed' ? '-' : '' }}
          </span>
          <span>
            {{
              ['added', 'removed'].includes(line.kind) || line.text.startsWith(' ')
                ? line.text.slice(1)
                : line.text
            }}
          </span>
        </span>
      </CodeBlockLine>
    </CodeBlockContent>
  </CodeBlock>
</template>
