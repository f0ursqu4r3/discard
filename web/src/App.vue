<script setup lang="ts">
import { computed } from 'vue'
import { useStore } from './bridge/store'
const store = useStore()
const live = computed(() => store.snapshot?.liveBlocks.filter(block => 'text' in block).map(block => 'text' in block ? block.text : '').join('') ?? '')
</script>
<template>
  <main data-testid="app-root" class="flex min-h-screen flex-col gap-4 p-6">
    <h1 class="text-[length:1.5em] font-semibold">{{ store.snapshot?.project }}</h1>
    <h2 class="text-[length:1.125em] text-muted-foreground">{{ store.snapshot?.channel }}</h2>
    <p class="whitespace-pre-wrap">{{ live }}</p>
  </main>
</template>
