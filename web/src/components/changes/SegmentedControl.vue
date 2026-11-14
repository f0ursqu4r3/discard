<script setup lang="ts" generic="T extends string">
defineProps<{
  label: string
  options: readonly { value: T; label: string }[]
  modelValue: T
}>()
defineEmits<{ 'update:modelValue': [value: T] }>()
</script>

<template>
  <div
    role="group"
    :aria-label="label"
    class="inline-flex h-7 shrink-0 items-center rounded-md border border-border/60 p-0.5"
  >
    <button
      v-for="option in options"
      :key="option.value"
      type="button"
      :aria-pressed="modelValue === option.value"
      class="h-full rounded-sm px-2 text-xs text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 aria-pressed:bg-muted aria-pressed:text-foreground motion-reduce:transition-none"
      @click="$emit('update:modelValue', option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>
