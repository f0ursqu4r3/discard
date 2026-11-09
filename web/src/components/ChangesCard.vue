<script setup lang="ts">
import type { BlockDto } from '../bridge/types'
import { Badge } from './ui/badge'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible'
import DiffBlock from './DiffBlock.vue'
defineProps<{ block: Extract<BlockDto, { type: 'changes' }> }>()
</script>
<template>
  <div
    data-ui-id="changes-card"
    data-protected="true"
    class="space-y-2 rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-3 text-[var(--protected-fg)]"
  >
    <div class="flex items-center gap-2">
      <p class="font-semibold">
        Changes · {{ block.files.length }} {{ block.files.length === 1 ? 'file' : 'files' }} ·
        {{ block.commits.length }} {{ block.commits.length === 1 ? 'commit' : 'commits' }}
      </p>
      <Badge v-if="block.state !== 'completed'" variant="outline">
        {{ block.state === 'stopped' ? 'Stopped' : 'Failed' }}
      </Badge>
    </div>
    <ul v-if="block.commits.length" class="space-y-1 text-sm">
      <li v-for="commit in block.commits" :key="commit.id">
        <code>{{ commit.id.slice(0, 7) }}</code>
        {{ commit.subject }}
      </li>
    </ul>
    <Collapsible v-for="file in block.files" :key="file.path" class="space-y-1">
      <CollapsibleTrigger
        class="w-full text-left text-sm focus-visible:outline focus-visible:outline-[var(--protected-border)]"
      >
        {{ file.status }} {{ file.path }}
      </CollapsibleTrigger>
      <p v-if="file.preexisting" class="text-sm">Had uncommitted changes before this response.</p>
      <CollapsibleContent>
        <DiffBlock :text="file.diff" />
        <p v-if="file.truncated" class="text-sm">Some changes are not shown.</p>
      </CollapsibleContent>
    </Collapsible>
    <p v-if="block.truncated" class="text-sm">Some changes are not shown.</p>
  </div>
</template>
