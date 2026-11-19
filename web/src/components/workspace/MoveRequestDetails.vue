<script setup lang="ts">
import { computed } from 'vue'
import type { MoveDetails } from '@/bridge/api'
const props = defineProps<{ details: MoveDetails }>()
const sections = computed(() => {
  const { constraints, decisions, openQuestions } = props.details.handoff
  return [
    { label: 'Constraints', items: constraints },
    { label: 'Decisions', items: decisions },
    { label: 'Open questions', items: openQuestions },
  ].filter((section) => section.items.length)
})
</script>
<template>
  <dl class="grid min-w-0 gap-1" data-ui-id="move-request-details">
    <dt class="font-medium">To</dt>
    <dd v-if="details.target.kind === 'channel'" class="break-all">#{{ details.target.name }}</dd>
    <template v-else>
      <dd class="break-all">
        New branch
        <code class="font-mono">{{ details.target.branch }}</code>
      </dd>
      <template v-if="details.target.repository">
        <dt class="mt-2 font-medium">Repository</dt>
        <dd class="break-all">
          <code class="font-mono">{{ details.target.repository }}</code>
        </dd>
      </template>
      <dt class="mt-2 font-medium">Base</dt>
      <dd class="break-all">
        <code class="font-mono">{{ details.target.base }}</code>
        at
        <code class="font-mono">{{ details.target.baseCommit.slice(0, 7) }}</code>
      </dd>
    </template>
    <template v-if="details.handoff.goal">
      <dt class="mt-2 font-medium">Goal</dt>
      <dd class="break-words whitespace-pre-wrap">{{ details.handoff.goal }}</dd>
    </template>
    <template v-if="details.attachments?.length">
      <dt class="mt-2 font-medium">Attachments</dt>
      <dd>
        <ul class="list-disc space-y-1 pl-5">
          <li v-for="file in details.attachments" :key="file.id" class="break-all">{{ file.name }}</li>
        </ul>
      </dd>
    </template>
    <template v-for="section in sections" :key="section.label">
      <dt class="mt-2 font-medium">{{ section.label }}</dt>
      <dd>
        <ul class="list-disc space-y-1 pl-5">
          <li
            v-for="(item, index) in section.items"
            :key="index"
            class="break-words whitespace-pre-wrap"
          >
            {{ item }}
          </li>
        </ul>
      </dd>
    </template>
  </dl>
</template>
