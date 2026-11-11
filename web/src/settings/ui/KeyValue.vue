<script setup lang="ts">
// Mono key-value row with a dotted leader. Place rows inside a <dl>.
// The sr-only separator keeps "key: value" in the text for assistive tech.
defineProps<{ label: string; title?: string }>()
</script>
<template>
  <div class="flex items-baseline font-mono text-xs">
    <!-- prettier-ignore -->
    <dt class="shrink-0 text-muted-foreground">{{ label }}<span class="sr-only">: </span></dt>
    <span aria-hidden="true" class="kv-leader" />
    <dd class="min-w-0 truncate text-right" :title="title"><slot /></dd>
  </div>
</template>
