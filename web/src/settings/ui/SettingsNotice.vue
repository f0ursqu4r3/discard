<script setup lang="ts">
// Protected surface. Keep data-protected and every protected class: protected.test.ts and
// the protected-surface rule depend on them. Pass `role` through as a normal attribute.
withDefaults(defineProps<{ as?: string }>(), { as: 'div' })
</script>
<template>
  <component
    :is="as"
    data-protected="true"
    class="rounded-md border border-[var(--protected-border)] bg-[var(--protected-bg)] p-3 text-[var(--protected-fg)]"
  >
    <slot />
  </component>
</template>
