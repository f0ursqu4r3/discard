<script setup lang="ts">
import { ChevronRight, Clock3 } from '@lucide/vue'
import { ref, computed } from 'vue'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible'
import DiffBlock from './DiffBlock.vue'
import BrailleSpinner from './BrailleSpinner.vue'
import ApprovalMark from './ApprovalMark.vue'
import {
  looksLikeUnifiedDiff,
  toolCallLabel,
  toolCallSummary,
  type ApprovalStatus,
} from './messages'

const props = withDefaults(
  defineProps<{
    name: string
    input: unknown
    output: string | null
    isError?: boolean
    running?: boolean
    sandboxed?: boolean
    /** The approval status of this call. Absent when the call needed none. */
    approval?: ApprovalStatus
  }>(),
  { isError: undefined },
)
const expanded = ref(false)
const label = computed(() => toolCallLabel(props.name))
const summary = computed(() => toolCallSummary(props.input))
const fullInput = computed(() =>
  typeof props.input === 'string' ? props.input : (JSON.stringify(props.input, null, 2) ?? 'null'),
)
const isDiff = computed(() => !!props.output && looksLikeUnifiedDiff(props.output))
</script>

<template>
  <Collapsible
    v-model:open="expanded"
    class="tool-call my-3 min-w-0 border-l-2 border-border rounded-none font-mono text-xs"
  >
    <CollapsibleTrigger
      class="group flex min-h-7 w-full min-w-0 items-center gap-2 rounded-none px-2 py-1 text-left text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      :aria-label="`${expanded ? 'Hide' : 'Show'} details for ${label}`"
    >
      <ChevronRight
        aria-hidden="true"
        class="size-3.5 shrink-0 transition-transform duration-150 group-data-[state=open]:rotate-90"
      />
      <span aria-hidden="true" class="opacity-60">$</span>
      <span class="min-w-0 truncate text-foreground/80" :title="name">{{ label }}</span>
      <span v-if="summary" class="min-w-0 flex-1 truncate" :title="summary">{{ summary }}</span>
      <span class="ml-auto flex shrink-0 items-center gap-1.5 pl-2 text-xs">
        <span v-if="sandboxed" class="text-muted-foreground">Ran in sandbox</span>
        <ApprovalMark :status="approval" />
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
