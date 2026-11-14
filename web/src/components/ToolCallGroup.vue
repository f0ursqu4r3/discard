<script setup lang="ts">
import { ChevronRight, Clock3 } from '@lucide/vue'
import { computed, ref } from 'vue'
import type { PromptDto } from '../bridge/types'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible'
import BlockView from './BlockView.vue'
import BrailleSpinner from './BrailleSpinner.vue'
import { approvalStatus, toolCallLabel, toolCallSummary, type ToolCallItem } from './messages'

const props = defineProps<{
  channelId: string
  items: ToolCallItem[]
  running?: boolean
  prompts?: PromptDto[]
  messageId?: string
}>()
const expanded = ref(false)
const active = computed(() => {
  const last = props.items.at(-1)?.block
  return props.running && last?.output === null ? last : null
})
const awaiting = computed(
  () => !props.running && props.items.some(({ block }) => block.output === null),
)
const needApproval = computed(
  () =>
    props.items.filter(({ approval }) => approvalStatus(approval, props.prompts) === 'pending')
      .length,
)
const failures = computed(() => props.items.filter(({ block }) => block.is_error === true).length)
const succeeded = computed(() => props.items.every(({ block }) => block.is_error === false))
const detail = computed(() => {
  if (active.value)
    return [toolCallLabel(active.value.name), toolCallSummary(active.value.input)]
      .filter(Boolean)
      .join(' ')
  const counts = new Map<string, number>()
  for (const { block } of props.items) {
    const label = toolCallLabel(block.name)
    counts.set(label, (counts.get(label) ?? 0) + 1)
  }
  return [...counts].map(([label, count]) => (count > 1 ? `${label} ×${count}` : label)).join(', ')
})
</script>

<template>
  <Collapsible
    v-model:open="expanded"
    class="tool-call-group my-3 min-w-0 border-l-2 border-border rounded-none font-mono text-xs"
  >
    <CollapsibleTrigger
      class="group flex min-h-7 w-full min-w-0 items-center gap-2 rounded-none px-2 py-1 text-left text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      :aria-label="`${expanded ? 'Hide' : 'Show'} ${items.length} tool calls`"
    >
      <ChevronRight
        aria-hidden="true"
        class="size-3.5 shrink-0 transition-transform duration-150 group-data-[state=open]:rotate-90"
      />
      <span class="shrink-0 text-foreground/80">{{ items.length }} tool calls</span>
      <span class="min-w-0 flex-1 truncate" :title="detail">{{ detail }}</span>
      <span class="ml-auto flex shrink-0 items-center gap-1.5 pl-2 text-xs">
        <span
          v-if="needApproval"
          data-ui-id="approval-count"
          class="font-medium"
          :style="{ color: 'var(--protected-fg)' }"
        >
          {{ needApproval === 1 ? '1 needs approval' : `${needApproval} need approval` }}
        </span>
        <BrailleSpinner v-if="active" label="Running" />
        <Clock3 v-else-if="awaiting" role="img" aria-label="Awaiting result" class="size-3" />
        <span v-if="failures" :aria-label="`${failures} failed`" class="text-destructive">
          {{ failures }} failed
        </span>
        <span
          v-else-if="!active && !awaiting && succeeded"
          role="img"
          aria-label="All succeeded"
          class="text-success"
        >
          ✓
        </span>
      </span>
    </CollapsibleTrigger>
    <!-- The group border marks the whole run. Nested rows do not repeat it. -->
    <CollapsibleContent class="pl-3 [&_.tool-call]:my-0 [&_.tool-call]:border-l-0">
      <BlockView
        v-for="{ block, index, approval } in items"
        :key="index"
        :channel-id="channelId"
        :block="block"
        :approval-block="approval"
        :inline="index === 0"
        :running="running"
        :prompts="prompts"
        :message-id="messageId"
        :index="index"
      />
    </CollapsibleContent>
  </Collapsible>
</template>
