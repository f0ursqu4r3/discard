<script setup lang="ts">
import { computed } from 'vue'
import { shortPath } from '../paths'
// A path that truncates at the START, so the tail stays visible. The untruncated short path is in `title`.
// The outer rtl box moves the ellipsis to the left. The inner ltr <bdi> keeps the path text in
// reading order, so a leading "/" or "~" does not jump to the other end. The text stays selectable.
const props = defineProps<{ path?: string }>()
const text = computed(() => shortPath(props.path ?? ''))
</script>
<template>
  <span dir="rtl" class="block min-w-0 truncate text-left" :title="text">
    <bdi dir="ltr">{{ text }}</bdi>
  </span>
</template>
