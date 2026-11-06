<script setup lang="ts">
import { Terminal } from '@lucide/vue'
import { ref, computed } from 'vue'
import { Button } from './ui/button'
import { CodeBlock, CodeBlockContent, CodeBlockHeader, CodeBlockTitle } from './ui/code-block'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible'
import DiffBlock from './DiffBlock.vue'
import { looksLikeUnifiedDiff, summarizeInput } from './messages'
const props = defineProps<{ name: string; input: unknown; output: string | null }>()
const expanded = ref(false)
const lines = computed(() => props.output?.split('\n') ?? [])
const isDiff = computed(() => !!props.output && looksLikeUnifiedDiff(props.output))
</script>
<template>
  <CodeBlock size="sm">
    <CodeBlockHeader>
      <CodeBlockTitle>
        <Terminal />
        {{ name }}
      </CodeBlockTitle>
      <span class="truncate font-mono font-normal text-muted-foreground">
        {{ summarizeInput(input) }}
      </span>
    </CodeBlockHeader>
    <Collapsible v-if="lines.length > 20" v-model:open="expanded">
      <DiffBlock v-if="isDiff" inline :text="lines.slice(0, 20).join('\n')" />
      <CodeBlockContent v-else wrap>{{ lines.slice(0, 20).join('\n') }}</CodeBlockContent>
      <CollapsibleContent>
        <DiffBlock v-if="isDiff" inline :text="lines.slice(20).join('\n')" />
        <CodeBlockContent v-else wrap>{{ lines.slice(20).join('\n') }}</CodeBlockContent>
      </CollapsibleContent>
      <CollapsibleTrigger as-child>
        <Button variant="ghost" size="sm">{{ expanded ? 'Show less' : 'Show more' }}</Button>
      </CollapsibleTrigger>
    </Collapsible>
    <DiffBlock v-else-if="isDiff" inline :text="output!" />
    <CodeBlockContent v-else-if="output" wrap>{{ output }}</CodeBlockContent>
  </CodeBlock>
</template>
