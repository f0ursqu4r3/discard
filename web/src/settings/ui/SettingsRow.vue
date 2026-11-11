<script setup lang="ts">
// `labelFor` wires the label to a control id. `helpId` sets the id of the help text,
// so a control can point at it with aria-describedby.
defineProps<{ label: string; help?: string; labelFor?: string; helpId?: string }>()
</script>
<template>
  <div
    class="flex items-center justify-between gap-4 border-b border-dashed border-border py-3 first:pt-1 last:border-b-0"
  >
    <div class="min-w-0">
      <label v-if="labelFor" :for="labelFor" class="font-medium">{{ label }}</label>
      <span v-else class="font-medium">{{ label }}</span>
      <p v-if="help" :id="helpId" class="text-sm text-muted-foreground">{{ help }}</p>
    </div>
    <div class="flex shrink-0 items-center gap-2"><slot /></div>
  </div>
</template>
