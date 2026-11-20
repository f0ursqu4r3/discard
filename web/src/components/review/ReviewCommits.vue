<script setup lang="ts">
import type { ReviewCommitDto } from '../../bridge/types'

defineProps<{ commits: ReviewCommitDto[] }>()

const date = (seconds: number) => new Date(seconds * 1000).toLocaleDateString()
</script>

<template>
  <section data-ui-id="review-commits" class="px-3 py-3 sm:px-5">
    <h3 class="mb-2 text-xs font-medium text-muted-foreground">
      {{ commits.length }} {{ commits.length === 1 ? 'commit' : 'commits' }}
    </h3>
    <ul class="flex max-h-48 flex-col gap-1 overflow-auto text-sm">
      <li v-for="commit in commits" :key="commit.id" class="flex items-baseline gap-2">
        <span class="shrink-0 font-mono text-xs text-muted-foreground">{{ commit.id }}</span>
        <span class="min-w-0 flex-1 truncate">{{ commit.subject }}</span>
        <span class="hidden shrink-0 text-xs text-muted-foreground sm:inline">
          {{ commit.author }} · {{ date(commit.time) }}
        </span>
      </li>
    </ul>
  </section>
</template>
