<script setup lang="ts">
import { Button } from './ui/button'
import type { ContextTargetDto } from '../bridge/types'
import { runAction } from '../bridge/api'
import { classifyLine } from './messages'
const props = defineProps<{ text: string; target?: ContextTargetDto | null; inline?: boolean }>()
async function copy() { if (!props.target) return; const text = await runAction(props.target.type === 'code_block' ? 'copy_code' : 'copy_diff', props.target); if (text !== null) await navigator.clipboard.writeText(text) }

</script>
<template>
  <div :class="inline ? 'text-xs' : 'rounded-md bg-muted p-2 text-sm'"><div v-if="!inline" class="flex items-center justify-between"><span>Diff</span><Button v-if="target" variant="ghost" size="sm" @click="copy">Copy</Button></div>
    <pre class="overflow-x-auto"><div v-for="(line, i) in text.split('\n')" :key="i" :data-diff="classifyLine(line)" :class="{ 'text-[color:var(--diff-added)]': !inline && classifyLine(line) === 'added', 'text-[color:var(--diff-removed)]': !inline && classifyLine(line) === 'removed', 'text-[color:var(--diff-added-inline)]': inline && classifyLine(line) === 'added', 'text-[color:var(--diff-removed-inline)]': inline && classifyLine(line) === 'removed', 'text-muted-foreground': classifyLine(line) === 'hunk' }">{{ line || ' ' }}</div></pre>
  </div>
</template>
