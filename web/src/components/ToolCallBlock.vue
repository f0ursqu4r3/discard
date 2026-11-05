<script setup lang="ts">
import { ref, computed } from 'vue'
import { Button } from './ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible'
import DiffBlock from './DiffBlock.vue'
import { looksLikeUnifiedDiff, summarizeInput } from './messages'
const props = defineProps<{ name: string; input: unknown; output: string | null }>()
const expanded = ref(false)
const lines = computed(() => props.output?.split('\n') ?? [])
const isDiff = computed(() => !!props.output && looksLikeUnifiedDiff(props.output))
</script>
<template>
  <div class="rounded-md border-l-2 border-primary p-2 text-sm">
    <p class="font-semibold">{{ name }} {{ summarizeInput(input) }}</p>
    <Collapsible v-if="lines.length > 20" v-model:open="expanded">
      <DiffBlock v-if="isDiff" inline :text="lines.slice(0, 20).join('\n')" />
      <pre v-else class="overflow-x-auto whitespace-pre-wrap text-xs text-muted-foreground">{{ lines.slice(0, 20).join('\n') }}</pre>
      <CollapsibleContent>
        <DiffBlock v-if="isDiff" inline :text="lines.slice(20).join('\n')" />
        <pre v-else class="overflow-x-auto whitespace-pre-wrap text-xs text-muted-foreground">{{ lines.slice(20).join('\n') }}</pre>
      </CollapsibleContent>
      <CollapsibleTrigger as-child><Button variant="ghost" size="sm">{{ expanded ? 'Show less' : 'Show more' }}</Button></CollapsibleTrigger>
    </Collapsible>
    <DiffBlock v-else-if="isDiff" inline :text="output!" />
    <pre v-else-if="output" class="overflow-x-auto whitespace-pre-wrap text-xs text-muted-foreground">{{ output }}</pre>
  </div>
</template>
