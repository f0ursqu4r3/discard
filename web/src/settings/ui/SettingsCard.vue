<script setup lang="ts">
// Bordered card with a mono header bar. The title renders as an h2. Use the `title` slot
// for a richer title, and `#actions` for controls at the right of the bar.
// `flush` removes the body padding. `rule: false` drops the header's bottom rule
// (use it when the body is hidden, so the card shows one border).
withDefaults(defineProps<{ title?: string; flush?: boolean; rule?: boolean }>(), {
  title: '',
  rule: true,
})
</script>
<template>
  <section class="overflow-hidden rounded-lg border border-border bg-card">
    <header
      class="flex items-center gap-2 bg-muted/50 px-3 py-1.5 font-mono text-xs"
      :class="rule ? 'border-b border-border' : ''"
    >
      <h2 class="min-w-0 font-medium">
        <slot name="title">{{ title }}</slot>
      </h2>
      <div v-if="$slots.actions" class="ml-auto flex min-w-0 items-center gap-3">
        <slot name="actions" />
      </div>
    </header>
    <div :class="flush ? '' : 'p-3'"><slot /></div>
  </section>
</template>
