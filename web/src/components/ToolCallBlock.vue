<script setup lang="ts">
import { ChevronRight, Clock3 } from '@lucide/vue'
import { ref, computed } from 'vue'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible'
import DiffBlock from './DiffBlock.vue'
import BrailleSpinner from './BrailleSpinner.vue'
import { looksLikeUnifiedDiff, summarizeInput } from './messages'

const props = withDefaults(
  defineProps<{
    name: string
    input: unknown
    output: string | null
    isError?: boolean
    running?: boolean
  }>(),
  { isError: undefined },
)
const expanded = ref(false)
const label = computed(() => {
  const action = props.name.split('__').filter(Boolean).at(-1) ?? props.name
  const words = action
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[_-]+/g, ' ')
    .trim()
  return words ? words.toLowerCase() : 'tool call'
})
const summary = computed(() => {
  if (props.input == null) return ''
  if (typeof props.input === 'object' && !Array.isArray(props.input)) {
    const fields = props.input as Record<string, unknown>
    if (!Object.keys(fields).length) return ''
    if (typeof fields.query === 'string') return summarizeInput({ pattern: fields.query })
  }
  return summarizeInput(props.input)
})
const fullInput = computed(() =>
  typeof props.input === 'string' ? props.input : (JSON.stringify(props.input, null, 2) ?? 'null'),
)
const isDiff = computed(() => !!props.output && looksLikeUnifiedDiff(props.output))
</script>

<template>
  <Collapsible
    v-model:open="expanded"
    class="tool-call my-1 min-w-0 border-l-2 border-agent rounded-none font-mono text-sm"
    style="background: color-mix(in srgb, var(--canvas) 55%, var(--background))"
  >
    <CollapsibleTrigger
      class="group flex min-h-9 w-full min-w-0 items-center gap-2 rounded-none px-2 py-1.5 text-left text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      :aria-label="`${expanded ? 'Hide' : 'Show'} details for ${label}`"
    >
      <ChevronRight
        aria-hidden="true"
        class="size-3.5 shrink-0 transition-transform duration-150 group-data-[state=open]:rotate-90"
      />
      <span aria-hidden="true" class="text-agent">$</span>
      <span class="min-w-0 truncate font-medium" :title="name">{{ label }}</span>
      <span v-if="summary" class="min-w-0 flex-1 truncate" :title="summary">{{ summary }}</span>
      <span class="ml-auto flex shrink-0 items-center gap-1.5 pl-2 text-xs">
        <BrailleSpinner v-if="output === null && running" label="Running" />
        <Clock3
          v-else-if="output === null"
          role="img"
          aria-label="Awaiting result"
          class="size-3"
        />
        <span v-else-if="isError === false" role="img" aria-label="Succeeded" class="text-success">
          ✓
        </span>
        <span v-else-if="isError === true" role="img" aria-label="Failed" class="text-destructive">
          ✗
        </span>
        <span v-else role="img" aria-label="Returned">↵</span>
      </span>
    </CollapsibleTrigger>
    <CollapsibleContent>
      <div
        class="mx-2 mb-2 mt-1 min-w-0 rounded-md border border-border/60 bg-background/40 p-3 font-mono text-muted-foreground"
      >
        <div class="mb-3 break-all font-mono text-xs text-muted-foreground">{{ name }}</div>
        <div class="mb-1 text-xs font-medium text-muted-foreground">Input</div>
        <pre
          class="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-sm font-mono text-xs leading-relaxed focus-visible:outline-2 focus-visible:outline-ring"
          tabindex="0"
          aria-label="Tool input"
          >{{ fullInput }}</pre>
        <div class="mb-1 mt-3 text-xs font-medium text-muted-foreground">Output</div>
        <p v-if="output === null" class="text-xs text-muted-foreground">No result received yet.</p>
        <p v-else-if="output === ''" class="text-xs text-muted-foreground">No output.</p>
        <div
          v-else
          class="max-h-80 overflow-auto rounded-sm focus-visible:outline-2 focus-visible:outline-ring"
          tabindex="0"
          aria-label="Tool output"
        >
          <DiffBlock v-if="isDiff" inline :text="output" />
          <pre v-else class="whitespace-pre-wrap break-all font-mono text-xs leading-relaxed">{{
            output
          }}</pre>
        </div>
      </div>
    </CollapsibleContent>
  </Collapsible>
</template>
