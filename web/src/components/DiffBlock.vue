<script setup lang="ts">
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
import { classifyLine } from './messages'
defineProps<{ text: string; target?: ContextTargetDto | null; inline?: boolean }>()
</script>
<template>
  <CodeBlock :size="inline ? 'sm' : 'default'">
    <CodeBlockHeader v-if="!inline">
      <CodeBlockTitle>Diff</CodeBlockTitle>
      <CodeBlockActions v-if="target">
        <CodeBlockCopyButton :value="text" />
      </CodeBlockActions>
    </CodeBlockHeader>
    <CodeBlockContent>
      <CodeBlockLine
        v-for="(line, i) in text.split('\n')"
        :key="i"
        :data-diff="classifyLine(line)"
        :class="{
          'text-[color:var(--diff-added)]': !inline && classifyLine(line) === 'added',
          'text-[color:var(--diff-removed)]': !inline && classifyLine(line) === 'removed',
          'text-[color:var(--diff-added-inline)]': inline && classifyLine(line) === 'added',
          'text-[color:var(--diff-removed-inline)]': inline && classifyLine(line) === 'removed',
          'text-muted-foreground': classifyLine(line) === 'hunk',
        }"
      >
        {{ line }}
      </CodeBlockLine>
    </CodeBlockContent>
  </CodeBlock>
</template>
